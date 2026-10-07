using AutoPaper.Core;
using AutoPaper.Services;

namespace AutoPaper.Models;

/// <summary>
/// The Moods summary's chart (user, 2026-10-06; the macOS app's MoodActivity): wallpapers per day over the last 30
/// days, stacked by mood, from the engine's activity(day_bounds). Days are the person's local days; each mood keeps
/// one colour (by when it was made, so reordering the list doesn't repaint it); past eight moods, and for moods since
/// deleted, the bars fold into "Other moods". No WinUI types: AutoPaper.Tests links it.
/// </summary>
internal static class MoodActivity
{
    public const int Days = 30;

    /// <summary>Categorical colours available (the palette's eight; the ninth series is "Other moods").</summary>
    public const int Slots = 8;

    /// <summary>The day_bounds for activity(): the local midnight starting each of the last <paramref name="days"/>
    /// days (today last), then tomorrow's, so a 23- or 25-hour day (daylight saving) is still one day.</summary>
    public static long[] DayBounds(int days = Days, DateTime? now = null, TimeZoneInfo? zone = null)
    {
        zone ??= TimeZoneInfo.Local;
        var local = now is { } given ? TimeZoneInfo.ConvertTime(given, zone) : TimeZoneInfo.ConvertTime(DateTime.UtcNow, zone);
        var today = DateTime.SpecifyKind(local.Date, DateTimeKind.Unspecified);
        return Enumerable.Range(0, days + 1)
            .Select(offset => today.AddDays(offset - days + 1))
            .Select(day => new DateTimeOffset(day, zone.GetUtcOffset(day)).ToUnixTimeSeconds())
            .ToArray();
    }

    /// <summary>Each mood's colour slot (0–7), by when it was made (oldest first; then id). With more than eight moods
    /// the newest share "Other moods" (no slot), so no colour is ever made up past the palette.</summary>
    public static IReadOnlyDictionary<string, int> ColorSlots(IEnumerable<Mood> moods)
    {
        var byAge = moods.OrderBy(mood => mood.CreatedAt).ThenBy(mood => mood.Id, StringComparer.Ordinal).ToList();
        var coloured = byAge.Count > Slots ? byAge.Take(Slots - 1) : byAge;
        return coloured.Select((mood, slot) => (mood.Id, slot)).ToDictionary(entry => entry.Id, entry => entry.slot);
    }

    /// <summary>One stacked segment: a day (its start), a mood (null: "Other moods"), how many, where it starts in its
    /// day's stack, and whether it's the day's top segment.</summary>
    internal sealed record Segment(long DayStart, string? MoodId, int Count, int Base, bool IsTop);

    /// <summary>The engine's counts as stacked segments, each day's in the moods' list order with "Other moods" on
    /// top. <paramref name="order"/> is the moods' ids in the person's order; <paramref name="slots"/> from
    /// <see cref="ColorSlots"/>.</summary>
    public static IReadOnlyList<Segment> Segments(IEnumerable<DayCount> counts, IReadOnlyList<string> order, IReadOnlyDictionary<string, int> slots)
    {
        var place = order.Select((id, index) => (id, index)).ToDictionary(entry => entry.id, entry => entry.index);
        var byDay = new SortedDictionary<long, Dictionary<string, int>>();
        const string Other = "";
        foreach (var count in counts)
        {
            var series = count.MoodId is { } id && slots.ContainsKey(id) ? id : Other;
            if (!byDay.TryGetValue(count.DayStart, out var day))
            {
                byDay[count.DayStart] = day = [];
            }
            day[series] = day.GetValueOrDefault(series) + (int)count.Count;
        }
        int Rank(string series) => series == Other ? int.MaxValue : place.GetValueOrDefault(series, int.MaxValue - 1);
        var segments = new List<Segment>();
        foreach (var (start, day) in byDay)
        {
            var stack = day.Where(entry => entry.Value > 0).OrderBy(entry => Rank(entry.Key)).ThenBy(entry => entry.Key, StringComparer.Ordinal).ToList();
            var below = 0;
            for (var index = 0; index < stack.Count; index++)
            {
                var (series, number) = (stack[index].Key, stack[index].Value);
                segments.Add(new Segment(start, series == Other ? null : series, number, below, index == stack.Count - 1));
                below += number;
            }
        }
        return segments;
    }

    /// <summary>The scale: whole numbers from 0 to a top at or above <paramref name="maximum"/>, at most five ticks
    /// (0, 1, 2 · 0, 2, 4, 6 · 0, 5, 10, 15…).</summary>
    public static IReadOnlyList<int> Ticks(int maximum)
    {
        var top = Math.Max(maximum, 1);
        int[] steps = [1, 2, 5, 10, 20, 25, 50, 100, 200, 250, 500, 1_000];
        var step = steps.FirstOrDefault(candidate => (top + candidate - 1) / candidate <= 4);
        if (step == 0)
        {
            step = Math.Max(1, top / 4);
        }
        var last = (top + step - 1) / step * step;
        return Enumerable.Range(0, last / step + 1).Select(index => index * step).ToList();
    }

    /// <summary>Where the day axis is labelled: a week apart, ending five days before the last day, so the last label
    /// sits clear of the scale on the trailing side (<paramref name="bounds"/> as from <see cref="DayBounds"/>).</summary>
    public static IReadOnlyList<long> WeekLabels(IReadOnlyList<long> bounds)
    {
        var starts = bounds.Take(Math.Max(0, bounds.Count - 1)).ToList();
        var labels = new List<long>();
        for (var index = starts.Count - 6; index >= 0; index -= 7)
        {
            labels.Insert(0, starts[index]);
        }
        return labels;
    }

    /// <summary>"42 wallpapers in the last 30 days. Kept on this PC only."</summary>
    public static string Footnote(int total) => Loc.Format("Activity_Footnote", Text.Count((ulong)total, "Wallpaper"), Days);

    /// <summary>One day as the table and a screen reader say it: "Rainy beach 2, Night city 1".</summary>
    public static string DayLine(IEnumerable<Segment> day, Func<string?, string> name) =>
        string.Join(", ", day.Select(segment => Loc.Format("Activity_DayPart", name(segment.MoodId), segment.Count)));
}
