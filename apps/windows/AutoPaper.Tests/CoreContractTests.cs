using System.Globalization;
using AutoPaper.Core;

namespace AutoPaper.Tests;

/// <summary>What the app takes from the core instead of copying it (prices_as_of, default_base_url, default_model,
/// surprise_band), the typed reasons it words errors by, and has_echoes, which decides whether History offers
/// "Show original and echoes". All offline, with the Demo providers (the defaults).</summary>
[TestClass]
public sealed class CoreContractTests
{
    private readonly string dataDir = Path.Combine(Path.GetTempPath(), "autopaper-tests-" + Guid.NewGuid().ToString("N")[..8]);
    private readonly Engine engine;

    public CoreContractTests()
    {
        engine = Engine.Open(new EngineConfig(dataDir, Path.Combine(dataDir, "no-model"), "en-US", "Windows/tests AutoPaper/0"), new NoSecrets());
    }

    [TestCleanup]
    public void Cleanup()
    {
        engine.Dispose();
        try
        {
            Directory.Delete(dataDir, recursive: true);
        }
        catch (IOException)
        {
            // The database may still be closing.
        }
    }

    /// <summary>Settings › Budget reads it as a date ("Prices as of October 5, 2026").</summary>
    [TestMethod]
    public void PricesAsOfIsAnIsoDate()
    {
        var asOf = AutopaperCoreMethods.PricesAsOf();
        Assert.IsTrue(DateTime.TryParseExact(asOf, "yyyy-MM-dd", CultureInfo.InvariantCulture, DateTimeStyles.None, out _), asOf);
    }

    /// <summary>The address field's placeholder: local kinds have one; OpenAI-compatible has none, so the app shows
    /// its own example there.</summary>
    [TestMethod]
    public void LocalKindsHaveADefaultAddress()
    {
        foreach (var kind in new[] { ProviderKind.Ollama, ProviderKind.ComfyUi })
        {
            var address = AutopaperCoreMethods.DefaultBaseUrl(kind);
            Assert.IsNotNull(address, kind.ToString());
            Assert.IsTrue(Uri.TryCreate(address, UriKind.Absolute, out var uri) && uri.IsLoopback, address);
        }
        Assert.IsNull(AutopaperCoreMethods.DefaultBaseUrl(ProviderKind.OpenAiCompatible));
        Assert.IsNull(AutopaperCoreMethods.DefaultBaseUrl(ProviderKind.OpenAi));
    }

    /// <summary>The model picker's blank choice: "Default (name)" where the core names one, "the server's first
    /// model" for Ollama and OpenAI-compatible servers.</summary>
    [TestMethod]
    public void DefaultModelsAreNamedWhereTheyAreFixed()
    {
        foreach (var job in new[] { ProviderJob.Concepts, ProviderJob.Images })
        {
            Assert.AreNotEqual("", AutopaperCoreMethods.DefaultModel(ProviderKind.OpenAi, job));
            Assert.AreNotEqual("", AutopaperCoreMethods.DefaultModel(ProviderKind.Google, job));
            Assert.AreEqual("", AutopaperCoreMethods.DefaultModel(ProviderKind.OpenAiCompatible, job));
        }
        Assert.AreNotEqual("", AutopaperCoreMethods.DefaultModel(ProviderKind.ComfyUi, ProviderJob.Images));
        Assert.AreEqual("", AutopaperCoreMethods.DefaultModel(ProviderKind.Ollama, ProviderJob.Concepts));
        Assert.AreEqual("", AutopaperCoreMethods.DefaultModel(ProviderKind.Ollama, ProviderJob.Images), "Ollama doesn't paint");
        Assert.AreEqual("", AutopaperCoreMethods.DefaultModel(ProviderKind.ComfyUi, ProviderJob.Concepts), "ComfyUI doesn't write");
    }

    /// <summary>The Surprise slider shows whole percents; each band's words come from the core's band for percent / 100.</summary>
    [TestMethod]
    public void SurpriseBandsByPercent()
    {
        (int Percent, SurpriseBand Band)[] cases =
        [
            (0, SurpriseBand.Faithful), (24, SurpriseBand.Faithful), (25, SurpriseBand.Fresh), (35, SurpriseBand.Fresh),
            (49, SurpriseBand.Fresh), (50, SurpriseBand.Adventurous), (62, SurpriseBand.Adventurous),
            (74, SurpriseBand.Adventurous), (75, SurpriseBand.Wild), (100, SurpriseBand.Wild),
        ];
        foreach (var (percent, band) in cases)
        {
            Assert.AreEqual(band, AutopaperCoreMethods.SurpriseBand(percent / 100f), $"{percent}%");
        }
    }

