using AutoPaper.Core;
using AutoPaper.Services;

namespace AutoPaper.Tests;

/// <summary>
/// The app's words for the engine's values, read from the app's own Resources.resw: every error has a plain sentence
/// (no resource key left showing, no core English), a problem the person can fix is a link to it (docs/app-spec.md 6a:
/// "Add your OpenAI key", a painting ComfyUI couldn't make is one line and a link to Providers with no "try again
/// later"), and progress and estimates read as a person says them.
/// </summary>
[TestClass]
public sealed class WordingTests
{
    private static readonly Settings Defaults = new(
        0.35f, Cadence.Daily, false, QuietPeriod.SixMonths, EchoFrequency.Sometimes,
        new ProviderSelection(ProviderKind.Demo, "", null), new ProviderSelection(ProviderKind.ComfyUi, "", null),
        ImageQuality.High, 500, Fallback.RevisitLiked, true, true, true, 2048, null);

    [TestInitialize]
    public void UseAppStrings() => ReswStrings.Use();

    /// <summary>Every variant (and every typed reason) is worded, scheduled or not, without a resource key or the
    /// core's English detail showing.</summary>
    [TestMethod]
    public void EveryErrorHasASentence()
    {
        var errors = new List<Exception>
        {
            new AutoPaperException.MissingKey(ProviderKind.OpenAi),
            new AutoPaperException.MissingKey(ProviderKind.OpenAiCompatible),
            new AutoPaperException.InvalidKey(ProviderKind.Google),
            new AutoPaperException.RateLimited(ProviderKind.OpenAi, 30),
            new AutoPaperException.RateLimited(ProviderKind.OpenAi, 600),
            new AutoPaperException.Refused(ProviderKind.OpenAi),
            new AutoPaperException.Unsupported(ProviderKind.Ollama, "paint images"),
            new AutoPaperException.Unsupported(ProviderKind.ComfyUi, "write ideas"),
            new AutoPaperException.BudgetReached(500),
            new AutoPaperException.Offline(),
            new AutoPaperException.InvalidResponse("CORE-DETAIL"),
            new AutoPaperException.PaintingFailed(ProviderKind.ComfyUi, "Qwen-Image 2.1", "CORE-DETAIL"),
            new AutoPaperException.PaintingFailed(ProviderKind.ComfyUi, "", "CORE-DETAIL"),
            new AutoPaperException.KeywordNotFollowed("lighthouse", KeywordWeight.Must, "mood-1"),
            new AutoPaperException.KeywordNotFollowed("people", KeywordWeight.Avoid, "mood-1"),
            new AutoPaperException.KeywordNotFollowed("fog", KeywordWeight.Maybe, "mood-1"),
            new AutoPaperException.NotFound(),
            new AutoPaperException.NothingToRevisit(),
            new AutoPaperException.Storage("CORE-DETAIL"),
            new AutoPaperException.Cancelled(),
            new AutoPaperException.Internal("CORE-DETAIL"),
            new InvalidOperationException("CORE-DETAIL"),
        };
        errors.AddRange(Enum.GetValues<ProviderUnavailableReason>()
            .SelectMany(reason => new[] { ProviderKind.Ollama, ProviderKind.OpenAiCompatible, ProviderKind.OpenAi }
                .Select(kind => new AutoPaperException.ProviderUnavailable(kind, reason, "CORE-DETAIL"))));
        errors.AddRange(Enum.GetValues<InvalidInputReason>().Select(reason => new AutoPaperException.InvalidInput(reason, "CORE-DETAIL")));
        foreach (var error in errors)
        {
            foreach (var scheduled in new[] { false, true })
            {
                var sentence = Text.Error(error, Defaults, scheduled);
                AssertWorded(sentence, error);
                var problem = Text.Problem(error, Defaults, scheduled);
                AssertWorded(problem.Spoken, error);
                if (problem.Link is not null)
                {
                    AssertWorded(problem.Link, error);
                    Assert.AreNotEqual(Fix.None, problem.Fix, error.ToString());
                }
            }
        }
    }

    private static void AssertWorded(string text, Exception error)
    {
        Assert.IsFalse(string.IsNullOrWhiteSpace(text), error.ToString());
        Assert.DoesNotContain("CORE-DETAIL", text, $"The core's detail shows in “{text}”");
        StringAssert.DoesNotMatch(text, new System.Text.RegularExpressions.Regex(@"\b[A-Z][a-z]+_[A-Za-z]+\b"), $"A resource key shows in “{text}”");
        // {0}-style placeholders left unfilled ({{prompt}} in a workflow sentence is the person's own text).
        StringAssert.DoesNotMatch(text, new System.Text.RegularExpressions.Regex(@"(?<!\{)\{\d+\}"), $"A placeholder is left in “{text}”");
    }

