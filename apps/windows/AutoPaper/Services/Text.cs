using System.Globalization;
using AutoPaper.Core;
using Windows.Globalization.DateTimeFormatting;

namespace AutoPaper.Services;

/// <summary>The engine's values in the app's words (spec: "Errors", "Status lines", Surprise bands). No WinUI types:
/// AutoPaper.Tests links this file and checks the wording against Resources.resw.</summary>
internal static class Text
{
    private static readonly CultureInfo Dollars = CultureInfo.GetCultureInfo("en-US");

    /// <summary>A provider's name in a sentence ("OpenAI didn't accept your key").</summary>
    public static string Provider(ProviderKind kind) => kind switch
    {
        ProviderKind.OpenAi => "OpenAI",
        ProviderKind.Google => "Google Gemini",
        ProviderKind.Ollama => "Ollama",
        ProviderKind.OpenAiCompatible => Loc.Get("Provider_CompatibleInSentence"),
        ProviderKind.ComfyUi => "ComfyUI",
        ProviderKind.System => "The on-device model",
        _ => Loc.Get("Provider_Demo"),
    };

    /// <summary>A provider's name in a picker.</summary>
    public static string ProviderOption(ProviderKind kind) => kind switch
    {
        ProviderKind.OpenAi => "OpenAI",
        ProviderKind.Google => "Google Gemini",
        ProviderKind.Ollama => "Ollama",
        ProviderKind.OpenAiCompatible => Loc.Get("Provider_Compatible"),
        ProviderKind.ComfyUi => "ComfyUI",
        ProviderKind.System => "On-device model",
        _ => Loc.Get("Provider_DemoOption"),
    };

    public static string Stage(ProgressStage stage) => stage switch
    {
        ProgressStage.CheckingServices => Loc.Get("Stage_CheckingServices"),
        ProgressStage.Composing => Loc.Get("Stage_Composing"),
        ProgressStage.CheckingMemory => Loc.Get("Stage_CheckingMemory"),
        ProgressStage.Generating => Loc.Get("Stage_Painting"),
        ProgressStage.Downloading => Loc.Get("Stage_Downloading"),
        ProgressStage.Rendering => Loc.Get("Stage_Preparing"),
        _ => "",
    };

    // ── Problems ────────────────────────────────────────────────────────────────────────────────

    /// <summary>Why something didn't work, in a plain sentence, from the error's variant and typed reason only (the
    /// core's <c>detail</c> is English for logs; it's never shown or matched). <paramref name="scheduled"/>: it
    /// happened on the schedule (the sentence then says what AutoPaper does next); otherwise the person asked just
    /// now. Where the problem can be shown with a link to its fix (the Now view, History, a notification, Settings),
    /// use <see cref="Problem"/> instead (docs/app-spec.md 6a).</summary>
    public static string Error(Exception error, Settings? settings, bool scheduled = false)
    {
        if (error is not AutoPaperException problem)
        {
            return Loc.Get("Error_Unexpected");
        }
        return problem switch
        {
            AutoPaperException.MissingKey e => e.provider == ProviderKind.OpenAiCompatible
                ? Loc.Get("Error_MissingKeyCompatible")
                : Loc.Format("Error_MissingKey", Provider(e.provider)),
            AutoPaperException.InvalidKey e => Loc.Format("Error_InvalidKey", Provider(e.provider)),
            AutoPaperException.ProviderUnavailable e => Unavailable(e.provider, e.reason, settings, scheduled),
            AutoPaperException.RateLimited e => e.retryAfterSecs <= 60
                ? Loc.Format("Error_RateLimitedOne", Provider(e.provider))
                : Loc.Format("Error_RateLimited", Provider(e.provider), (e.retryAfterSecs + 59) / 60),
            AutoPaperException.Refused e => Loc.Format("Error_Refused", Provider(e.provider)),
            // Worded from what the provider can do, not the core's English `job`.
            AutoPaperException.Unsupported e => Loc.Format(
                Paints(e.provider) && !Writes(e.provider) ? "Error_UnsupportedConcepts" : "Error_UnsupportedImages",
                Provider(e.provider)),
            AutoPaperException.BudgetReached => Loc.Get("Error_BudgetReachedKeep"),
            AutoPaperException.Offline => Loc.Get(scheduled ? "Error_OfflineScheduled" : "Error_Offline"),
            AutoPaperException.InvalidResponse => Loc.Get(scheduled ? "Error_InvalidResponseScheduled" : "Error_InvalidResponse"),
            AutoPaperException.PaintingFailed e => PaintingFailed(e.provider, e.model),
            AutoPaperException.KeywordNotFollowed e => KeywordNotFollowed(e.keyword, e.weight),
            AutoPaperException.InvalidInput e => InvalidInput(e.reason),
            AutoPaperException.NotFound => Loc.Get("Error_NotFound"),
            AutoPaperException.NothingToRevisit => Loc.Get("Error_NothingToRevisit"),
            AutoPaperException.Storage => Loc.Get("Error_Storage"),
            AutoPaperException.Cancelled => Loc.Get("Status_Cancelled"),
            _ => Loc.Get("Error_Internal"),
        };
    }

