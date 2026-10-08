using System.Collections.Concurrent;
using AutoPaper.Core;

namespace AutoPaper.Tests;

/// <summary>
/// What the Moods view, the notification-area menu, History's mood filter and the progress ring rely on, through the
/// C# bindings with the Demo providers: moods are made, renamed, duplicated and deleted with typed mistakes; switching
/// makes nothing and Settings' Surprise follows the mood in use; wallpapers are filed under their mood; and progress
/// detail arrives in order with a fraction that never goes back (Demo's slow mode, AUTOPAPER_DEMO_DELAY_SECS).
/// </summary>
[TestClass]
public sealed class MoodsContractTests
{
    private readonly List<string> dataDirs = [];
    private readonly List<Engine> engines = [];

    private Engine Open(int? demoDelaySeconds = null)
    {
        var dataDir = Path.Combine(Path.GetTempPath(), "autopaper-tests-" + Guid.NewGuid().ToString("N")[..8]);
        dataDirs.Add(dataDir);
        // Read once, when the engine opens.
        Environment.SetEnvironmentVariable("AUTOPAPER_DEMO_DELAY_SECS", demoDelaySeconds?.ToString(System.Globalization.CultureInfo.InvariantCulture));
        try
        {
            var engine = Engine.Open(new EngineConfig(dataDir, Path.Combine(dataDir, "no-model"), "en-US", "Windows/tests AutoPaper/0"), new NoSecrets());
            engines.Add(engine);
            return engine;
        }
        finally
        {
            Environment.SetEnvironmentVariable("AUTOPAPER_DEMO_DELAY_SECS", null);
        }
    }

    [TestCleanup]
    public void Cleanup()
    {
        foreach (var engine in engines)
        {
            engine.Dispose();
        }
        foreach (var dataDir in dataDirs)
        {
            try
            {
                Directory.Delete(dataDir, recursive: true);
            }
            catch (IOException)
            {
                // The database may still be closing.
            }
        }
    }

    [TestMethod]
    public void AFreshEngineHasOneMoodInUse()
    {
        var engine = Open();
        var moods = engine.Moods();
        Assert.HasCount(1, moods);
        Assert.IsTrue(moods[0].Active);
        Assert.AreEqual(moods[0].Id, engine.ActiveMood().Id);
    }

    [TestMethod]
    public void MoodsAreMadeRenamedAndDeletedWithTypedMistakes()
    {
        var engine = Open();
        var first = engine.ActiveMood();
        var beach = engine.CreateMood("Rainy beach", null);
        Assert.IsFalse(beach.Active, "a new mood isn't made current");
        Assert.IsEmpty(beach.Keywords);
        Assert.AreEqual(0.35f, beach.Surprise, 0.001f);

        Assert.AreEqual(InvalidInputReason.MoodNameEmpty,
            Assert.ThrowsExactly<AutoPaperException.InvalidInput>(() => engine.CreateMood("   ", null)).reason);
        Assert.AreEqual(InvalidInputReason.MoodNameTooLong,
            Assert.ThrowsExactly<AutoPaperException.InvalidInput>(() => engine.CreateMood(new string('a', 41), null)).reason);
        Assert.AreEqual(InvalidInputReason.DuplicateMoodName,
            Assert.ThrowsExactly<AutoPaperException.InvalidInput>(() => engine.CreateMood("rainy  BEACH", null)).reason);

        Assert.AreEqual("Rainy shore", engine.RenameMood(beach.Id, "  Rainy   shore ").Name);
        Assert.AreEqual("rainy shore", engine.RenameMood(beach.Id, "rainy shore").Name, "changing only the case is fine");

        engine.MoveMood(beach.Id, 0);
        Assert.AreEqual(beach.Id, engine.Moods()[0].Id);

        engine.DeleteMood(first.Id);
        Assert.AreEqual(beach.Id, engine.ActiveMood().Id, "deleting the current mood makes another current");
        Assert.AreEqual(InvalidInputReason.LastMood,
            Assert.ThrowsExactly<AutoPaperException.InvalidInput>(() => engine.DeleteMood(beach.Id)).reason);
        Assert.ThrowsExactly<AutoPaperException.NotFound>(() => engine.SetActiveMood("no-such-mood"));
    }

    [TestMethod]
    public void DuplicateCopiesKeywordsAndSurprise()
    {
        var engine = Open();
        var first = engine.ActiveMood();
        engine.AddMoodKeyword(first.Id, "rain", KeywordWeight.Must);
        engine.AddMoodKeyword(first.Id, "people", KeywordWeight.Avoid);
        engine.SetMoodSurprise(first.Id, 0.8f);
        var copy = engine.CreateMood("Copy", first.Id);
        CollectionAssert.AreEqual(new[] { "rain", "people" }, copy.Keywords.Select(keyword => keyword.Text).ToArray());
        Assert.AreEqual(KeywordWeight.Avoid, copy.Keywords[1].Weight);
        Assert.IsFalse(engine.ActiveMood().Keywords.Select(k => k.Id).Intersect(copy.Keywords.Select(k => k.Id)).Any(), "the copy's keywords have new ids");
        Assert.AreEqual(0.8f, copy.Surprise, 0.001f);
    }