    [TestMethod]
    public void AMissingKeyIsALinkToItsField()
    {
        var problem = Text.Problem(new AutoPaperException.MissingKey(ProviderKind.OpenAi), Defaults, scheduled: true);
        Assert.AreEqual("", problem.Sentence);
        Assert.AreEqual("Add your OpenAI key", problem.Link);
        Assert.AreEqual(Fix.Keys, problem.Fix);
        Assert.AreEqual(AutopaperCoreMethods.SecretAccountFor(new ProviderSelection(ProviderKind.OpenAi, "", null)), problem.KeyAccount);
        Assert.AreEqual("Add your OpenAI key", problem.Spoken);
    }

    [TestMethod]
    public void ARefusedKeyIsCheckYourKey()
    {
        var problem = Text.Problem(new AutoPaperException.InvalidKey(ProviderKind.Google), Defaults);
        Assert.AreEqual("Check your Google Gemini key", problem.Link);
        Assert.AreEqual(Fix.Keys, problem.Fix);
        Assert.AreEqual("google.api_key", problem.KeyAccount);
    }

    [TestMethod]
    public void BudgetBlocksAlwaysKeepTheCurrentWallpaperAndLinkToBudget()
    {
        foreach (var fallback in new[] { Fallback.RevisitLiked, Fallback.KeepCurrent })
        {
            foreach (var scheduled in new[] { false, true })
            {
                var settings = Defaults with { Fallback = fallback };
                var error = new AutoPaperException.BudgetReached(500);
                var problem = Text.Problem(error, settings, scheduled);
                Assert.AreEqual(Fix.Budget, problem.Fix);
                Assert.AreEqual("Raise the budget", problem.Link);
                StringAssert.Contains(problem.Sentence, "estimated next wallpaper");
                StringAssert.Contains(Text.Error(error, settings, scheduled), "keeping the current wallpaper");
                Assert.DoesNotContain("bringing back", Text.Error(error, settings, scheduled));
            }
        }
    }

    [TestMethod]
    public void ServicePreflightNamesBothChecksAndDescribesASavedWallpaper()
    {
        Assert.AreEqual("Checking services…", Text.Stage(ProgressStage.CheckingServices));
        var sentence = Text.Revisit(RevisitReason.ServicesUnavailable, Defaults);
        StringAssert.Contains(sentence, "writing or painting service");
        StringAssert.Contains(sentence, "latest saved wallpaper from this mood");
        StringAssert.Contains(sentence, "Console");
        Assert.DoesNotContain("one you liked", sentence);
        Assert.DoesNotContain("New wallpaper", sentence);
    }

    [TestMethod]
    public void ConsoleNamesEveryRunOutcomeAndTriggerAndKeepsExactModelIds()
    {
        foreach (var status in Enum.GetValues<RunStatus>())
        {
            Assert.DoesNotContain("ConsoleStatus_", Text.RunOutcome(status));
            Assert.IsFalse(string.IsNullOrWhiteSpace(Text.RunOutcome(status)));
        }
        Assert.AreEqual("Blocked", Text.RunOutcome(RunStatus.Blocked));
        Assert.AreEqual("Completed", Text.RunOutcome(RunStatus.Succeeded));
        foreach (var trigger in Enum.GetValues<Trigger>())
        {
            Assert.DoesNotContain("ConsoleTrigger_", Text.RunTrigger(trigger));
        }
        Assert.AreEqual("Google Gemini · gemini-3.1-flash-image", Text.RunProvider(ProviderKind.Google, "gemini-3.1-flash-image"));
        Assert.AreEqual("Demo", Text.RunProvider(ProviderKind.Demo, ""));
    }

    [TestMethod]
    public void ConsolePreservesRecordedCostPrecision()
    {
        Assert.AreEqual("$0.003564", Text.RunMoney(3_564));
        Assert.AreEqual("$0.000001", Text.RunMoney(1));
        Assert.AreEqual("$0.00", Text.RunMoney(0));
        Assert.AreEqual("$1.20", Text.RunMoney(1_200_000));
        Assert.AreEqual("$10.0125", Text.RunMoney(10_012_500));
    }

    [TestMethod]
    public void ConsoleStatisticsKeepMissingValuesAndPreciseMeasuredTimes()
    {
        Assert.AreEqual("—", Text.RunRate(null));
        Assert.AreEqual("0%", Text.RunRate(0));
        Assert.AreEqual("33%", Text.RunRate(1d / 3));
        Assert.AreEqual("100%", Text.RunRate(1));
        Assert.AreEqual("—", Text.RunSeconds(null));
        Assert.AreEqual("0 s", Text.RunSeconds(0));
        Assert.AreEqual("2.124 s", Text.RunSeconds(2.124));
        Assert.AreEqual("0.000001 s", Text.RunSeconds(0.000001));
    }