    /// <summary>
    /// A problem as it's shown where the app can link to its fix (docs/app-spec.md 6a, "one problem, said once, as a
    /// link to the fix"): the sentence says what happened without directions ("in Settings › …"), and the link goes
    /// there. A missing or refused key is only the link ("Add your OpenAI key", "Check your OpenAI key"), which opens
    /// Settings › Keys with that key's field focused. A painting ComfyUI couldn't make is one line and a link to
    /// Providers, with no "try again later" (trying again on the timer can't fix it).
    /// </summary>
    /// <param name="inSettings">Shown in Settings › Providers: the link to Providers is left out (it's there).</param>
    public static Problem Problem(Exception error, Settings? settings, bool scheduled = false, bool inSettings = false)
    {
        var later = scheduled ? " " + Loc.Get("Error_TryLater") : "";
        Problem Linked(string sentence, string link, Fix fix, string? account = null) =>
            inSettings && fix == Fix.Providers ? new(sentence, null, Fix.None, null) : new(sentence, link, fix, account);
        return error switch
        {
            AutoPaperException.MissingKey e => new("", KeyLink(e.provider, refused: false), Fix.Keys, KeyAccount(e.provider, settings)),
            AutoPaperException.InvalidKey e => new("", KeyLink(e.provider, refused: true), Fix.Keys, KeyAccount(e.provider, settings)),
            AutoPaperException.Unsupported e => Paints(e.provider) && !Writes(e.provider)
                ? Linked(Loc.Format("Error_UnsupportedConceptsShort", Provider(e.provider)), Loc.Get("Link_ChooseWriter"), Fix.Providers)
                : Linked(Loc.Format("Error_UnsupportedImagesShort", Provider(e.provider)), Loc.Get("Link_ChoosePainter"), Fix.Providers),
            AutoPaperException.ProviderUnavailable { reason: ProviderUnavailableReason.NotRunning } e =>
                Linked(Unavailable(e.provider, e.reason, settings, scheduled), SettingsLink(e.provider), Fix.Providers),
            AutoPaperException.PaintingFailed e => Linked(PaintingFailed(e.provider, e.model), SettingsLink(e.provider), Fix.Providers),
            AutoPaperException.BudgetReached => new(Loc.Get("Error_BudgetReachedShort"), Loc.Get("Link_RaiseBudget"), Fix.Budget, null),
            // The whole line is the link: it names the keyword, and opens the mood where it's reworded, made a Maybe
            // or removed. No "try again later": asking again rarely helps.
            AutoPaperException.KeywordNotFollowed e => new("", KeywordNotFollowed(e.keyword, e.weight), Fix.Moods, null, e.moodId),
            AutoPaperException.InvalidInput e when MoodsFix(e.reason) => new(InvalidInput(e.reason), Loc.Get("Link_OpenMoods"), Fix.Moods, null),
            AutoPaperException.InvalidInput e when ProvidersFix(e.reason) =>
                Linked(InvalidInput(e.reason) + later, Loc.Get("Link_OpenProviders"), Fix.Providers),
            _ => new(Error(error, settings, scheduled), null, Fix.None, null),
        };
    }

