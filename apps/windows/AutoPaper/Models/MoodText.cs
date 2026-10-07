using AutoPaper.Core;
using AutoPaper.Services;

namespace AutoPaper.Models;

/// <summary>
/// Moods in words: names for new and duplicated moods, a mood's keywords in short, and what a screen reader says for
/// a mood. The engine owns the rules (1–40 characters, unique ignoring case); these only pick names that pass them,
/// so New mood and Duplicate never fail on a name the person didn't type. No WinUI types (AutoPaper.Tests links it).
/// </summary>
internal static class MoodText
{
    /// <summary>The core's <c>Mood::MAX_NAME_LEN</c> (not exported; longer names are refused with MoodNameTooLong).</summary>
    public const int MaxNameLength = 40;

    /// <summary>The engine's own normalising (trimmed, single-spaced), for keywords and mood names.</summary>
    public static string Normalise(string text) =>
        string.Join(' ', text.Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries));

    /// <summary><paramref name="name"/>, or "name 2", "name 3"…: the first that no mood has (ignoring case and spacing),
    /// cut to fit 40 characters with its number.</summary>
    public static string Unique(string name, IEnumerable<string> taken)
    {
        var names = taken.Select(Key).ToHashSet();
        var trimmed = Normalise(name);
        for (var number = 1; ; number++)
        {
            var suffix = number == 1 ? "" : $" {number}";
            var stem = trimmed.Length + suffix.Length > MaxNameLength ? trimmed[..(MaxNameLength - suffix.Length)].TrimEnd() : trimmed;
            var candidate = stem + suffix;
            if (!names.Contains(Key(candidate)))
            {
                return candidate;
            }
        }
    }

    /// <summary>New mood's name: "New mood", then "New mood 2".</summary>
    public static string NewName(IEnumerable<string> taken) => Unique(Loc.Get("Mood_NewName"), taken);

    /// <summary>Duplicate's name: "Rainy beach copy", then "Rainy beach copy 2" (File Explorer's "copy" pattern).</summary>
    public static string CopyName(string name, IEnumerable<string> taken)
    {
        var suffix = Loc.Get("Mood_CopySuffix");
        // Room for the suffix and a number after it (" 99").
        var room = MaxNameLength - suffix.Length - 3;
        var stem = Normalise(name);
        stem = stem.Length > room ? stem[..room].TrimEnd() : stem;
        return Unique(stem + suffix, taken);
    }

    /// <summary>A mood's keywords in short, in their order: "rain · beach · night · no people" (an Avoid reads as what's
    /// left out); "No keywords yet" when it has none.</summary>
    public static string Summary(IReadOnlyCollection<Keyword> keywords) => keywords.Count == 0
        ? Loc.Get("Mood_NoKeywords")
        : string.Join(" · ", keywords.Select(Short));

    /// <summary>The same for a screen reader, with commas: "rain, beach, night, no people".</summary>
    public static string SpokenSummary(IReadOnlyCollection<Keyword> keywords) => keywords.Count == 0
        ? Loc.Get("Mood_NoKeywords")
        : string.Join(", ", keywords.Select(Short));

    /// <summary>A mood's name as a screen reader says it in lists and menus: "Rainy beach, current mood".</summary>
    public static string SpokenName(Mood mood) => mood.Active ? Loc.Format("Mood_SpokenCurrent", mood.Name) : mood.Name;

    /// <summary>A list row: its name (and "current mood"), then its keywords in short.</summary>
    public static string RowName(Mood mood) => Loc.Format("Mood_RowName", SpokenName(mood), SpokenSummary(mood.Keywords));

    // ── What moods have made (the Moods summary, a mood's header; mood_stats) ──────────────────────

    /// <summary>"Surprise: Fresh (35%)".</summary>
    public static string SurpriseLine(float surprise)
    {
        var percent = Percent(surprise);
        return Loc.Format("Mood_SurpriseLine", Text.Band(percent).Name, percent);
    }

    /// <summary>A mood's Surprise as a whole percentage (0–100).</summary>
    public static int Percent(float surprise) => (int)Math.Round(Math.Clamp(float.IsNaN(surprise) ? 0 : surprise, 0, 1) * 100, MidpointRounding.AwayFromZero);

    /// <summary>What a mood has made, on one line: "12 wallpapers · 3 liked · last made yesterday", or "Nothing made
    /// yet". <paramref name="separator"/> is " · " to show and ", " to speak.</summary>
    public static string MadeLine(uint wallpapers, uint liked, long? lastMadeAt, string separator = " · ", DateTimeOffset? now = null)
    {
        if (wallpapers == 0)
        {
            return Loc.Get("Mood_NothingMade");
        }
        var parts = new List<string> { Text.Count(wallpapers, "Wallpaper"), Loc.Format("Mood_Liked", liked) };
        if (lastMadeAt is { } made)
        {
            parts.Add(Loc.Format("Mood_LastMade", Relative(made, now)));
        }
        return string.Join(separator, parts);
    }

    /// <summary>When, as a person says it in a sentence: "just now", "5 minutes ago", "3 hours ago", "yesterday",
    /// "4 days ago", "2 weeks ago", else the date. Never in the future (a clock that moved back reads "just now").</summary>
    public static string Relative(long unix, DateTimeOffset? now = null)
    {
        var at = DateTimeOffset.FromUnixTimeSeconds(unix).ToLocalTime();
        var current = (now ?? DateTimeOffset.Now).ToLocalTime();
        var ago = current - at;
        if (ago < TimeSpan.FromMinutes(1))
        {
            return Loc.Get("Relative_JustNow");
        }
        if (ago < TimeSpan.FromHours(1))
        {
            var minutes = (int)ago.TotalMinutes;
            return minutes == 1 ? Loc.Get("Relative_MinuteAgo") : Loc.Format("Relative_MinutesAgo", minutes);
        }
        var days = (current.Date - at.Date).Days;
        if (days == 0)
        {
            var hours = (int)ago.TotalHours;
            return hours == 1 ? Loc.Get("Relative_HourAgo") : Loc.Format("Relative_HoursAgo", hours);
        }
        if (days == 1)
        {
            return Loc.Get("Relative_Yesterday");
        }
        if (days < 7)
        {
            return Loc.Format("Relative_DaysAgo", days);
        }
        if (days < 31)
        {
            var weeks = days / 7;
            return weeks == 1 ? Loc.Get("Relative_WeekAgo") : Loc.Format("Relative_WeeksAgo", weeks);
        }
        return Loc.Format("Relative_On", Text.Date(at));
    }

    /// <summary>The same at the start of a stat tile: "Yesterday", "4 hours ago", "Not yet".</summary>
    public static string LastMadeTile(long? lastMadeAt, DateTimeOffset? now = null) =>
        lastMadeAt is { } made ? Text.Sentence(Relative(made, now)) : Loc.Get("Mood_NotYet");

    /// <summary>A mood's keywords grouped by weight, Must then Maybe then Avoid (each in the mood's order), leaving out
    /// weights it has none of: "Must: rain, beach".</summary>
    public static IReadOnlyList<string> KeywordGroups(IEnumerable<Keyword> keywords) =>
        Groups(keywords.Select(keyword => (keyword.Text, keyword.Weight)));

    /// <summary>The keywords a wallpaper was made with (its snapshot), grouped the same way.</summary>
    public static IReadOnlyList<string> KeywordGroups(IEnumerable<KeywordSnapshot> keywords) =>
        Groups(keywords.Select(keyword => (keyword.Text, keyword.Weight)));

    private static List<string> Groups(IEnumerable<(string Text, KeywordWeight Weight)> keywords)
    {
        var all = keywords.ToList();
        return new[] { KeywordWeight.Must, KeywordWeight.Maybe, KeywordWeight.Avoid }
            .Select(weight => (weight, words: all.Where(keyword => keyword.Weight == weight).Select(keyword => keyword.Text).ToList()))
            .Where(group => group.words.Count > 0)
            .Select(group => Loc.Format("Mood_KeywordGroup", WeightWord(group.weight), string.Join(", ", group.words)))
            .ToList();
    }

    /// <summary>Every mood's keywords by weight, for the summary: "12 Must · 7 Maybe · 3 Avoid keywords".</summary>
    public static string KeywordBreakdown(IEnumerable<Mood> moods)
    {
        var keywords = moods.SelectMany(mood => mood.Keywords).ToList();
        if (keywords.Count == 0)
        {
            return Loc.Get("Mood_NoKeywords");
        }
        int Of(KeywordWeight weight) => keywords.Count(keyword => keyword.Weight == weight);
        return Loc.Format(keywords.Count == 1 ? "Mood_BreakdownOne" : "Mood_Breakdown", Of(KeywordWeight.Must), Of(KeywordWeight.Maybe), Of(KeywordWeight.Avoid));
    }

    /// <summary>A mood's latest wallpapers as one spoken line: "Latest wallpapers: Harbour at dusk, Fog".</summary>
    public static string SpokenLatest(IReadOnlyList<string> titles) => titles.Count == 1
        ? Loc.Format("Mood_LatestOne", titles[0])
        : Loc.Format("Mood_LatestMany", string.Join(", ", titles));

    /// <summary>The totals above the summary of every mood, as tiles: moods, wallpapers, liked, echoes.</summary>
    public static IReadOnlyList<Total> Totals(int moods, IEnumerable<MoodStats> stats)
    {
        var all = stats.ToList();
        var made = all.Sum(entry => (long)entry.Wallpapers);
        var liked = all.Sum(entry => (long)entry.Liked);
        var echoes = all.Sum(entry => (long)entry.Echoes);
        return
        [
            new Total(Loc.Get("Stat_Moods"), moods, Loc.Format(moods == 1 ? "Stat_MoodsSpokenOne" : "Stat_MoodsSpoken", moods)),
            new Total(Loc.Get("Stat_Wallpapers"), made, Text.Count((ulong)made, "Wallpaper")),
            new Total(Loc.Get("Stat_Liked"), liked, Loc.Format("Mood_Liked", liked)),
            new Total(Loc.Get("Stat_Echoes"), echoes, Loc.Format(echoes == 1 ? "Stat_EchoesSpokenOne" : "Stat_EchoesSpoken", echoes)),
        ];
    }

    /// <summary>One total: its tile's caption, its number, and how it's spoken ("3 moods").</summary>
    internal sealed record Total(string Caption, long Value, string Spoken);

    /// <summary>Must, Maybe or Avoid in words.</summary>
    public static string WeightWord(KeywordWeight weight) => weight switch
    {
        KeywordWeight.Maybe => Loc.Get("Weight_Maybe"),
        KeywordWeight.Avoid => Loc.Get("Weight_Avoid"),
        _ => Loc.Get("Weight_Must"),
    };

    private static string Short(Keyword keyword) =>
        keyword.Weight == KeywordWeight.Avoid ? Loc.Format("Mood_AvoidShort", keyword.Text) : keyword.Text;

    private static string Key(string name) => Normalise(name).ToLowerInvariant();
}