    /// <summary>has_echoes: false for a lone wallpaper, true for an original and its echo, false again once the echo
    /// is deleted, and NotFound for an id that isn't in History.</summary>
    [TestMethod]
    public async Task HasEchoesFollowsTheLineage()
    {
        engine.AddKeyword("lighthouse", KeywordWeight.Must);
        var original = await engine.Generate(Trigger.Manual, null);
        Assert.IsFalse(engine.HasEchoes(original.Id));

        var echo = await engine.MakeEcho(original.Id, null);
        Assert.AreEqual(original.Id, echo.EchoOf);
        Assert.IsTrue(engine.HasEchoes(original.Id));
        Assert.IsTrue(engine.HasEchoes(echo.Id));
        Assert.HasCount(2, engine.Lineage(echo.Id));

        engine.DeleteGeneration(echo.Id);
        Assert.IsFalse(engine.HasEchoes(original.Id));
        Assert.ThrowsExactly<AutoPaperException.NotFound>(() => engine.HasEchoes(echo.Id));
    }

    /// <summary>Keyword mistakes carry the reason the Keywords view words them by.</summary>
    [TestMethod]
    public void KeywordMistakesHaveTypedReasons()
    {
        Assert.AreEqual(InvalidInputReason.KeywordEmpty,
            Assert.ThrowsExactly<AutoPaperException.InvalidInput>(() => engine.AddKeyword("  !  ", KeywordWeight.Must)).reason);
        Assert.AreEqual(InvalidInputReason.KeywordTooLong,
            Assert.ThrowsExactly<AutoPaperException.InvalidInput>(() => engine.AddKeyword(new string('a', 41), KeywordWeight.Must)).reason);
        engine.AddKeyword("rain", KeywordWeight.Must);
        var ruins = engine.AddKeyword("ruins", KeywordWeight.Maybe);
        Assert.AreEqual(InvalidInputReason.DuplicateKeyword,
            Assert.ThrowsExactly<AutoPaperException.InvalidInput>(() => engine.RenameKeyword(ruins.Id, "Rain")).reason);
    }

    /// <summary>A server address is worded by why it can't be used (Settings › Providers shows it under the field).</summary>
    [TestMethod]
    public void AddressMistakesHaveTypedReasons()
    {
        var settings = engine.Settings();
        Assert.AreEqual(InvalidInputReason.AddressNotAllowed, Assert.ThrowsExactly<AutoPaperException.InvalidInput>(() =>
            engine.UpdateSettings(settings with { TextProvider = new ProviderSelection(ProviderKind.OpenAiCompatible, "", "http://example.com/v1") })).reason);
        Assert.AreEqual(InvalidInputReason.AddressInvalid, Assert.ThrowsExactly<AutoPaperException.InvalidInput>(() =>
            engine.UpdateSettings(settings with { TextProvider = new ProviderSelection(ProviderKind.OpenAiCompatible, "", "my server") })).reason);
        Assert.AreEqual(InvalidInputReason.DisplaySizeInvalid,
            Assert.ThrowsExactly<AutoPaperException.InvalidInput>(() => engine.SetDisplayHint(0, 839)).reason);
    }

    /// <summary>Make an echo of one that doesn't exist: NotFound, which History says as "isn't in History any more".</summary>
    [TestMethod]
    public async Task EchoOfAMissingWallpaperIsNotFound()
    {
        await Assert.ThrowsExactlyAsync<AutoPaperException.NotFound>(() => engine.MakeEcho("missing", null));
    }

    /// <summary>The Console list is a small summary; selecting it retrieves the exact persisted provider trace.
    /// Clearing Console leaves wallpapers and the budget ledger intact.</summary>
    [TestMethod]
    public async Task ConsoleSummariesOpenTheirPersistedTraceAndClearIndependently()
    {
        engine.AddKeyword("lighthouse", KeywordWeight.Must);
        var made = await engine.Generate(Trigger.Manual, null);
        var summaries = engine.Runs(100, 0);
        Assert.HasCount(1, summaries);
        Assert.AreEqual(RunStatus.Succeeded, summaries[0].Status);
        Assert.AreEqual(made.Id, summaries[0].GenerationId);
        Assert.IsEmpty(summaries[0].Events, "Lists don't transfer every prompt and response body.");
        var run = engine.Run(summaries[0].Id);
        Assert.IsTrue(run.Events.Any(entry => entry.Kind == "instructions"));
        Assert.IsTrue(run.Events.Any(entry => entry.Kind == "candidates"));
        Assert.IsTrue(run.Events.Any(entry => entry.Kind == "parameters"));
        Assert.AreEqual("lighthouse", run.Keywords[0].Text);
        var report = engine.RunReport(run.Id);
        using var json = System.Text.Json.JsonDocument.Parse(report);
        Assert.AreEqual(run.Id, json.RootElement.GetProperty("id").GetString());
        Assert.IsGreaterThan(0, json.RootElement.GetProperty("events").GetArrayLength());
        var spending = engine.SpendSummary().SpentMicrousd;
        engine.ClearRuns();
        Assert.IsEmpty(engine.Runs(100, 0));
        Assert.HasCount(1, engine.History(HistoryFilter.All, 100, 0));
        Assert.AreEqual(spending, engine.SpendSummary().SpentMicrousd);
    }