    /// <summary>"ComfyUI couldn't paint with Qwen-Image 2.1." (the model in words, from the core), or without a
    /// model when it isn't known.</summary>
    private static string PaintingFailed(ProviderKind provider, string model) => string.IsNullOrWhiteSpace(model)
        ? Loc.Format("Error_PaintingFailedUnnamed", Provider(provider))
        : Loc.Format("Error_PaintingFailed", Provider(provider), model.Trim());

    /// <summary>The writing model kept breaking one of the mood's keywords: "The writing model kept leaving out
    /// “lighthouse”." (a Must) or "…kept including “people”, which this mood avoids." (an Avoid).</summary>
    public static string KeywordNotFollowed(string keyword, KeywordWeight weight) => Loc.Format(
        weight == KeywordWeight.Avoid ? "Error_KeywordIncluded" : "Error_KeywordLeftOut", keyword.Trim());

    /// <summary>A missing or refused key, as the link that goes to its field: "Add your OpenAI key", "Check your
    /// Google Gemini key", "Add your server's key".</summary>
    public static string KeyLink(ProviderKind provider, bool refused) => provider == ProviderKind.OpenAiCompatible
        ? Loc.Get(refused ? "Link_CheckServerKey" : "Link_AddServerKey")
        : Loc.Format(refused ? "Link_CheckKey" : "Link_AddKey", Provider(provider));

    /// <summary>"Check ComfyUI settings": a link to Settings › Providers about one provider.</summary>
    private static string SettingsLink(ProviderKind provider) => provider == ProviderKind.OpenAiCompatible
        ? Loc.Get("Link_CheckServerSettings")
        : Loc.Format("Link_CheckSettings", Provider(provider));

    /// <summary>The Credential Manager account (secret_account_for) of the key a provider uses with these settings:
    /// for an OpenAI-compatible server, the one at the chosen address.</summary>
    public static string? KeyAccount(ProviderKind provider, Settings? settings)
    {
        var selection = settings is null ? null
            : settings.TextProvider.Kind == provider ? settings.TextProvider
            : settings.ImageProvider.Kind == provider ? settings.ImageProvider
            : null;
        return AutopaperCoreMethods.SecretAccountFor(selection ?? new ProviderSelection(provider, "", null));
    }

    private static bool MoodsFix(InvalidInputReason reason) => reason is InvalidInputReason.KeywordEmpty
        or InvalidInputReason.KeywordTooLong or InvalidInputReason.TooManyKeywords or InvalidInputReason.DuplicateKeyword;

    private static bool ProvidersFix(InvalidInputReason reason) => reason is InvalidInputReason.AddressMissing
        or InvalidInputReason.AddressNotAllowed or InvalidInputReason.AddressInvalid or InvalidInputReason.NoModels
        or InvalidInputReason.WorkflowNeedsPrompt or InvalidInputReason.WorkflowNotApiFormat or InvalidInputReason.WorkflowInvalid;

    /// <summary>A provider that couldn't be reached or didn't finish, by its typed reason. On the schedule it adds
    /// that AutoPaper tries again later; asked just now, a passing problem (too slow, a server error) suggests
    /// trying again, while one the person fixes (not running) or caused (stopped in ComfyUI) doesn't.</summary>
    private static string Unavailable(ProviderKind provider, ProviderUnavailableReason reason, Settings? settings, bool scheduled)
    {
        var later = scheduled ? " " + Loc.Get("Error_TryLater") : "";
        var again = scheduled ? later : " " + Loc.Get("Error_TryAgain");
        return reason switch
        {
            ProviderUnavailableReason.NotRunning => Address(provider, settings) is { Length: > 0 } address
                ? Loc.Format("Error_NotRunningAt", Subject(provider), address) + later
                : Loc.Format("Error_NotRunning", Subject(provider)) + later,
            ProviderUnavailableReason.TimedOut => Loc.Format("Error_TimedOut", Subject(provider)) + again,
            ProviderUnavailableReason.ServerError => Loc.Format("Error_ServerError", Subject(provider)) + again,
            ProviderUnavailableReason.Stopped => Loc.Format("Error_Stopped", Provider(provider)) + later,
            _ => Loc.Format("Error_Unavailable", Subject(provider)) + again,
        };
    }