    /// <summary>An OpenAI-compatible server's key is the one for the address in Settings (secret_account_for).</summary>
    [TestMethod]
    public void AServersKeyLinksToThatServersField()
    {
        var compatible = new ProviderSelection(ProviderKind.OpenAiCompatible, "", "http://127.0.0.1:1234");
        var settings = Defaults with { TextProvider = compatible };
        var problem = Text.Problem(new AutoPaperException.InvalidKey(ProviderKind.OpenAiCompatible), settings);
        Assert.AreEqual("Check your server's key", problem.Link);
        Assert.AreEqual(AutopaperCoreMethods.SecretAccountFor(compatible), problem.KeyAccount);
        Assert.IsNotNull(problem.KeyAccount);
    }

    /// <summary>A painting ComfyUI couldn't make: one line naming the model, a link to Providers, and no "try again
    /// later" even on the schedule (the core fills the slot; the timer can't fix it).</summary>
    [TestMethod]
    public void APaintingThatFailedIsOneLineAndALink()
    {
        foreach (var scheduled in new[] { false, true })
        {
            var problem = Text.Problem(new AutoPaperException.PaintingFailed(ProviderKind.ComfyUi, "Qwen-Image 2.1", "the KSampler node (8) failed"), Defaults, scheduled);
            Assert.AreEqual("ComfyUI couldn't paint with Qwen-Image 2.1.", problem.Sentence);
            Assert.AreEqual("Check ComfyUI settings", problem.Link);
            Assert.AreEqual(Fix.Providers, problem.Fix);
            Assert.DoesNotContain("try again", problem.Spoken, StringComparison.OrdinalIgnoreCase, problem.Spoken);
        }
        Assert.AreEqual("ComfyUI couldn't paint this wallpaper.",
            Text.Problem(new AutoPaperException.PaintingFailed(ProviderKind.ComfyUi, " ", "x"), Defaults).Sentence);
    }

    /// <summary>In Settings › Providers the person is already where it's fixed: the sentence alone.</summary>
    [TestMethod]
    public void InProvidersTheresNoLinkToProviders()
    {
        var problem = Text.Problem(new AutoPaperException.PaintingFailed(ProviderKind.ComfyUi, "Z-Image Turbo", "x"), Defaults, inSettings: true);
        Assert.IsNull(problem.Link);
        Assert.AreEqual(Fix.None, problem.Fix);
        Assert.AreEqual("ComfyUI couldn't paint with Z-Image Turbo.", problem.Spoken);
        // A key is still a link, even in Settings (to Keys).
        Assert.AreEqual(Fix.Keys, Text.Problem(new AutoPaperException.MissingKey(ProviderKind.OpenAi), Defaults, inSettings: true).Fix);
    }

    /// <summary>Linked sentences don't give directions ("in Settings › …"): the link goes there.</summary>
    [TestMethod]
    public void LinkedSentencesGiveNoDirections()
    {
        Exception[] errors =
        [
            new AutoPaperException.Unsupported(ProviderKind.Ollama, "paint images"),
            new AutoPaperException.BudgetReached(500),
            new AutoPaperException.ProviderUnavailable(ProviderKind.Ollama, ProviderUnavailableReason.NotRunning, "x"),
            new AutoPaperException.InvalidInput(InvalidInputReason.AddressInvalid, "x"),
            new AutoPaperException.InvalidInput(InvalidInputReason.KeywordTooLong, "x"),
        ];
        foreach (var error in errors)
        {
            var problem = Text.Problem(error, Defaults);
            Assert.IsNotNull(problem.Link, error.ToString());
            Assert.DoesNotContain("Settings", problem.Sentence, problem.Sentence);
        }
        Assert.AreEqual("Choose another painter", Text.Problem(new AutoPaperException.Unsupported(ProviderKind.Ollama, "paint images"), Defaults).Link);
        Assert.AreEqual("Raise the budget", Text.Problem(new AutoPaperException.BudgetReached(500), Defaults).Link);
        Assert.AreEqual(Fix.Moods, Text.Problem(new AutoPaperException.InvalidInput(InvalidInputReason.DuplicateKeyword, "x"), Defaults).Fix);
        Assert.AreEqual("Ollama isn't running at http://127.0.0.1:11434.",
            Text.Problem(new AutoPaperException.ProviderUnavailable(ProviderKind.Ollama, ProviderUnavailableReason.NotRunning, "x"), Defaults).Sentence);
    }

