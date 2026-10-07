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