    /// <summary>What was wrong with something the person entered; a mistake of AutoPaper's own (a display size,
    /// <c>Other</c>) is said generically.</summary>
    public static string InvalidInput(InvalidInputReason reason) => reason switch
    {
        InvalidInputReason.KeywordEmpty => Loc.Get("Invalid_KeywordEmpty"),
        InvalidInputReason.KeywordTooLong => Loc.Get("Invalid_KeywordTooLong"),
        InvalidInputReason.TooManyKeywords => Loc.Get("Invalid_TooManyKeywords"),
        InvalidInputReason.DuplicateKeyword => Loc.Get("Invalid_DuplicateKeyword"),
        InvalidInputReason.AddressMissing => Loc.Get("Invalid_AddressMissing"),
        InvalidInputReason.AddressNotAllowed => Loc.Get("Invalid_AddressNotAllowed"),
        InvalidInputReason.AddressInvalid => Loc.Get("Invalid_AddressInvalid"),
        InvalidInputReason.NoModels => Loc.Get("Invalid_NoModels"),
        InvalidInputReason.WorkflowNeedsPrompt => Loc.Get("Invalid_WorkflowNeedsPrompt"),
        InvalidInputReason.WorkflowNotApiFormat => Loc.Get("Invalid_WorkflowNotApiFormat"),
        InvalidInputReason.WorkflowInvalid => Loc.Get("Invalid_WorkflowInvalid"),
        InvalidInputReason.NothingToEcho => Loc.Get("Invalid_NothingToEcho"),
        InvalidInputReason.MoodNameEmpty => Loc.Get("Invalid_MoodNameEmpty"),
        InvalidInputReason.MoodNameTooLong => Loc.Get("Invalid_MoodNameTooLong"),
        InvalidInputReason.DuplicateMoodName => Loc.Get("Invalid_DuplicateMoodName"),
        InvalidInputReason.LastMood => Loc.Get("Invalid_LastMood"),
        _ => Loc.Get("Error_Internal"),
    };

    /// <summary>The address a local provider is used at: the one in Settings, else the core's default.</summary>
    public static string? Address(ProviderKind provider, Settings? settings)
    {
        var chosen = settings is null ? null
            : settings.ImageProvider.Kind == provider ? settings.ImageProvider.BaseUrl
            : settings.TextProvider.Kind == provider ? settings.TextProvider.BaseUrl
            : null;
        return string.IsNullOrWhiteSpace(chosen) ? AutopaperCoreMethods.DefaultBaseUrl(provider) : chosen.Trim();
    }

    /// <summary>The provider as the subject of a sentence ("ComfyUI isn't running…", "Your OpenAI-compatible server
    /// took too long…").</summary>
    private static string Subject(ProviderKind kind) => kind switch
    {
        ProviderKind.OpenAiCompatible => Loc.Get("Provider_CompatibleSubject"),
        _ => Provider(kind),
    };

    /// <summary>Providers for writing ideas, in Settings' order (spec: OpenAI · Google Gemini · Ollama ·
    /// OpenAI-compatible · Demo).</summary>
    public static readonly ProviderKind[] Writers =
        [ProviderKind.OpenAi, ProviderKind.Google, ProviderKind.Ollama, ProviderKind.OpenAiCompatible, ProviderKind.Demo];

    /// <summary>Providers for painting (OpenAI · Google Gemini · OpenAI-compatible · ComfyUI · Demo).</summary>
    public static readonly ProviderKind[] Painters =
        [ProviderKind.OpenAi, ProviderKind.Google, ProviderKind.OpenAiCompatible, ProviderKind.ComfyUi, ProviderKind.Demo];