    /// <summary>The Moods detail edits a mood that isn't current; Use switches without making anything; Settings'
    /// Surprise is the current mood's, read and written promptly (AppModel.UpdateSettingsAsync does both in one call).</summary>
    [TestMethod]
    public void SwitchingMakesNothingAndSurpriseFollowsTheMood()
    {
        var engine = Open();
        var first = engine.ActiveMood();
        var beach = engine.CreateMood("Rainy beach", null);
        engine.AddMoodKeyword(beach.Id, "beach", KeywordWeight.Must);
        engine.SetMoodSurprise(beach.Id, 0.7f);
        Assert.IsEmpty(engine.Keywords(), "editing another mood leaves the current one alone");

        engine.SetActiveMood(beach.Id);
        Assert.IsEmpty(engine.History(HistoryFilter.All, 10, 0), "switching never makes a wallpaper");
        Assert.AreEqual(0.7f, engine.Settings().Surprise, 0.001f);
        CollectionAssert.AreEqual(new[] { "beach" }, engine.Keywords().Select(k => k.Text).ToArray());

        engine.UpdateSettings(engine.Settings() with { Surprise = 0.9f });
        Assert.AreEqual(0.9f, engine.Moods().Single(m => m.Id == beach.Id).Surprise, 0.001f);
        Assert.AreEqual(0.35f, engine.Moods().Single(m => m.Id == first.Id).Surprise, 0.001f);
    }

    [TestMethod]
    public async Task WallpapersAreFiledUnderTheirMood()
    {
        var engine = Open();
        var first = engine.ActiveMood();
        engine.AddKeyword("lighthouse", KeywordWeight.Must);
        await engine.Generate(Trigger.Manual, null);
        var beach = engine.CreateMood("Rainy beach", null);
        engine.AddMoodKeyword(beach.Id, "beach", KeywordWeight.Must);
        engine.SetActiveMood(beach.Id);
        var made = await engine.Generate(Trigger.Manual, null);
        Assert.AreEqual(beach.Id, made.MoodId);
        Assert.AreEqual("Rainy beach", made.MoodName);

        Assert.HasCount(1, engine.HistoryByMood(HistoryFilter.All, beach.Id, 10, 0));
        Assert.HasCount(1, engine.HistoryByMood(HistoryFilter.All, first.Id, 10, 0));
        Assert.HasCount(2, engine.HistoryByMood(HistoryFilter.All, null, 10, 0));

        engine.DeleteMood(beach.Id);
        var kept = engine.Generation(made.Id);
        Assert.IsNull(kept.MoodName, "a deleted mood's wallpapers stay, without its name");
        Assert.HasCount(2, engine.HistoryByMood(HistoryFilter.All, null, 10, 0));
    }

    /// <summary>The ring and "about N seconds left": every stage reports without numbers, painting reports a fraction
    /// that never goes back and stays under 1 until Done (1 and 0 seconds). Afterwards Demo has an estimate.</summary>
    [TestMethod]
    public async Task ProgressDetailArrivesInOrder()
    {
        var engine = Open(demoDelaySeconds: 2);
        var details = new ConcurrentQueue<ProgressDetail>();
        engine.SetProgressDetailObserver(new Recorder(details));
        engine.AddKeyword("lighthouse", KeywordWeight.Must);
        var demo = new ProviderSelection(ProviderKind.Demo, "", null);
        Assert.IsNull(engine.Estimate(demo, ProviderJob.Images, 0, 0), "nothing recorded yet, so no estimate");

        await engine.Generate(Trigger.Manual, null);

        var seen = details.ToList();
        Assert.AreEqual(ProgressStage.CheckingServices, seen[0].Stage);
        var done = seen[^1];
        Assert.AreEqual(ProgressStage.Done, done.Stage);
        Assert.AreEqual(1f, done.Fraction);
        Assert.AreEqual(0u, done.SecondsLeft);
        var painting = seen.Where(detail => detail.Stage == ProgressStage.Generating && detail.Fraction is not null).Select(detail => detail.Fraction!.Value).ToList();
        Assert.IsNotEmpty(painting, "Demo's slow mode reports its steps");
        for (var i = 1; i < painting.Count; i++)
        {
            Assert.IsGreaterThanOrEqualTo(painting[i - 1], painting[i], "the fraction never goes back");
        }
        Assert.IsLessThanOrEqualTo(0.99f, painting.Max());
        foreach (var stage in seen.Where(detail => detail.Stage is ProgressStage.CheckingServices or ProgressStage.Composing or ProgressStage.CheckingMemory))
        {
            Assert.IsNull(stage.Fraction);
        }
        Assert.IsNotNull(engine.Estimate(demo, ProviderJob.Images, 0, 0), "one painting recorded makes an estimate");

        engine.SetProgressDetailObserver(null);
        var before = details.Count;
        await engine.Generate(Trigger.Manual, null);
        Assert.HasCount(before, details, "a removed observer hears nothing");
    }

    /// <summary>The app's timer wakes at next_start: never after the due time it's starting early for.</summary>
    [TestMethod]
    public async Task NextStartIsNoLaterThanNextDue()
    {
        var engine = Open();
        engine.AddKeyword("lighthouse", KeywordWeight.Must);
        await engine.Generate(Trigger.Manual, null);
        var due = engine.NextDue();
        var start = engine.NextStart();
        Assert.IsNotNull(due);
        Assert.IsNotNull(start);
        Assert.IsLessThanOrEqualTo(due.Value, start.Value);
        engine.UpdateSettings(engine.Settings() with { Paused = true });
        Assert.IsNull(engine.NextStart());
    }

    private sealed class Recorder(ConcurrentQueue<ProgressDetail> details) : ProgressDetailObserver
    {
        public void OnProgressDetail(ProgressDetail detail) => details.Enqueue(detail);
    }

    private sealed class NoSecrets : SecretStore
    {
        public string? Get(string account) => null;

        public void Set(string account, string value)
        {
        }

        public void Delete(string account)
        {
        }
    }
}
