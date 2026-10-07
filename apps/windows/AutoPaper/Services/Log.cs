using System.Diagnostics;
using Windows.Storage;

namespace AutoPaper.Services;

/// <summary>
/// Unexpected failures (and a few notes on the person's own wallpaper and lock screen), written to LocalState\logs\autopaper.log for support (local only, never sent; no keys or
/// prompts: only exception types, messages and stack traces). Kept under 256 KB.
/// </summary>
internal static class Log
{
    private const long MaxBytes = 256 * 1024;
    private static readonly Lock Gate = new();

    public static void Error(string context, Exception error) => Write($"{context}: {error}");

    /// <summary>A line about what AutoPaper did with the person's own wallpaper or lock screen (kinds and sizes only),
    /// for support when one doesn't come back as expected.</summary>
    public static void Note(string line) => Write(line);

    private static void Write(string line)
    {
        Debug.WriteLine(line);
        try
        {
            var folder = Path.Combine(ApplicationData.Current.LocalFolder.Path, "logs");
            Directory.CreateDirectory(folder);
            var file = Path.Combine(folder, "autopaper.log");
            lock (Gate)
            {
                if (File.Exists(file) && new FileInfo(file).Length > MaxBytes)
                {
                    File.Move(file, file + ".1", overwrite: true);
                }
                File.AppendAllText(file, $"{DateTimeOffset.Now:O} {line}{Environment.NewLine}");
            }
        }
        catch (Exception)
        {
            // Logging must never take the app down.
        }
    }
}