    private static bool Writes(ProviderKind kind) => Writers.Contains(kind);

    private static bool Paints(ProviderKind kind) => Painters.Contains(kind);

    /// <summary>Providers that can't work without a key of the person's (an OpenAI-compatible server's key is
    /// optional: only if the server asks for one).</summary>
    public static bool NeedsKey(ProviderKind kind) => kind is ProviderKind.OpenAi or ProviderKind.Google;

    /// <summary>The model picker's blank choice: "Default (gpt-6-luna)", from the core's <c>default_model</c>, named
    /// as the provider lists it when it does, else, for ComfyUI, by AutoPaper's own workflow for it ("Default (Z-Image
    /// Turbo)" for the file z_image_turbo_bf16.safetensors, also while ComfyUI can't be asked). Ollama and
    /// OpenAI-compatible servers have no fixed default (they use the first model they list).</summary>
    public static string DefaultModelChoice(ProviderKind kind, ProviderJob job, IEnumerable<ModelInfo> listed)
    {
        var model = AutopaperCoreMethods.DefaultModel(kind, job);
        if (model.Length > 0)
        {
            var named = listed.FirstOrDefault(info => info.Id == model)?.DisplayName;
            if (string.IsNullOrWhiteSpace(named) && kind == ProviderKind.ComfyUi)
            {
                named = AutoPaper.Models.BundledWorkflows.NameOf(model);
            }
            return Loc.Format("Models_DefaultNamed", string.IsNullOrWhiteSpace(named) ? model : named);
        }
        return kind is ProviderKind.Ollama or ProviderKind.OpenAiCompatible ? Loc.Get("Models_DefaultFirst") : Loc.Get("Models_Default");
    }

    /// <summary>The person's own lock screen put back (Restore my wallpaper, quit), or what Windows didn't let AutoPaper
    /// put back: Spotlight and a slideshow only the person can turn on again.</summary>
    public static string LockScreenRestored(LockScreenNote note) => Loc.Get(note switch
    {
        LockScreenNote.Spotlight => "OwnLockScreen_Spotlight",
        LockScreenNote.Slideshow => "OwnLockScreen_Slideshow",
        LockScreenNote.Unreadable => "OwnLockScreen_Unreadable",
        LockScreenNote.Failed => "OwnLockScreen_Failed",
        _ => "OwnLockScreen_Restored",
    });

    /// <summary>The link to Windows' lock screen settings that goes with a note, or null when there's nothing to fix.</summary>
    public static string? LockScreenLink(LockScreenNote note) => note switch
    {
        LockScreenNote.Spotlight => Loc.Get("OwnLockScreen_TurnOnSpotlight"),
        LockScreenNote.Slideshow => Loc.Get("OwnLockScreen_TurnOnSlideshow"),
        LockScreenNote.Unreadable or LockScreenNote.Failed => Loc.Get("OwnLockScreen_Choose"),
        _ => null,
    };

    public static string Revisit(RevisitReason reason, Settings settings) => reason switch
    {
        RevisitReason.ServicesUnavailable => Loc.Get("Revisit_ServicesUnavailable"),
        RevisitReason.OverBudget => Loc.Get("Revisit_OverBudget"),
        RevisitReason.Offline => Loc.Get("Revisit_Offline"),
        RevisitReason.ProviderFailed => Loc.Format("Revisit_ProviderFailed", Provider(settings.ImageProvider.Kind)),
        _ => Loc.Get("Revisit_Requested"),
    };

    // ── Status ──────────────────────────────────────────────────────────────────────────────────

