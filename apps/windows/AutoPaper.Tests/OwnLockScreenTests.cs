using AutoPaper.Services;

namespace AutoPaper.Tests;

/// <summary>The person's own lock screen (user, 2026-10-06): kept before AutoPaper first changes it, put back on quit
/// and Restore my wallpaper; a picture the person chose since is never overwritten; what Windows can't let an app put
/// back (Spotlight, a slideshow, a picture it didn't hand over) is told apart, so it can be said.</summary>
[TestClass]
public sealed class OwnLockScreenTests
{
    private readonly string root = Path.Combine(Path.GetTempPath(), "autopaper-lock-" + Guid.NewGuid().ToString("N")[..8]);

    private static readonly byte[] Theirs = [0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3];
    private static readonly byte[] AutoPapers = [0xFF, 0xD8, 0xFF, 0xE0, 9, 9, 9];
    private static readonly byte[] NewChoice = [0x89, 0x50, 0x4E, 0x47, 7, 7];

    [TestCleanup]
    public void Cleanup()
    {
        try
        {
            Directory.Delete(root, recursive: true);
        }
        catch (IOException)
        {
        }
    }

    [TestMethod]
    public void ThePersonsPictureIsKeptOnceAndPutBack()
    {
        var own = new OwnLockScreen(root);
        Assert.IsNull(own.ToRestore(Theirs, LockScreenKind.Picture), "Nothing to put back before AutoPaper changed it.");

        Assert.IsTrue(own.Keep(Theirs, LockScreenKind.Picture));
        own.SetByAutoPaper(AutoPapers);
        // The next wallpaper: AutoPaper's own picture is showing, so nothing new is kept.
        Assert.IsTrue(own.ShowsAutoPapers(AutoPapers, LockScreenKind.Picture));
        Assert.IsFalse(own.Keep(AutoPapers, LockScreenKind.Picture));
        own.SetByAutoPaper([0xFF, 0xD8, 0xFF, 0xE0, 8]);

        var plan = own.ToRestore([0xFF, 0xD8, 0xFF, 0xE0, 8], LockScreenKind.Picture);
        Assert.IsNotNull(plan);
        Assert.AreEqual(OwnLockScreen.Outcome.Picture, plan.Value.Outcome);
        var fresh = own.FreshCopy();
        Assert.IsNotNull(fresh);
        CollectionAssert.AreEqual(Theirs, File.ReadAllBytes(fresh));
        Assert.AreNotEqual(own.Read()!.Copy, fresh, "Windows needs a new file name each time.");
        StringAssert.EndsWith(fresh, ".jpg");
        own.Restored();
        Assert.IsNull(own.ToRestore(Theirs, LockScreenKind.Picture), "Put back once; nothing more until AutoPaper changes it again.");
        Assert.IsTrue(own.Keep(Theirs, LockScreenKind.Picture), "The next change keeps theirs again.");
    }

    [TestMethod]
    public void APictureChosenSinceIsNeverOverwritten()
    {
        var own = new OwnLockScreen(root);
        own.Keep(Theirs, LockScreenKind.Picture);
        own.SetByAutoPaper(AutoPapers);
        Assert.IsNull(own.ToRestore(NewChoice, LockScreenKind.Picture), "The person picked another picture: it stays.");
        Assert.IsNull(own.ToRestore(AutoPapers, LockScreenKind.Picture), "And the old one is never put back later.");

        // Next time AutoPaper changes it, the new choice is what's kept.
        Assert.IsTrue(own.Keep(NewChoice, LockScreenKind.Picture));
        StringAssert.EndsWith(own.Read()!.Copy, ".png");
        own.SetByAutoPaper(AutoPapers);
        Assert.IsNull(own.ToRestore(AutoPapers, LockScreenKind.Spotlight), "Spotlight turned on since: it stays.");
    }

    [TestMethod]
    public void SpotlightSlideshowAndUnreadablePicturesAreToldApart()
    {
        var spotlight = new OwnLockScreen(Path.Combine(root, "a"));
        spotlight.Keep(Theirs, LockScreenKind.Spotlight);
        spotlight.SetByAutoPaper(AutoPapers);
        Assert.AreEqual(OwnLockScreen.Outcome.Spotlight, spotlight.ToRestore(AutoPapers, LockScreenKind.Picture)!.Value.Outcome);
        Assert.IsNotNull(spotlight.FreshCopy(), "Its last picture can still go back.");

        var slideshow = new OwnLockScreen(Path.Combine(root, "b"));
        slideshow.Keep(null, LockScreenKind.Slideshow);
        slideshow.SetByAutoPaper(null);
        Assert.AreEqual(OwnLockScreen.Outcome.Slideshow, slideshow.ToRestore(null, LockScreenKind.Picture)!.Value.Outcome);
        Assert.IsNull(slideshow.FreshCopy());

        // Windows handed over no picture: AutoPaper can't tell a new choice from its own, and can't put theirs back.
        var unreadable = new OwnLockScreen(Path.Combine(root, "c"));
        Assert.IsTrue(unreadable.Keep(null, LockScreenKind.Picture));
        unreadable.SetByAutoPaper(null);
        Assert.IsFalse(unreadable.Keep(null, LockScreenKind.Picture), "Kept once, until it's put back.");
        Assert.AreEqual(OwnLockScreen.Outcome.Unreadable, unreadable.ToRestore(null, LockScreenKind.Picture)!.Value.Outcome);
    }

    [TestMethod]
    public void FingerprintsIgnoreNothing()
    {
        Assert.IsNull(OwnLockScreen.Fingerprint(null));
        Assert.IsNull(OwnLockScreen.Fingerprint([]));
        Assert.AreNotEqual(OwnLockScreen.Fingerprint(Theirs), OwnLockScreen.Fingerprint(AutoPapers));
        Assert.AreEqual(OwnLockScreen.Fingerprint(Theirs), OwnLockScreen.Fingerprint([.. Theirs]));
    }
}
