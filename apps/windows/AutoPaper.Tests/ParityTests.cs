using AutoPaper.Core;
using AutoPaper.Models;
using AutoPaper.Services;

namespace AutoPaper.Tests;

/// <summary>
/// The logic behind the Windows app's parity with the macOS app (2026-10-06), read against the app's own strings:
/// the one-line KeywordNotFollowed link to its mood, the provenance line, Now's reload/stop, what moods have made in
/// words, and the summary's 30-day chart.
/// </summary>
[TestClass]
public sealed class ParityTests
{
    [TestInitialize]
    public void UseAppStrings() => ReswStrings.Use();

    // ── KeywordNotFollowed: one line naming the keyword, a link that opens the mood ───────────────

    [TestMethod]
    public void KeywordNotFollowedNamesTheKeywordAsALinkToItsMood()
    {
        var mustProblem = Text.Problem(new AutoPaperException.KeywordNotFollowed("lighthouse", KeywordWeight.Must, "mood-1"), null, scheduled: true);
        Assert.AreEqual("", mustProblem.Sentence, "One line: the link says it all.");
        Assert.AreEqual("The writing model kept leaving out “lighthouse”.", mustProblem.Link);
        Assert.AreEqual(Fix.Moods, mustProblem.Fix);
        Assert.AreEqual("mood-1", mustProblem.MoodId);
        Assert.AreEqual(mustProblem.Link, mustProblem.Spoken);

        var avoid = Text.Problem(new AutoPaperException.KeywordNotFollowed(" people ", KeywordWeight.Avoid, "mood-2"), null);
        Assert.AreEqual("The writing model kept including “people”, which this mood avoids.", avoid.Link);
        Assert.AreEqual("mood-2", avoid.MoodId);

        // Where there's no link (the notification-area menu, a toast), the same words.
        Assert.AreEqual("The writing model kept leaving out “lighthouse”.",
            Text.Error(new AutoPaperException.KeywordNotFollowed("lighthouse", KeywordWeight.Must, "mood-1"), null, scheduled: true));
    }

    // ── Provenance ──────────────────────────────────────────────────────────────────────────────

    private static Generation Made(ProviderKind writer, string textModel, ProviderKind painter, string imageModel, uint width = 3840, uint height = 2160, ulong cost = 0) => new(
        "g1", 1_780_000_000, Trigger.Manual, GenerationStatus.Ok,
        new Concept("Harbour at dusk", "A harbour.", "", "", [], "", "", "", [], [], "", "", [], [], ""),
        null, null, width, height, Rating.Unrated, null, null, writer, textModel, painter, imageModel, 0.35f,
        [new KeywordSnapshot("rain", KeywordWeight.Must)], cost, null, 0, null, "m1", "Rainy beach");

    [TestMethod]
    public void ProvenanceSaysWhoWroteAndPaintedIt()
    {
        var demo = Provenance.Of(Made(ProviderKind.Demo, "", ProviderKind.Demo, "", 1440, 824));
        Assert.AreEqual("Written by Demo · Painted by Demo · 1440×824", demo.Shown);
        Assert.AreEqual("Written by Demo, Painted by Demo, 1440 by 824", demo.Spoken);
        Assert.AreEqual("Written by Demo · ", demo.Head);
        Assert.AreEqual(" · 1440×824", demo.Tail);

        var google = Provenance.Of(Made(ProviderKind.Google, "gemini-3.5-flash-lite", ProviderKind.Google, "gemini-3.1-flash-image", cost: 40_000));
        Assert.AreEqual("Written by Gemini 3.5 Flash-Lite (Google Gemini)", google.Written);
        Assert.AreEqual("Painted by Nano Banana 2 (Google Gemini)", google.Painted);
        Assert.AreEqual("about $0.04", google.Cost);
        Assert.AreEqual("Written by Gemini 3.5 Flash-Lite (Google Gemini) · Painted by Nano Banana 2 (Google Gemini) · 3840×2160 · about $0.04", google.Shown);

        // The provider's own list names a model better than AutoPaper's table; an unknown id stays the id.
        var listed = new Dictionary<string, string> { ["gpt-6-luna"] = "GPT-6 Luna" };
        var openAi = Provenance.Of(Made(ProviderKind.OpenAi, "gpt-6-luna", ProviderKind.OpenAi, "gpt-image-2"), listed);
        Assert.AreEqual("Written by GPT-6 Luna (OpenAI)", openAi.Written);
        Assert.AreEqual("Painted by gpt-image-2 (OpenAI)", openAi.Painted);
        Assert.IsNull(openAi.Cost, "No cost recorded, nothing said.");

        // Local providers cost nothing, even with a number recorded; an empty model is the provider's name.
        var local = Provenance.Of(Made(ProviderKind.Ollama, "", ProviderKind.ComfyUi, "", 0, 0, cost: 5_000));
        Assert.AreEqual("Written by Ollama · Painted by ComfyUI", local.Shown);
        Assert.AreEqual("", local.Tail);

        // A ComfyUI model AutoPaper has a workflow for is named by it.
        var name = BundledWorkflows.All.First();
        StringAssert.Contains(Provenance.Of(Made(ProviderKind.Demo, "", ProviderKind.ComfyUi, name.Key)).Painted, $"Painted by {name.Value} (ComfyUI)");
    }