    /// <summary>"Next new wallpaper at 9:00", "… tomorrow at 9:00", or why there's none.</summary>
    public static string Next(long? due, Settings settings)
    {
        if (settings.Paused)
        {
            return Loc.Get("Next_Paused");
        }
        if (settings.Cadence == Core.Cadence.Manual)
        {
            return Loc.Get("Next_Manual");
        }
        if (due is null)
        {
            return "";
        }
        var at = DateTimeOffset.FromUnixTimeSeconds(due.Value).ToLocalTime();
        var now = DateTimeOffset.Now;
        if (at <= now.AddSeconds(60))
        {
            return Loc.Get("Next_Soon");
        }
        var time = Time(at);
        var days = (at.Date - now.Date).Days;
        return days switch
        {
            0 => Loc.Format("Next_Today", time),
            1 => Loc.Format("Next_Tomorrow", time),
            < 7 => Loc.Format("Next_Weekday", Clean(new DateTimeFormatter("dayofweek.full").Format(at)), time),
            _ => Loc.Format("Next_Date", Date(at), time),
        };
    }

    /// <summary>
    /// A length of time rounded the way a person says it, for "about 6 minutes left" and "about 9 minutes per
    /// wallpaper": seconds to the nearest 5 under a minute (10 s or more), whole minutes under an hour (90 s and
    /// up; 60–89 s is "a minute"), then hours and minutes (minutes to the nearest 5).
    /// </summary>
    public static Span Round(uint seconds)
    {
        if (seconds < 10)
        {
            return new Span(SpanUnit.FewSeconds, 0, 0);
        }
        if (seconds < 58)
        {
            return new Span(SpanUnit.Seconds, (uint)Math.Max(10, Math.Round(seconds / 5.0, MidpointRounding.AwayFromZero) * 5), 0);
        }
        if (seconds < 90)
        {
            return new Span(SpanUnit.Minute, 1, 0);
        }
        var minutes = (uint)Math.Round(seconds / 60.0, MidpointRounding.AwayFromZero);
        if (minutes < 60)
        {
            return new Span(SpanUnit.Minutes, minutes, 0);
        }
        var fives = (uint)Math.Round(seconds / 300.0, MidpointRounding.AwayFromZero) * 5;
        var (hours, rest) = (fives / 60, fives % 60);
        return rest == 0 ? new Span(hours == 1 ? SpanUnit.Hour : SpanUnit.Hours, hours, 0) : new Span(SpanUnit.HoursMinutes, hours, rest);
    }

    /// <summary>"about 6 minutes", "about 20 seconds", "about 1 hour 10 minutes", "a few seconds" (lower case: it's
    /// put into sentences; <see cref="Sentence"/> capitalises it at the start of one).</summary>
    public static string About(uint seconds)
    {
        var span = Round(seconds);
        return span.Unit switch
        {
            SpanUnit.FewSeconds => Loc.Get("Span_FewSeconds"),
            SpanUnit.Seconds => Loc.Format("Span_Seconds", span.Amount),
            SpanUnit.Minute => Loc.Get("Span_Minute"),
            SpanUnit.Minutes => Loc.Format("Span_Minutes", span.Amount),
            SpanUnit.Hour => Loc.Get("Span_Hour"),
            SpanUnit.Hours => Loc.Format("Span_Hours", span.Amount),
            _ => Loc.Format(span.Amount == 1 ? "Span_HourMinutes" : "Span_HoursMinutes", span.Amount, span.Minutes),
        };
    }

    /// <summary>While painting: "About 6 minutes left", "Almost done" (only decoding and saving left), or nothing
    /// when there's no estimate (the ring is indeterminate then).</summary>
    public static string TimeLeft(uint? seconds) => seconds switch
    {
        null => "",
        0 => Loc.Get("TimeLeft_AlmostDone"),
        { } left => Sentence(Loc.Format("TimeLeft_Left", About(left))),
    };

    /// <summary>The stage with its time left, for the notification-area menu and tooltip ("Painting… about 6
    /// minutes left", "Painting… almost done").</summary>
    public static string StageWithTimeLeft(string stage, uint? seconds) => seconds switch
    {
        _ when stage.Length == 0 => stage,
        null => stage,
        0 => Loc.Format("Stage_AlmostDone", stage),
        { } left => Loc.Format("Stage_WithTimeLeft", stage, Loc.Format("TimeLeft_Left", About(left))),
    };

