using AutoPaper.Core;
using AutoPaper.Models;

namespace AutoPaper.Tests;

/// <summary>Moods in words: New mood and Duplicate pick names the engine accepts (1–40 characters, unique ignoring
/// case), a mood's keywords read short with Avoids as what's left out, and the current one says so.</summary>
[TestClass]
public sealed class MoodTextTests
{
    [TestInitialize]
    public void UseAppStrings() => ReswStrings.Use();

    [TestMethod]
    public void NewMoodNamesAreUnique()
    {
        Assert.AreEqual("New mood", MoodText.NewName(["Rainy beach"]));
        Assert.AreEqual("New mood 2", MoodText.NewName(["Rainy beach", "new MOOD"]));
        Assert.AreEqual("New mood 3", MoodText.NewName(["New mood", "New  mood 2"]));
    }

    [TestMethod]
    public void DuplicatesAreCalledCopy()
    {
        Assert.AreEqual("Rainy beach copy", MoodText.CopyName("Rainy beach", ["Rainy beach"]));
        Assert.AreEqual("Rainy beach copy 2", MoodText.CopyName("Rainy beach", ["Rainy beach", "Rainy beach copy"]));
    }

    /// <summary>A long name still fits 40 characters with " copy" and a number.</summary>
    [TestMethod]
    public void NamesFitFortyCharacters()
    {
        var long40 = new string('a', 40);
        var copy = MoodText.CopyName(long40, [long40]);
        Assert.IsLessThanOrEqualTo(MoodText.MaxNameLength, copy.Length, copy);
        StringAssert.EndsWith(copy, " copy");
        var taken = new List<string> { long40, copy };
        for (var i = 2; i < 12; i++)
        {
            var next = MoodText.CopyName(long40, taken);
            Assert.IsLessThanOrEqualTo(MoodText.MaxNameLength, next.Length, next);
            CollectionAssert.DoesNotContain(taken, next);
            taken.Add(next);
        }
        var unique = MoodText.Unique(long40, [long40]);
        Assert.AreEqual(MoodText.MaxNameLength, unique.Length);
        StringAssert.EndsWith(unique, " 2");
    }

    [TestMethod]
    public void SummariesAreShortWithAvoidsLeftOut()
    {
        Keyword[] keywords =
        [
            new("1", "rain", KeywordWeight.Must, 0, 0),
            new("2", "beach", KeywordWeight.Maybe, 1, 0),
            new("3", "people", KeywordWeight.Avoid, 2, 0),
        ];
        Assert.AreEqual("rain · beach · no people", MoodText.Summary(keywords));
        Assert.AreEqual("rain, beach, no people", MoodText.SpokenSummary(keywords));
        Assert.AreEqual("No keywords yet", MoodText.Summary([]));
    }

    [TestMethod]
    public void TheCurrentMoodSaysSo()
    {
        var current = new Mood("m1", "Rainy beach", 0, 0.35f, 0, [new("1", "rain", KeywordWeight.Must, 0, 0)], true);
        var other = current with { Id = "m2", Name = "Forest", Active = false };
        Assert.AreEqual("Rainy beach, current mood", MoodText.SpokenName(current));
        Assert.AreEqual("Forest", MoodText.SpokenName(other));
        Assert.AreEqual("Rainy beach, current mood, rain", MoodText.RowName(current));
    }
}
