using System.Collections.Concurrent;
using AutoPaper.Core;
using AutoPaper.Services;

namespace AutoPaper.Tests;

/// <summary>The core reads keys from Windows Credential Manager through the SecretStore callback (UniFFI foreign
/// trait), all offline: a missing key and a key that couldn't be sent both fail before any request.</summary>
[TestClass]
public sealed class EngineSecretsTests
{
    private readonly string dataDir = Path.Combine(Path.GetTempPath(), "autopaper-tests-" + Guid.NewGuid().ToString("N")[..8]);
    private readonly CredentialStore credentials = new($"AutoPaperTest:{Guid.NewGuid():N}:");
    private readonly RecordingStore store;
    private readonly Engine engine;

    public EngineSecretsTests()
    {
        store = new RecordingStore(credentials);
        engine = Engine.Open(new EngineConfig(dataDir, Path.Combine(dataDir, "no-model"), "en-US", "Windows/tests AutoPaper/0"), store);
    }

    [TestCleanup]
    public void Cleanup()
    {
        credentials.Delete("openai.api_key");
        credentials.Delete("google.api_key");
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

    [TestMethod]
    public async Task NoKeyInCredentialManagerIsMissingKey()
    {
        var error = await Assert.ThrowsExactlyAsync<AutoPaperException.MissingKey>(() =>
            engine.TestProvider(new ProviderSelection(ProviderKind.OpenAi, "", null), ProviderJob.Concepts));
        Assert.AreEqual(ProviderKind.OpenAi, error.provider);
        CollectionAssert.Contains(store.Read.ToArray(), "openai.api_key");
    }

    [TestMethod]
    public async Task TheCoreReadsTheSavedKey()
    {
        // A key that can't go in a header: the core rejects it without sending it, proving it read this one.
        credentials.Set("openai.api_key", "sk-not-ascii-é");
        var error = await Assert.ThrowsExactlyAsync<AutoPaperException.InvalidKey>(() =>
            engine.TestProvider(new ProviderSelection(ProviderKind.OpenAi, "", null), ProviderJob.Images));
        Assert.AreEqual(ProviderKind.OpenAi, error.provider);
    }

    [TestMethod]
    public void CompatibleKeysBelongToTheirServer()
    {
        var account = AutopaperCoreMethods.SecretAccountFor(new ProviderSelection(ProviderKind.OpenAiCompatible, "", "http://192.168.1.20:1234/v1"));
        Assert.AreEqual("openai_compatible.api_key@http://192.168.1.20:1234", account);
        Assert.IsNull(AutopaperCoreMethods.SecretAccountFor(new ProviderSelection(ProviderKind.OpenAiCompatible, "", null)));
        Assert.IsNull(AutopaperCoreMethods.SecretAccountFor(new ProviderSelection(ProviderKind.ComfyUi, "", null)));
    }

    [TestMethod]
    public async Task DemoMakesAWallpaperWithoutAnyKey()
    {
        engine.AddKeyword("rain", KeywordWeight.Must);
        var made = await engine.Generate(Trigger.Manual, null);
        Assert.IsFalse(string.IsNullOrWhiteSpace(made.Concept.Title));
        var render = engine.RenderForDisplay(made.Id, new DisplayTarget("test", 1465, 839));
        Assert.IsTrue(File.Exists(render));
        StringAssert.EndsWith(render, "-1465x839.jpg");
        Assert.IsEmpty(store.Read, "Demo never asks for a key");
    }

    private sealed class RecordingStore(SecretStore inner) : SecretStore
    {
        public ConcurrentQueue<string> Read { get; } = new();

        public string? Get(string account)
        {
            Read.Enqueue(account);
            return inner.Get(account);
        }

        public void Set(string account, string value) => inner.Set(account, value);

        public void Delete(string account) => inner.Delete(account);
    }
}
