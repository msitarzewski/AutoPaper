using System.Collections.ObjectModel;

namespace AutoPaper.Models;

/// <summary>
/// Brings an ObservableCollection in line with a fresh list in place, with the fewest changes (removes, moves,
/// inserts), so the items that stay keep their objects and their places: a list control keeps focus on them and
/// doesn't scroll (History after a delete, an echo or a rating). No WinUI types, so AutoPaper.Tests can link it.
/// </summary>
internal static class ListSync
{
    /// <param name="list">The collection shown.</param>
    /// <param name="fresh">What it should hold, in order (unique keys).</param>
    /// <param name="keyOf">An item's key.</param>
    /// <param name="freshKeyOf">A fresh entry's key.</param>
    /// <param name="create">A new item for a fresh entry that isn't in the list.</param>
    /// <param name="update">Updates a kept item from its fresh entry.</param>
    public static void Apply<TItem, TFresh, TKey>(
        ObservableCollection<TItem> list,
        IReadOnlyList<TFresh> fresh,
        Func<TItem, TKey> keyOf,
        Func<TFresh, TKey> freshKeyOf,
        Func<TFresh, TItem> create,
        Action<TItem, TFresh> update)
        where TKey : notnull
    {
        var wanted = fresh.Select(freshKeyOf).ToHashSet();
        for (var index = list.Count - 1; index >= 0; index--)
        {
            if (!wanted.Contains(keyOf(list[index])))
            {
                list.RemoveAt(index);
            }
        }
        var comparer = EqualityComparer<TKey>.Default;
        for (var index = 0; index < fresh.Count; index++)
        {
            var key = freshKeyOf(fresh[index]);
            var at = -1;
            for (var look = index; look < list.Count; look++)
            {
                if (comparer.Equals(keyOf(list[look]), key))
                {
                    at = look;
                    break;
                }
            }
            if (at < 0)
            {
                list.Insert(index, create(fresh[index]));
                continue;
            }
            if (at != index)
            {
                list.Move(at, index);
            }
            update(list[index], fresh[index]);
        }
        // Anything left past the fresh list: a key the list held twice (a page read while a new item arrived can
        // repeat one).
        while (list.Count > fresh.Count)
        {
            list.RemoveAt(list.Count - 1);
        }
    }
}