    /// <summary>How long a model takes here, for its card in Settings › Providers: "About 9 minutes per wallpaper on
    /// this PC." (painting, with the idea's writing when that's known too), "About 20 seconds per idea on this PC.".
    /// Empty when nothing is recorded yet (the core never makes a number up).</summary>
    public static string Estimate(ProviderJob job, uint? seconds) => seconds is { } known
        ? Sentence(Loc.Format(job == ProviderJob.Images ? "Estimate_PerWallpaper" : "Estimate_PerIdea", About(known)))
        : "";

    /// <summary>The text with its first letter capitalised (a phrase put at the start of a sentence).</summary>
    public static string Sentence(string text) =>
        text.Length == 0 ? text : char.ToUpper(text[0], CultureInfo.CurrentCulture) + text[1..];

    public static string Budget(SpendSummary spend) => spend.BudgetCents is { } cents
        ? Loc.Format("Budget_OfLimit", Money(spend.SpentMicrousd), Money((ulong)cents * 10_000))
        : Loc.Format("Budget_NoLimit", Money(spend.SpentMicrousd));

    /// <summary>US dollars (the price table's currency), "$1.20"; amounts under a cent say so.</summary>
    public static string Money(ulong microusd)
    {
        if (microusd > 0 && microusd < 5_000)
        {
            return Loc.Get("Money_UnderACent");
        }
        return (microusd / 1_000_000m).ToString("C2", Dollars);
    }

    public static string Dollars0(uint cents) => (cents / 100m).ToString(cents % 100 == 0 ? "C0" : "C2", Dollars);

    /// <summary>The Console keeps the recorded microUSD precision, including paid writing that costs under a cent.</summary>
    public static string RunMoney(ulong microusd) => "$" + (microusd / 1_000_000m).ToString("#,0.00####", Dollars);

    public static string RunRate(double? fraction) => fraction?.ToString("P0", CultureInfo.CurrentCulture) ?? "—";

    public static string RunSeconds(double? seconds)
    {
        if (seconds is not { } value || !double.IsFinite(value) || value < 0) return "—";
        // "8.4 s", "35 s", "5 min 22 s": a chart label, not a measurement printout.
        if (value < 0.1) return Loc.Format("Console_Seconds", value.ToString("0.###", CultureInfo.CurrentCulture));
        if (value < 10) return Loc.Format("Console_Seconds", value.ToString("0.0", CultureInfo.CurrentCulture));
        var whole = (long)Math.Round(value);
        if (whole < 60) return Loc.Format("Console_Seconds", whole.ToString(CultureInfo.CurrentCulture));
        var minutes = whole / 60;
        var rest = whole % 60;
        var hours = minutes / 60;
        var text = hours > 0 ? $"{hours} hr {minutes % 60} min" : $"{minutes} min";
        return rest > 0 && hours == 0 ? $"{text} {Loc.Format("Console_Seconds", rest.ToString(CultureInfo.CurrentCulture))}" : text;
    }

    /// <summary>The Console's outcomes include requests stopped before any provider was contacted.</summary>
    public static string RunOutcome(RunStatus status) => Loc.Get("ConsoleStatus_" + status);

    public static string RunTrigger(Trigger trigger) => Loc.Get("ConsoleTrigger_" + trigger);

    /// <summary>The precise model id is kept visible for debugging, including when a provider's friendly name differs.</summary>
    public static string RunProvider(ProviderKind provider, string model) => string.IsNullOrEmpty(model)
        ? Provenance.ProviderName(provider)
        : Loc.Format("Console_ProviderModel", Provenance.ProviderName(provider), model);

    public static string Size(ulong bytes)
    {
        string[] units = ["Size_Bytes", "Size_KB", "Size_MB", "Size_GB", "Size_TB"];
        double value = bytes;
        var unit = 0;
        while (value >= 1024 && unit < units.Length - 1)
        {
            value /= 1024;
            unit++;
        }
        return Loc.Format(units[unit], unit == 0 ? value.ToString("N0", CultureInfo.CurrentCulture) : value.ToString(value < 10 ? "N1" : "N0", CultureInfo.CurrentCulture));
    }

