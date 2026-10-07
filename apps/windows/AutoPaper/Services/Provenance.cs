using AutoPaper.Core;

namespace AutoPaper.Services;

/// <summary>
/// Which models made a wallpaper (user, 2026-10-06; the macOS app's Provenance): "Written by Gemini 3.5 Flash-Lite
/// (Google Gemini) · Painted by Nano Banana 2 (Google Gemini) · 3840×2160 · about $0.04". From the generation's own
/// record (the models the engine actually used, defaults resolved), so it shows whether a model chosen in Settings
/// was applied. No WinUI types: AutoPaper.Tests links it.
/// </summary>
internal static class Provenance
{
    /// <summary>Names for model ids, for when the provider's own list (list_models' display names) hasn't been read
    /// this session: Google's names from its model list (core/tests/fixtures/google). ComfyUI's come from AutoPaper's
    /// own workflows (BundledWorkflows); OpenAI's list names its models by id, so they stay ids.</summary>
    private static readonly Dictionary<string, string> KnownNames = new(StringComparer.Ordinal)
    {
        ["gemini-3.1-flash-image"] = "Nano Banana 2",
        ["gemini-3.1-flash-lite-image"] = "Nano Banana 2 Lite",
        ["gemini-3-pro-image"] = "Nano Banana Pro",
        ["gemini-2.5-flash-image"] = "Nano Banana",
        ["gemini-3.5-flash-lite"] = "Gemini 3.5 Flash-Lite",
        ["gemini-3.1-flash-lite"] = "Gemini 3.1 Flash-Lite",
        ["gemini-3.8-flash"] = "Gemini 3.8 Flash",
    };

    /// <summary>A provider's name on its own ("Google Gemini", "OpenAI-compatible", "Demo").</summary>
    public static string ProviderName(ProviderKind kind) => kind switch
    {
        ProviderKind.OpenAiCompatible => Loc.Get("Provider_Compatible"),
        ProviderKind.Demo => Loc.Get("Provider_Demo"),
        _ => Text.Provider(kind),
    };

    /// <summary>A model in words: the provider's own name (<paramref name="listed"/>, by id), else AutoPaper's, else the
    /// id. Demo's is "Demo"; an empty id (a record from before models were kept) is the provider's name.</summary>
    public static string ModelName(string id, ProviderKind kind, IReadOnlyDictionary<string, string>? listed = null)
    {
        var model = id.Trim();
        if (kind == ProviderKind.Demo || model.Length == 0)
        {
            return ProviderName(kind);
        }
        if (listed is not null && listed.TryGetValue(model, out var named) && !string.IsNullOrWhiteSpace(named))
        {
            return named.Trim();
        }
        if (kind == ProviderKind.ComfyUi && AutoPaper.Models.BundledWorkflows.NameOf(model) is { Length: > 0 } workflow)
        {
            return workflow;
        }
        return KnownNames.GetValueOrDefault(model) ?? model;
    }

    /// <summary>The model and its provider: "Nano Banana 2 (Google Gemini)", "Z-Image Turbo (ComfyUI)", or "Demo" alone.</summary>
    public static string Who(ProviderKind kind, string model, IReadOnlyDictionary<string, string>? listed = null)
    {
        var name = ModelName(model, kind, listed);
        var provider = ProviderName(kind);
        return name == provider ? name : Loc.Format("Provenance_Who", name, provider);
    }

    /// <summary>The line's parts.</summary>
    /// <param name="Written">"Written by Gemini 3.5 Flash-Lite (Google Gemini)".</param>
    /// <param name="Painted">"Painted by Nano Banana 2 (Google Gemini)": a link to Settings › Providers where it's shown.</param>
    /// <param name="Size">"3840×2160"; null when the size wasn't kept.</param>
    /// <param name="SizeSpoken">"3840 by 2160"; null when the size wasn't kept.</param>
    /// <param name="Cost">"about $0.04"; null when it cost nothing (local providers, Demo).</param>
    internal sealed record Line(string Written, string Painted, string? Size, string? SizeSpoken, string? Cost)
    {
        private const string Separator = " · ";

        /// <summary>The line as shown.</summary>
        public string Shown => string.Join(Separator, new[] { Written, Painted, Size, Cost }.OfType<string>());

        /// <summary>The line as a screen reader says it: commas, and "3840 by 2160".</summary>
        public string Spoken => string.Join(", ", new[] { Written, Painted, SizeSpoken, Cost }.OfType<string>());

        /// <summary>Before the painter, with its separator ("Written by … · "), for showing the painter as a link.</summary>
        public string Head => Written + Separator;

        /// <summary>After the painter, with its separator (" · 3840×2160 · about $0.04"); empty when there's nothing.</summary>
        public string Tail => string.Concat(new[] { Size, Cost }.OfType<string>().Select(part => Separator + part));
    }

    public static Line Of(Generation generation, IReadOnlyDictionary<string, string>? listed = null)
    {
        var sized = generation.Width > 0 && generation.Height > 0;
        ProviderKind[] hosted = [ProviderKind.OpenAi, ProviderKind.Google, ProviderKind.OpenAiCompatible];
        var paid = generation.CostMicrousd > 0 && (hosted.Contains(generation.TextProvider) || hosted.Contains(generation.ImageProvider));
        return new Line(
            Loc.Format("Provenance_Written", Who(generation.TextProvider, generation.TextModel, listed)),
            Loc.Format("Provenance_Painted", Who(generation.ImageProvider, generation.ImageModel, listed)),
            sized ? $"{generation.Width}×{generation.Height}" : null,
            sized ? Loc.Format("Provenance_SizeSpoken", generation.Width, generation.Height) : null,
            paid ? Loc.Format("Provenance_Cost", Text.Money(generation.CostMicrousd)) : null);
    }
}
