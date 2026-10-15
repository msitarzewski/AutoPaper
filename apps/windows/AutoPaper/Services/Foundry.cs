using System.Diagnostics;
using System.Text.Json;

namespace AutoPaper.Services;

/// <summary>
/// Foundry Local, Microsoft's on-device model runner. It serves an OpenAI-compatible API on a port it picks itself, so
/// the address comes from its own command line (<c>foundry server status</c>), not from the person. Experimental: only
/// ever asked for by a click on Settings › Providers › Find Foundry Local, and only a loopback address is accepted.
/// </summary>
internal static class Foundry
{
    private static readonly TimeSpan StartLimit = TimeSpan.FromSeconds(90);
    private static readonly TimeSpan StatusLimit = TimeSpan.FromSeconds(20);

    /// <summary>Starts Foundry Local's service if it isn't running and returns its address (for example
    /// "http://127.0.0.1:65151"), or null when the <c>foundry</c> command isn't installed or gave no usable address.</summary>
    public static async Task<string?> FindAddressAsync()
    {
        if (await RunAsync("server start", StartLimit) is null)
        {
            return null;
        }
        var status = await RunAsync("server status -o json", StatusLimit);
        return status is null ? null : AddressIn(status);
    }

    /// <summary>The first loopback http address in <c>foundry server status -o json</c>'s "webUrls", else null.</summary>
    internal static string? AddressIn(string json)
    {
        try
        {
            using var document = JsonDocument.Parse(json);
            if (!document.RootElement.TryGetProperty("webUrls", out var urls) || urls.ValueKind != JsonValueKind.Array)
            {
                return null;
            }
            foreach (var url in urls.EnumerateArray())
            {
                if (url.ValueKind == JsonValueKind.String
                    && Uri.TryCreate(url.GetString(), UriKind.Absolute, out var uri)
                    && uri.Scheme == Uri.UriSchemeHttp
                    && uri.IsLoopback
                    && uri.UserInfo.Length == 0)
                {
                    return $"{uri.Scheme}://{uri.Host}:{uri.Port}";
                }
            }
        }
        catch (JsonException)
        {
        }
        return null;
    }

    /// <summary>Runs <c>foundry</c> with fixed arguments; its standard output, or null when it isn't installed, failed or
    /// took longer than <paramref name="limit"/>.</summary>
    private static async Task<string?> RunAsync(string arguments, TimeSpan limit)
    {
        using var cancel = new CancellationTokenSource(limit);
        try
        {
            using var process = Process.Start(new ProcessStartInfo("foundry", arguments)
            {
                CreateNoWindow = true,
                UseShellExecute = false,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
            });
            if (process is null)
            {
                return null;
            }
            var output = process.StandardOutput.ReadToEndAsync(cancel.Token);
            _ = process.StandardError.ReadToEndAsync(cancel.Token);
            await process.WaitForExitAsync(cancel.Token);
            return process.ExitCode == 0 ? await output : null;
        }
        catch (Exception error) when (error is System.ComponentModel.Win32Exception or OperationCanceledException or InvalidOperationException)
        {
            Log.Note($"Foundry Local: {error.GetType().Name}");
            return null;
        }
    }
}