    /// <summary>Surprise in words, by the core's own bands (<c>surprise_band</c>), so the thresholds are never
    /// copied here.</summary>
    public static (string Name, string Detail) Band(int percent)
    {
        var band = AutopaperCoreMethods.SurpriseBand(percent / 100f);
        return (Loc.Get($"Surprise_{band}"), Loc.Get($"Surprise_{band}Detail"));
    }

    public static string SurpriseSpoken(int percent) =>
        Loc.Format("Surprise_Spoken", percent, Band(percent).Name.ToLower(CultureInfo.CurrentCulture));

    /// <summary>The rating in the app's words, for accessible descriptions ("Liked."); empty when unrated.</summary>
    public static string RatingWords(Rating rating) => rating switch
    {
        Rating.Liked => Loc.Get("Rating_Liked"),
        Rating.Disliked => Loc.Get("Rating_Disliked"),
        _ => "",
    };

    /// <summary><c>describe(id)</c> plus the rating, as the spec asks hosts to say it.</summary>
    public static string Spoken(string description, Rating rating)
    {
        var words = RatingWords(rating);
        return words.Length == 0 ? description : $"{description} {words}.";
    }

    /// <summary>The date in the person's regional format ("October 5, 2026"). WinRT's formatter adds invisible
    /// left-to-right marks around each part; they're removed so names read and compare cleanly.</summary>
    public static string Date(DateTimeOffset at) => Clean(new DateTimeFormatter("day month.full year").Format(at));

    public static string Time(DateTimeOffset at) => Clean(new DateTimeFormatter("shorttime").Format(at));

    /// <summary>A WinRT-formatted date without its invisible direction marks.</summary>
    public static string Clean(string formatted) => formatted.Replace("‎", "").Replace("‏", "");

    public static string WhenMade(long createdAt)
    {
        var at = DateTimeOffset.FromUnixTimeSeconds(createdAt).ToLocalTime();
        return Loc.Format("History_Made", Date(at), Time(at));
    }

    public static string Cadence(Cadence cadence) => Loc.Get($"Cadence_{cadence}");

    /// <summary>"1 image" / "3 images" (noun: "Image" or "Wallpaper").</summary>
    public static string Count(ulong count, string noun) =>
        count == 1 ? Loc.Get($"Count_{noun}One") : Loc.Format($"Count_{noun}Many", count.ToString("N0", CultureInfo.CurrentCulture));
}

/// <summary>Where a problem is fixed (its link opens it).</summary>
internal enum Fix
{
    None,
    Keys,
    Providers,
    Budget,
    Moods,
}

/// <summary>A problem as shown with a link to its fix (docs/app-spec.md 6a).</summary>
/// <param name="Sentence">What happened, without directions; empty when the link says it all ("Add your OpenAI key").</param>
/// <param name="Link">The link's words ("Check ComfyUI settings"), or null when there's nothing for the person to change.</param>
/// <param name="Fix">Where the link goes.</param>
/// <param name="KeyAccount">For <see cref="Fix.Keys"/>: the key whose field gets focus (secret_account_for).</param>
/// <param name="MoodId">For <see cref="Fix.Moods"/>: the mood the link opens (null: the mood in use).</param>
internal sealed record Problem(string Sentence, string? Link, Fix Fix, string? KeyAccount, string? MoodId = null)
{
    /// <summary>All of it in words, for a screen reader or a line without a link: the sentence, then the link's words.</summary>
    public string Spoken => Link is null ? Sentence : Sentence.Length == 0 ? Link : $"{Sentence} {Link}.";
}

/// <summary>A rounded length of time (<see cref="Text.Round"/>).</summary>
internal readonly record struct Span(SpanUnit Unit, uint Amount, uint Minutes);

internal enum SpanUnit
{
    FewSeconds,
    Seconds,
    Minute,
    Minutes,
    Hour,
    Hours,
    HoursMinutes,
}