    [TestMethod]
    public void MoodMistakesAreWorded()
    {
        Assert.AreEqual("A mood needs a name.", Text.InvalidInput(InvalidInputReason.MoodNameEmpty));
        Assert.AreEqual("Mood names can be up to 40 characters.", Text.InvalidInput(InvalidInputReason.MoodNameTooLong));
        Assert.AreEqual("Another mood already has this name.", Text.InvalidInput(InvalidInputReason.DuplicateMoodName));
        Assert.AreEqual("There's always at least one mood, so the last one can't be deleted.", Text.InvalidInput(InvalidInputReason.LastMood));
    }

    [TestMethod]
    public void TimeIsRoundedAsAPersonSaysIt()
    {
        (uint Seconds, string Said)[] cases =
        [
            (1, "A few seconds left"), (9, "A few seconds left"), (10, "About 10 seconds left"), (12, "About 10 seconds left"),
            (13, "About 15 seconds left"), (57, "About 55 seconds left"), (58, "About a minute left"), (89, "About a minute left"),
            (90, "About 2 minutes left"), (125, "About 2 minutes left"), (540, "About 9 minutes left"),
            (3569, "About 59 minutes left"), (3600, "About an hour left"), (4200, "About 1 hour 10 minutes left"),
            (7200, "About 2 hours left"), (7500, "About 2 hours 5 minutes left"),
        ];
        foreach (var (seconds, said) in cases)
        {
            Assert.AreEqual(said, Text.TimeLeft(seconds), $"{seconds} s");
        }
        Assert.AreEqual("Almost done", Text.TimeLeft(0));
        Assert.AreEqual("", Text.TimeLeft(null));
    }

    [TestMethod]
    public void TheStageCarriesItsTimeLeft()
    {
        Assert.AreEqual("Painting… about 5 minutes left", Text.StageWithTimeLeft("Painting…", 300));
        Assert.AreEqual("Painting… almost done", Text.StageWithTimeLeft("Painting…", 0));
        Assert.AreEqual("Painting…", Text.StageWithTimeLeft("Painting…", null));
        Assert.AreEqual("", Text.StageWithTimeLeft("", 300));
    }

    /// <summary>Settings › Providers says how long a model takes here, and nothing when nothing is recorded.</summary>
    [TestMethod]
    public void EstimatesAreSaidPerWallpaperAndPerIdea()
    {
        Assert.AreEqual("About 9 minutes per wallpaper on this PC.", Text.Estimate(ProviderJob.Images, 540));
        Assert.AreEqual("About 20 seconds per idea on this PC.", Text.Estimate(ProviderJob.Concepts, 20));
        Assert.AreEqual("A few seconds per wallpaper on this PC.", Text.Estimate(ProviderJob.Images, 3));
        Assert.AreEqual("", Text.Estimate(ProviderJob.Images, null));
    }

    /// <summary>The model picker's blank choice names the default the way the provider lists it; ComfyUI's also
    /// while it can't be asked (named by AutoPaper's own workflow for it, not by its file).</summary>
    [TestMethod]
    public void TheDefaultModelIsNamedAsListed()
    {
        var file = AutopaperCoreMethods.DefaultModel(ProviderKind.ComfyUi, ProviderJob.Images);
        Assert.AreEqual("Default (Z-Image Turbo, bf16)", Text.DefaultModelChoice(ProviderKind.ComfyUi, ProviderJob.Images, [new ModelInfo(file, "Z-Image Turbo, bf16")]));
        Assert.AreEqual("Default (Z-Image Turbo)", Text.DefaultModelChoice(ProviderKind.ComfyUi, ProviderJob.Images, []));
        Assert.AreEqual("Default (the server's first model)", Text.DefaultModelChoice(ProviderKind.Ollama, ProviderJob.Concepts, []));
        Assert.AreEqual($"Default ({AutopaperCoreMethods.DefaultModel(ProviderKind.OpenAi, ProviderJob.Concepts)})",
            Text.DefaultModelChoice(ProviderKind.OpenAi, ProviderJob.Concepts, []));
    }

    /// <summary>Every string the app words things with in code is in the .resw, and every {n} it formats is there.</summary>
    [TestMethod]
    public void EveryStringHasBalancedPlaceholders()
    {
        foreach (var (key, value) in ReswStrings.All)
        {
            var open = value.Count(c => c == '{');
            var close = value.Count(c => c == '}');
            Assert.AreEqual(open, close, $"{key}: “{value}”");
        }
    }
}