    /// <summary>The prospective budget gate is visible before a provider call, and never replaces the most recent
    /// wallpaper with the first liked wallpaper, even when that fallback is selected.</summary>
    [TestMethod]
    public async Task BudgetBlockRecordsWhyAndKeepsTheLastWallpaper()
    {
        engine.AddKeyword("lighthouse", KeywordWeight.Must);
        var first = await engine.Generate(Trigger.Manual, null);
        engine.Rate(first.Id, Rating.Liked);
        var last = await engine.MakeEcho(first.Id, null);
        engine.MarkShown(last.Id);
        engine.UpdateSettings(engine.Settings() with
        {
            MonthlyBudgetCents = 1,
            Fallback = Fallback.RevisitLiked,
            ImageProvider = new ProviderSelection(ProviderKind.OpenAi, "gpt-image-2", null),
        });
        var budget = engine.BudgetStatus();
        Assert.IsTrue(budget.Blocked);
        Assert.IsGreaterThan(10_000ul, budget.NextCostMicrousd);
        Assert.IsFalse(string.IsNullOrWhiteSpace(budget.Message));
        await Assert.ThrowsExactlyAsync<AutoPaperException.BudgetReached>(() => engine.Generate(Trigger.Manual, null));
        var summary = engine.Runs(1, 0).Single();
        Assert.AreEqual(RunStatus.Blocked, summary.Status);
        var run = engine.Run(summary.Id);
        Assert.IsFalse(run.Events.Any(entry => entry.Kind == "request"), "The budget blocks requests before credentials or providers are used.");
        Assert.AreEqual(last.Id, engine.Current()!.Id);
        Assert.IsFalse(string.IsNullOrWhiteSpace(run.Detail));
    }

    /// <summary>The native overview reads exact retained outcomes and linked provider timings through the FFI.
    /// Budget blocks do not enter the success denominator, and cleared runs no longer appear in its charts.</summary>
    [TestMethod]
    public async Task ConsoleStatisticsDescribeRetainedRunsAndLinkedCalls()
    {
        var empty = engine.ConsoleStatistics();
        Assert.AreEqual(0ul, empty.Total);
        Assert.IsNull(empty.SuccessRate);
        Assert.IsNull(empty.AverageRunSecs);
        Assert.IsEmpty(empty.Models);

        engine.AddKeyword("lighthouse", KeywordWeight.Must);
        var first = await engine.Generate(Trigger.Manual, null);
        await engine.MakeEcho(first.Id, null);
        var finished = engine.ConsoleStatistics();
        Assert.AreEqual(2ul, finished.Total);
        Assert.AreEqual(1.0, finished.SuccessRate);
        Assert.IsNotNull(finished.AverageRunSecs);
        Assert.IsGreaterThanOrEqualTo(0.0, finished.AverageRunSecs!.Value);
        Assert.AreEqual(2ul, finished.Outcomes.Single(outcome => outcome.Status == RunStatus.Succeeded).Count);
        Assert.AreEqual(2ul, finished.Days.Aggregate(0ul, (sum, day) => sum + day.Count));
        Assert.IsTrue(finished.Days.All(day => day.DayStart % 86_400 == 0));
        Assert.HasCount(2, finished.Models);
        foreach (var model in finished.Models)
        {
            Assert.AreEqual(ProviderKind.Demo, model.Provider);
            Assert.IsGreaterThan(0ul, model.Calls);
            Assert.IsTrue(double.IsFinite(model.AverageSecs));
            Assert.IsGreaterThanOrEqualTo(0.0, model.AverageSecs);
        }
        Assert.AreEqual(2ul, finished.Models.Single(model => model.Job == ProviderJob.Images).Calls);

        engine.UpdateSettings(engine.Settings() with { MonthlyBudgetCents = 1,
            ImageProvider = new ProviderSelection(ProviderKind.OpenAi, "gpt-image-2", null) });
        await Assert.ThrowsExactlyAsync<AutoPaperException.BudgetReached>(() => engine.Generate(Trigger.Manual, null));
        var blocked = engine.ConsoleStatistics();
        Assert.AreEqual(3ul, blocked.Total);
        Assert.AreEqual(finished.SuccessRate, blocked.SuccessRate);
        Assert.AreEqual(finished.AverageRunSecs, blocked.AverageRunSecs);
        Assert.AreEqual(1ul, blocked.Outcomes.Single(outcome => outcome.Status == RunStatus.Blocked).Count);
        Assert.AreEqual(finished.Models.Sum(model => (long)model.Calls), blocked.Models.Sum(model => (long)model.Calls));

        engine.ClearRuns();
        var cleared = engine.ConsoleStatistics();
        Assert.AreEqual(0ul, cleared.Total);
        Assert.IsNull(cleared.SuccessRate);
        Assert.IsNull(cleared.AverageRunSecs);
        Assert.IsTrue(cleared.Outcomes.All(outcome => outcome.Count == 0));
        Assert.IsEmpty(cleared.Days);
        Assert.IsEmpty(cleared.Models);
        Assert.HasCount(2, engine.History(HistoryFilter.All, 100, 0));
    }

