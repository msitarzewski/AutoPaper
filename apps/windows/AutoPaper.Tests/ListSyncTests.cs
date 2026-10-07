using System.Collections.ObjectModel;
using System.Collections.Specialized;
using AutoPaper.Models;

namespace AutoPaper.Tests;

/// <summary>History's in-place refresh (ListSync): kept items stay the same objects, and the list changes only where it
/// must, so the grid keeps focus and scroll position after a delete, an echo or a rating.</summary>
[TestClass]
public sealed class ListSyncTests
{
    private sealed class Item(string id, string text)
    {
        public string Id { get; } = id;
        public string Text { get; set; } = text;
    }

    private static List<NotifyCollectionChangedAction> Sync(ObservableCollection<Item> list, params string[] fresh)
    {
        var changes = new List<NotifyCollectionChangedAction>();
        list.CollectionChanged += (_, args) => changes.Add(args.Action);
        ListSync.Apply(list, fresh, item => item.Id, entry => entry, entry => new Item(entry, "new"), (item, entry) => item.Text = "updated");
        Assert.AreEqual(string.Join(",", fresh), string.Join(",", list.Select(item => item.Id)));
        return changes;
    }

    private static ObservableCollection<Item> List(params string[] ids) => new(ids.Select(id => new Item(id, "old")));

    [TestMethod]
    public void ADeletedItemIsRemovedAndTheOthersAreKept()
    {
        var list = List("a", "b", "c", "d");
        var kept = list.Where(item => item.Id != "c").ToList();
        var changes = Sync(list, "a", "b", "d");
        CollectionAssert.AreEqual(new[] { NotifyCollectionChangedAction.Remove }, changes);
        CollectionAssert.AreEqual(kept, list.ToList(), "the same objects, so their containers stay");
        Assert.IsTrue(list.All(item => item.Text == "updated"));
    }

    [TestMethod]
    public void ANewItemIsInsertedAtTheTop()
    {
        var list = List("a", "b", "c");
        var original = list[1];
        var changes = Sync(list, "echo", "a", "b", "c");
        CollectionAssert.AreEqual(new[] { NotifyCollectionChangedAction.Add }, changes);
        Assert.AreSame(original, list[2], "the echoed wallpaper keeps its object");
        Assert.AreEqual("new", list[0].Text);
    }

    [TestMethod]
    public void AnUnchangedListOnlyUpdatesItsItems()
    {
        var list = List("a", "b");
        var changes = Sync(list, "a", "b");
        Assert.IsEmpty(changes);
        Assert.IsTrue(list.All(item => item.Text == "updated"));
    }

    [TestMethod]
    public void AnItemThatLeftAFilterGoesAndTheListCanEmpty()
    {
        var list = List("a");
        CollectionAssert.AreEqual(new[] { NotifyCollectionChangedAction.Remove }, Sync(list));
        Assert.IsEmpty(list);
    }

    [TestMethod]
    public void AMovedItemIsMovedNotRebuilt()
    {
        var list = List("a", "b", "c");
        var c = list[2];
        var changes = Sync(list, "c", "a", "b");
        CollectionAssert.AreEqual(new[] { NotifyCollectionChangedAction.Move }, changes);
        Assert.AreSame(c, list[0]);
    }

    [TestMethod]
    public void ARepeatedItemFromAShiftedPageIsDropped()
    {
        // A page read while a new wallpaper arrived repeats the last item of the page before.
        var list = List("b", "c", "c");
        Sync(list, "a", "b", "c");
        Assert.HasCount(3, list);
    }

    [TestMethod]
    public void InsertsAndRemovesTogether()
    {
        var list = List("a", "b", "c", "d");
        Sync(list, "x", "a", "c", "y", "d");
    }
}
