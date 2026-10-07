using System.Text.Json;
using System.Text.Json.Nodes;
using AutoPaper.Core;

namespace AutoPaper.Models;

/// <summary>
/// A person's own ComfyUI workflow file, read the way the core reads it when it fills one in
/// (<c>fill_placeholders</c>, <c>api_graph</c> and <c>loader</c> in core/src/providers/comfyui.rs): a file that can't
/// work is refused when it's chosen rather than at the next wallpaper, and Settings › Providers can say which model
/// it loads (the read-only Model line). No WinUI types (AutoPaper.Tests links it).
/// </summary>
internal static class OwnWorkflow
{
    /// <summary>The inputs a loader names its model file with, in the core's order (<c>MODEL_INPUTS</c>).</summary>
    private static readonly string[] ModelInputs = ["ckpt_name", "unet_name"];

    /// <summary>Usable (<see cref="Reading.Problem"/> null) with the model file its first loader loads, if any; or
    /// why it can't be used (WorkflowNeedsPrompt, WorkflowNotApiFormat, WorkflowInvalid).</summary>
    public readonly record struct Reading(string? Model, InvalidInputReason? Problem);

    public static Reading Read(string text)
    {
        if (!text.Contains("{{prompt}}", StringComparison.Ordinal))
        {
            return new Reading(null, InvalidInputReason.WorkflowNeedsPrompt);
        }
        var filled = text;
        foreach (var name in new[] { "width", "height", "seed" })
        {
            filled = filled.Replace($"\"{{{{{name}}}}}\"", "1", StringComparison.Ordinal).Replace($"{{{{{name}}}}}", "1", StringComparison.Ordinal);
        }
        filled = filled.Replace("\"{{prompt}}\"", "\"\"", StringComparison.Ordinal).Replace("{{prompt}}", "", StringComparison.Ordinal);
        JsonNode? value;
        try
        {
            value = JsonNode.Parse(filled);
        }
        catch (JsonException)
        {
            return new Reading(null, InvalidInputReason.WorkflowInvalid);
        }
        if (value is not JsonObject root || root["nodes"] is JsonArray)
        {
            return new Reading(null, InvalidInputReason.WorkflowNotApiFormat);
        }
        if (root["prompt"] is JsonObject inner && !root.All(entry => IsNode(entry.Value)))
        {
            root = inner;
        }
        if (root.Count == 0 || !root.All(entry => IsNode(entry.Value)))
        {
            return new Reading(null, InvalidInputReason.WorkflowNotApiFormat);
        }
        // Node ids in number order (then text), as the core looks for the loader.
        var ids = root.Select(entry => entry.Key)
            .OrderBy(id => ulong.TryParse(id, out var number) ? number : ulong.MaxValue)
            .ThenBy(id => id, StringComparer.Ordinal)
            .ToList();
        foreach (var input in ModelInputs)
        {
            foreach (var id in ids)
            {
                if (root[id]?["inputs"]?[input] is JsonValue model && model.TryGetValue<string>(out var file))
                {
                    return new Reading(file, null);
                }
            }
        }
        return new Reading(null, null);
    }

    private static bool IsNode(JsonNode? value) =>
        value is JsonObject node
        && node["class_type"] is JsonValue type && type.TryGetValue<string>(out _)
        && (!node.ContainsKey("inputs") || node["inputs"] is JsonObject);

    /// <summary>"krea2_turbo_fp8_scaled.safetensors" → "krea2_turbo_fp8_scaled" (the core's <c>display_name</c>);
    /// folders are kept.</summary>
    public static string PlainName(string file)
    {
        foreach (var extension in new[] { ".safetensors", ".ckpt", ".gguf", ".pt", ".pth", ".bin" })
        {
            if (file.EndsWith(extension, StringComparison.Ordinal))
            {
                return file[..^extension.Length];
            }
        }
        return file;
    }
}

/// <summary>
/// AutoPaper's own ComfyUI workflows: the very files the core is built with (core/resources/comfyui, embedded in this
/// assembly as "ComfyUI.&lt;file&gt;"), read for each one's name ("Z-Image Turbo", from its .map.json) and the model file
/// its loader loads (read as the core reads it: <see cref="OwnWorkflow.Read"/>). Settings › Providers names a model
/// in words with it while ComfyUI can't be asked for its list ("Default (Z-Image Turbo)"). No WinUI types
/// (AutoPaper.Tests links it, with the same files).
/// </summary>
internal static class BundledWorkflows
{
    private const string Prefix = "ComfyUI.";
    private static readonly Lazy<IReadOnlyDictionary<string, string>> Names = new(Load);

    /// <summary>The name of AutoPaper's workflow that loads <paramref name="model"/>, or null when it has none.</summary>
    public static string? NameOf(string model) => Names.Value.GetValueOrDefault(model);

    /// <summary>Every bundled model file and its workflow's name.</summary>
    public static IReadOnlyDictionary<string, string> All => Names.Value;

    private static IReadOnlyDictionary<string, string> Load()
    {
        var assembly = typeof(BundledWorkflows).Assembly;
        var names = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach (var resource in assembly.GetManifestResourceNames())
        {
            if (!resource.StartsWith(Prefix, StringComparison.Ordinal) || !resource.EndsWith(".map.json", StringComparison.Ordinal))
            {
                continue;
            }
            try
            {
                using var mapStream = assembly.GetManifestResourceStream(resource);
                if (mapStream is null || JsonNode.Parse(mapStream) is not JsonObject map
                    || map["name"]?.GetValue<string>() is not { Length: > 0 } name
                    || map["template"]?.GetValue<string>() is not { Length: > 0 } template)
                {
                    continue;
                }
                using var templateStream = assembly.GetManifestResourceStream(Prefix + template);
                if (templateStream is null)
                {
                    continue;
                }
                using var reader = new StreamReader(templateStream);
                if (OwnWorkflow.Read(reader.ReadToEnd()).Model is { } model)
                {
                    names[model] = name;
                }
            }
            catch (Exception error) when (error is JsonException or InvalidOperationException or IOException)
            {
                // A file that can't be read leaves its model named by its file (it's the core's resource: tested).
            }
        }
        return names;
    }
}