    /// <summary>The native display flow receives Shown, so newly generated images remain distinct from saved images.</summary>
    [TestMethod]
    public async Task ManualAndEchoShownIdentifyNewImages()
    {
        engine.AddKeyword("lighthouse", KeywordWeight.Must);
        var made = await engine.GenerateOrRevisit(Trigger.Manual, null);
        Assert.IsNull(made.Revisit);
        var echo = await engine.MakeEchoOrRevisit(made.Generation.Id, null);
        Assert.IsNull(echo.Revisit);
        Assert.AreEqual(made.Generation.Id, echo.Generation.EchoOf);
        Assert.HasCount(2, engine.History(HistoryFilter.All, 100, 0));
    }

    /// <summary>A failed preflight returns the latest saved image from the active mood, without requiring a Like or
    /// adding another generated image. This includes an echo request whose source belongs to another mood.</summary>
    [TestMethod]
    public async Task UnavailableServicesReturnLatestSavedImageFromTheActiveMood()
    {
        engine.AddKeyword("lighthouse", KeywordWeight.Must);
        var original = (await engine.GenerateOrRevisit(Trigger.Manual, null)).Generation;
        await engine.MakeEchoOrRevisit(original.Id, null);
        var other = engine.CreateMood("Other mood", null);
        engine.SetActiveMood(other.Id);
        engine.AddKeyword("forest", KeywordWeight.Must);
        var otherImage = (await engine.GenerateOrRevisit(Trigger.Manual, null)).Generation;
        var latest = (await engine.MakeEchoOrRevisit(otherImage.Id, null)).Generation;
        engine.MarkShown(original.Id);
        engine.UpdateSettings(engine.Settings() with
        {
            TextProvider = new ProviderSelection(ProviderKind.Ollama, "offline", "http://127.0.0.1:1"),
            Fallback = Fallback.KeepCurrent,
        });

        var manual = await engine.GenerateOrRevisit(Trigger.Manual, null);
        Assert.AreEqual(RevisitReason.ServicesUnavailable, manual.Revisit);
        Assert.AreEqual(latest.Id, manual.Generation.Id);
        var echo = await engine.MakeEchoOrRevisit(original.Id, null);
        Assert.AreEqual(RevisitReason.ServicesUnavailable, echo.Revisit);
        Assert.AreEqual(latest.Id, echo.Generation.Id);
        Assert.HasCount(4, engine.History(HistoryFilter.All, 100, 0));
        var run = engine.Run(engine.Runs(1, 0).Single().Id);
        var checks = run.Events.Where(entry => entry.Kind == "service_check").ToArray();
        Assert.HasCount(2, checks);
        Assert.IsTrue(checks.All(entry => entry.Stage == "Checking services"));
        Assert.IsTrue(checks.Any(entry => entry.Detail.StartsWith("Writing:", StringComparison.Ordinal)));
        Assert.IsTrue(checks.Any(entry => entry.Detail.StartsWith("Painting:", StringComparison.Ordinal)));
        Assert.IsFalse(run.Events.Any(entry => entry.Kind == "instructions"), "Unavailable services must stop before composing a new scene.");
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