    // ── Now's reload/stop ───────────────────────────────────────────────────────────────────────

    [TestMethod]
    public void ReloadBecomesStopWhileWorking()
    {
        var make = MakeOrStop.For(ready: true, working: false);
        Assert.IsFalse(make.IsStop);
        Assert.IsTrue(make.IsEnabled);
        Assert.AreEqual("New wallpaper now", make.Label);
        Assert.AreEqual("New wallpaper now (Ctrl+R)", make.Tooltip);
        Assert.AreEqual("Ctrl+R", make.Keys);

        var other = MakeOrStop.For(ready: true, working: false, otherMood: true);
        Assert.AreEqual("Use this mood and make a new wallpaper", other.Label);
        Assert.AreEqual("Use this mood and make a new wallpaper (Ctrl+R)", other.Tooltip);

        var stop = MakeOrStop.For(ready: true, working: true, otherMood: true);
        Assert.IsTrue(stop.IsStop);
        Assert.AreEqual("Stop", stop.Label, "While working it's Stop, in Now and in any mood.");
        Assert.AreEqual("Stop making this wallpaper (Esc)", stop.Tooltip);
        Assert.AreEqual("Escape", stop.Keys);
        Assert.AreNotEqual(make.Glyph, stop.Glyph);

        Assert.IsFalse(MakeOrStop.For(ready: false, working: false).IsEnabled, "Nothing to do until the engine is open.");
    }

    // ── What moods have made, in words ──────────────────────────────────────────────────────────

    private static Keyword Word(string text, KeywordWeight weight) => new("k-" + text, text, weight, 0, 0);

    private static Mood MoodOf(string id, string name, long created, bool active = false, params Keyword[] keywords) =>
        new(id, name, 0, 0.35f, created, keywords, active);

    [TestMethod]
    public void MoodsAreSummedUpInWords()
    {
        Assert.AreEqual("Surprise: Fresh (35%)", MoodText.SurpriseLine(0.35f));
        Assert.AreEqual("Surprise: Faithful (0%)", MoodText.SurpriseLine(-1));

        var now = new DateTimeOffset(2026, 10, 6, 18, 0, 0, TimeSpan.Zero);
        Assert.AreEqual("Nothing made yet", MoodText.MadeLine(0, 0, null, now: now));
        Assert.AreEqual("1 wallpaper · 0 liked", MoodText.MadeLine(1, 0, null, now: now));
        Assert.AreEqual("12 wallpapers, 3 liked, last made 3 hours ago",
            MoodText.MadeLine(12, 3, now.AddHours(-3).ToUnixTimeSeconds(), ", ", now));

        Assert.AreEqual("just now", MoodText.Relative(now.AddSeconds(20).ToUnixTimeSeconds(), now), "Never in the future.");
        Assert.AreEqual("a minute ago", MoodText.Relative(now.AddSeconds(-70).ToUnixTimeSeconds(), now));
        Assert.AreEqual("5 minutes ago", MoodText.Relative(now.AddMinutes(-5).ToUnixTimeSeconds(), now));
        Assert.AreEqual("4 days ago", MoodText.Relative(now.AddDays(-4).ToUnixTimeSeconds(), now));
        Assert.AreEqual("2 weeks ago", MoodText.Relative(now.AddDays(-15).ToUnixTimeSeconds(), now));
        StringAssert.StartsWith(MoodText.Relative(now.AddDays(-90).ToUnixTimeSeconds(), now), "on ");
        Assert.AreEqual("Not yet", MoodText.LastMadeTile(null, now));
        Assert.AreEqual("5 minutes ago", MoodText.LastMadeTile(now.AddMinutes(-5).ToUnixTimeSeconds(), now).ToLowerInvariant());

        Keyword[] keywords = [Word("rain", KeywordWeight.Must), Word("people", KeywordWeight.Avoid), Word("beach", KeywordWeight.Must)];
        CollectionAssert.AreEqual(new[] { "Must: rain, beach", "Avoid: people" }, MoodText.KeywordGroups(keywords).ToArray());
        Assert.AreEqual("2 Must · 0 Maybe · 1 Avoid keywords", MoodText.KeywordBreakdown([MoodOf("a", "A", 1, true, keywords)]));
        Assert.AreEqual("1 Must · 0 Maybe · 0 Avoid keyword", MoodText.KeywordBreakdown([MoodOf("a", "A", 1, true, Word("rain", KeywordWeight.Must))]));
        Assert.AreEqual("No keywords yet", MoodText.KeywordBreakdown([MoodOf("a", "A", 1)]));
        Assert.AreEqual("Latest wallpaper: Fog", MoodText.SpokenLatest(["Fog"]));
        Assert.AreEqual("Latest wallpapers: Fog, Harbour", MoodText.SpokenLatest(["Fog", "Harbour"]));

        var totals = MoodText.Totals(1, [new MoodStats("a", 11, 2, 1, 1, null, [])]);
        CollectionAssert.AreEqual(new[] { "1 mood", "11 wallpapers", "2 liked", "1 echo" }, totals.Select(total => total.Spoken).ToArray());
        CollectionAssert.AreEqual(new long[] { 1, 11, 2, 1 }, totals.Select(total => total.Value).ToArray());
    }

