using AutoPaper.Services;

namespace AutoPaper.Tests;

/// <summary>The person's own wallpaper (docs/app-spec.md 3a): before AutoPaper replaces the desktop, each monitor's
/// picture that isn't AutoPaper's is copied and kept with its position and colour; what to put back survives the
/// original file being deleted; AutoPaper's own pictures (its renders, and the copies it restores) are never kept as
/// the person's.</summary>
[TestClass]
public sealed class OwnWallpaperTests
{
    private readonly string root = Path.Combine(Path.GetTempPath(), "autopaper-own-" + Guid.NewGuid().ToString("N")[..8]);

    private string DataFolder => Path.Combine(root, "LocalState");

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

    private string Picture(string name, string text)
    {
        var path = Path.Combine(root, "Pictures", name);
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        File.WriteAllText(path, text);
        return path;
    }

    [TestMethod]
    public void AutoPapersPicturesAreTheOnesInItsDataFolder()
    {
        Assert.IsTrue(OwnWallpaper.IsAutoPapers(Path.Combine(DataFolder, "renders", "a-1920x1080.jpg"), DataFolder));
        Assert.IsTrue(OwnWallpaper.IsAutoPapers(Path.Combine(DataFolder.ToUpperInvariant(), "renders", "a.jpg"), DataFolder));
        Assert.IsFalse(OwnWallpaper.IsAutoPapers(@"C:\Windows\Web\Wallpaper\Windows\img0.jpg", DataFolder));
        Assert.IsFalse(OwnWallpaper.IsAutoPapers(DataFolder + "-other\\a.jpg", DataFolder), "a sibling folder isn't ours");
    }

    [TestMethod]
    public void ThePersonsPicturesAreCopiedAndKept()
    {
        var own = new OwnWallpaper(DataFolder);
        Assert.IsNull(own.ToRestore());
        Assert.IsFalse(own.HasCopy);
        var beach = Picture("beach.jpg", "beach");
        var ours = Path.Combine(DataFolder, "renders", "x-1920x1080.jpg");
        Assert.IsTrue(own.Keep(new Dictionary<string, string> { ["monitor-1"] = beach, ["monitor-2"] = ours, ["monitor-3"] = "" }, 4, 0x00112233));
        var kept = own.ToRestore()!;
        Assert.HasCount(2, kept.Pictures, "monitor 2 showed AutoPaper's");
        Assert.AreEqual(beach, kept.Pictures["monitor-1"], "the person's own file goes back while it's there");
        Assert.AreEqual("", kept.Pictures["monitor-3"], "no picture: the colour shows");
        Assert.AreEqual(4, kept.Position);
        Assert.AreEqual(0x00112233u, kept.Color);

        // The original goes away; the copy comes back instead.
        File.Delete(beach);
        var copy = own.ToRestore()!.Pictures["monitor-1"];
        Assert.IsTrue(OwnWallpaper.IsAutoPapers(copy, DataFolder), "the copy is in AutoPaper's folder");
        Assert.AreEqual("beach", File.ReadAllText(copy));
    }

    /// <summary>AutoPaper's on every monitor (or its restored copy): nothing new is kept, and what was kept stays.</summary>
    [TestMethod]
    public void AutoPapersPicturesDontReplaceWhatWasKept()
    {
        var own = new OwnWallpaper(DataFolder);
        var forest = Picture("forest.png", "forest");
        own.Keep(new Dictionary<string, string> { ["m"] = forest }, 2, 0);
        File.Delete(forest);
        var copy = own.ToRestore()!.Pictures["m"];
        Assert.IsFalse(own.Keep(new Dictionary<string, string> { ["m"] = Path.Combine(DataFolder, "renders", "y.jpg") }, 4, 0));
        Assert.IsFalse(own.Keep(new Dictionary<string, string> { ["m"] = copy }, 4, 0), "the restored copy is the person's, already kept");
        Assert.AreEqual(2, own.ToRestore()!.Position);
        Assert.AreEqual("forest", File.ReadAllText(own.ToRestore()!.Pictures["m"]));
    }

    /// <summary>The person chose a new picture of their own meanwhile: that one is kept instead.</summary>
    [TestMethod]
    public void ANewPictureOfTheirOwnReplacesTheOldCopy()
    {
        var own = new OwnWallpaper(DataFolder);
        var one = Picture("one.jpg", "one");
        own.Keep(new Dictionary<string, string> { ["m"] = one }, 2, 0);
        File.Delete(one);
        var first = own.ToRestore()!.Pictures["m"];
        var two = Picture("two.png", "two");
        Assert.IsTrue(own.Keep(new Dictionary<string, string> { ["m"] = two }, 3, 0));
        Assert.AreEqual(two, own.ToRestore()!.Pictures["m"]);
        File.Delete(two);
        var second = own.ToRestore()!.Pictures["m"];
        Assert.AreEqual("two", File.ReadAllText(second));
        Assert.IsFalse(File.Exists(first), "the old copy (another file type) is removed");
        Assert.AreEqual(3, own.ToRestore()!.Position);
    }

