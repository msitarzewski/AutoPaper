using AutoPaper.Services;

namespace AutoPaper.Tests;

[TestClass]
public sealed class CredentialStoreTests
{
    // A prefix of its own per run: the person's "AutoPaper:" keys are never read or written.
    private readonly CredentialStore store = new($"AutoPaperTest:{Guid.NewGuid():N}:");
    private readonly List<string> accounts = [];

    private string Account(string name)
    {
        accounts.Add(name);
        return name;
    }

    [TestCleanup]
    public void Cleanup()
    {
        foreach (var account in accounts)
        {
            store.Delete(account);
        }
    }

    [TestMethod]
    public void MissingAccountReadsAsNoKey()
    {
        Assert.IsNull(store.Get(Account("openai.api_key")));
    }

    [TestMethod]
    public void SavesReadsAndDeletes()
    {
        var account = Account("openai.api_key");
        store.Set(account, "sk-test-0123456789");
        Assert.AreEqual("sk-test-0123456789", store.Get(account));
        store.Delete(account);
        Assert.IsNull(store.Get(account));
    }

    [TestMethod]
    public void KeepsUnicodeAndTrimsSpace()
    {
        var account = Account("google.api_key");
        store.Set(account, "  clé-🔑-key \n");
        Assert.AreEqual("clé-🔑-key", store.Get(account));
    }

    [TestMethod]
    public void OverwritesTheSavedKey()
    {
        var account = Account("google.api_key");
        store.Set(account, "first");
        store.Set(account, "second");
        Assert.AreEqual("second", store.Get(account));
    }

    [TestMethod]
    public void AnEmptyValueDeletesTheKey()
    {
        var account = Account("openai_compatible.api_key@http://127.0.0.1:1234");
        store.Set(account, "abc");
        store.Set(account, "   ");
        Assert.IsNull(store.Get(account));
    }

    [TestMethod]
    public void DeletingAMissingKeyIsFine()
    {
        store.Delete(Account("never.saved"));
        Assert.IsNull(store.Get("never.saved"));
    }

    [TestMethod]
    public void RefusesAKeyTooLongForCredentialManager()
    {
        var account = Account("openai.api_key");
        Assert.ThrowsExactly<ArgumentException>(() => store.Set(account, new string('k', 1281)));
        Assert.IsNull(store.Get(account));
    }

    [TestMethod]
    public void TargetNamesCarryThePrefix()
    {
        Assert.AreEqual("AutoPaper:openai.api_key", new CredentialStore().TargetName("openai.api_key"));
    }
}