    // ── The summary's chart ─────────────────────────────────────────────────────────────────────

    [TestMethod]
    public void DayBoundsAreLocalMidnightsAcrossDaylightSaving()
    {
        var zone = TimeZoneInfo.FindSystemTimeZoneById("Pacific Standard Time");
        // 2026-11-01 is when the US leaves daylight saving: that day is 25 hours long.
        var bounds = MoodActivity.DayBounds(30, new DateTime(2026, 11, 10, 20, 0, 0, DateTimeKind.Utc), zone);
        Assert.HasCount(31, bounds);
        for (var index = 1; index < bounds.Length; index++)
        {
            var length = bounds[index] - bounds[index - 1];
            Assert.IsTrue(length is 23 * 3600 or 24 * 3600 or 25 * 3600, $"day {index} is {length} s");
        }
        Assert.IsTrue(bounds.Zip(bounds.Skip(1)).Any(pair => pair.Second - pair.First == 25 * 3600), "The 25-hour day is one day.");
        var lastDay = TimeZoneInfo.ConvertTime(DateTimeOffset.FromUnixTimeSeconds(bounds[^2]), zone);
        Assert.AreEqual(new DateTime(2026, 11, 10), lastDay.Date, "Today is the last day.");
        Assert.AreEqual(TimeSpan.Zero, lastDay.TimeOfDay, "It starts at local midnight.");
    }

    [TestMethod]
    public void EachMoodKeepsItsColourAndExtrasFoldIntoOther()
    {
        var moods = Enumerable.Range(0, 9).Select(index => MoodOf($"m{index}", $"Mood {index}", 100 - index)).ToList();
        var slots = MoodActivity.ColorSlots(moods);
        Assert.HasCount(7, slots, "Nine moods: the seven oldest get colours, the rest are Other.");
        Assert.AreEqual(0, slots["m8"], "The oldest gets the first colour, whatever the list order.");
        Assert.IsFalse(slots.ContainsKey("m0"));
        Assert.HasCount(2, MoodActivity.ColorSlots(moods.Take(2)));
    }

    [TestMethod]
    public void DaysStackInTheMoodsOrderWithOtherOnTop()
    {
        var slots = new Dictionary<string, int> { ["a"] = 0, ["b"] = 1 };
        var segments = MoodActivity.Segments(
            [new DayCount(200, "b", 2), new DayCount(200, null, 1), new DayCount(200, "gone", 4), new DayCount(200, "a", 3), new DayCount(100, "a", 1)],
            ["a", "b"], slots);
        var day = segments.Where(segment => segment.DayStart == 200).ToList();
        CollectionAssert.AreEqual(new string?[] { "a", "b", null }, day.Select(segment => segment.MoodId).ToArray());
        CollectionAssert.AreEqual(new[] { 0, 3, 5 }, day.Select(segment => segment.Base).ToArray());
        Assert.AreEqual(5, day[2].Count, "A deleted mood and none fold into Other moods.");
        Assert.IsTrue(day[2].IsTop);
        Assert.IsFalse(day[0].IsTop);
        Assert.AreEqual(100, segments[0].DayStart, "Days in order.");
        Assert.AreEqual("A 3, B 2, Other moods 5",
            MoodActivity.DayLine(day, id => id switch { "a" => "A", "b" => "B", _ => "Other moods" }));
    }

    [TestMethod]
    public void TheScaleHasAFewRoundTicks()
    {
        CollectionAssert.AreEqual(new[] { 0, 1 }, MoodActivity.Ticks(0).ToArray());
        CollectionAssert.AreEqual(new[] { 0, 1, 2, 3 }, MoodActivity.Ticks(3).ToArray());
        CollectionAssert.AreEqual(new[] { 0, 5, 10, 15 }, MoodActivity.Ticks(14).ToArray());
        CollectionAssert.AreEqual(new[] { 0, 25, 50, 75, 100 }, MoodActivity.Ticks(97).ToArray());
        var bounds = Enumerable.Range(0, 31).Select(index => (long)index).ToList();
        CollectionAssert.AreEqual(new long[] { 3, 10, 17, 24 }, MoodActivity.WeekLabels(bounds).ToArray());
        Assert.AreEqual("14 wallpapers in the last 30 days. Kept on this PC only.", MoodActivity.Footnote(14));
        Assert.AreEqual("1 wallpaper in the last 30 days. Kept on this PC only.", MoodActivity.Footnote(1));
    }
}