    [TestMethod]
    public void AMissingFileIsSkipped()
    {
        var own = new OwnWallpaper(DataFolder);
        Assert.IsFalse(own.Keep(new Dictionary<string, string> { ["m"] = Path.Combine(root, "gone.jpg") }, 4, 0));
        Assert.IsNull(own.ToRestore());
    }

    /// <summary>A slideshow is kept as one (its folders, shuffle and interval) and comes back as one; the picture it
    /// showed is kept too, for when its folder has gone.</summary>
    [TestMethod]
    public void ASlideshowIsKeptAsASlideshow()
    {
        var own = new OwnWallpaper(DataFolder);
        var shown = Picture("Slides\\one.jpg", "one");
        var slideshow = new SlideshowSetting([Path.GetDirectoryName(shown)!], 1, 600_000);
        Assert.IsTrue(own.Keep(new Dictionary<string, string> { ["m"] = shown }, 4, 0, BackgroundKind.Slideshow, slideshow));
        var kept = own.ToRestore()!;
        Assert.AreEqual(BackgroundKind.Slideshow, kept.Kind);
        CollectionAssert.AreEqual(slideshow.Items.ToList(), kept.Slideshow!.Items.ToList());
        Assert.AreEqual(1, kept.Slideshow.Options);
        Assert.AreEqual(600_000u, kept.Slideshow.TickMs);
        Assert.AreEqual(shown, kept.Pictures["m"]);

        // Later the person chose a picture of their own: no slideshow is kept any more.
        var beach = Picture("beach.jpg", "beach");
        Assert.IsTrue(own.Keep(new Dictionary<string, string> { ["m"] = beach }, 4, 0));
        Assert.AreEqual(BackgroundKind.Picture, own.ToRestore()!.Kind);
        Assert.IsNull(own.ToRestore()!.Slideshow);
    }

    /// <summary>Windows Spotlight comes back as its last picture (Windows turns Spotlight off when an app sets a
    /// picture): while that picture is still what shows, it's still the person's Spotlight background (the link to
    /// turn it on stays); once they choose another picture, it's a picture.</summary>
    [TestMethod]
    public void SpotlightStaysSpotlightUntilThePersonChoosesAnother()
    {
        var own = new OwnWallpaper(DataFolder);
        var spotlight = Picture("IrisService\\134357027284952944.jpg", "spotlight");
        Assert.IsTrue(own.Keep(new Dictionary<string, string> { ["m"] = spotlight }, 4, 0, BackgroundKind.Spotlight));
        Assert.AreEqual(BackgroundKind.Spotlight, own.ToRestore()!.Kind);

        // Put back, then AutoPaper's again: the desktop showed the restored picture, with Spotlight off.
        Assert.IsTrue(own.Keep(new Dictionary<string, string> { ["m"] = own.ToRestore()!.Pictures["m"] }, 4, 0, BackgroundKind.Picture));
        Assert.AreEqual(BackgroundKind.Spotlight, own.ToRestore()!.Kind, "still their Spotlight background");

        var chosen = Picture("chosen.png", "chosen");
        Assert.IsTrue(own.Keep(new Dictionary<string, string> { ["m"] = chosen }, 4, 0, BackgroundKind.Picture));
        Assert.AreEqual(BackgroundKind.Picture, own.ToRestore()!.Kind);
    }

    /// <summary>What was kept before the kind of background was recorded reads as pictures.</summary>
    [TestMethod]
    public void AnOlderRecordIsPictures()
    {
        Directory.CreateDirectory(Path.Combine(DataFolder, "own-wallpaper"));
        var beach = Picture("beach.jpg", "beach");
        File.WriteAllText(Path.Combine(DataFolder, "own-wallpaper", "own.json"),
            $$"""{"Pictures":{"m":{{System.Text.Json.JsonSerializer.Serialize(beach)}}},"Position":4,"Color":0}""");
        var kept = new OwnWallpaper(DataFolder).ToRestore()!;
        Assert.AreEqual(BackgroundKind.Picture, kept.Kind);
        Assert.AreEqual(beach, kept.Pictures["m"]);
    }
}
