using System.Collections.ObjectModel;
using System.Runtime.InteropServices.WindowsRuntime;
using AutoPaper.Core;
using Microsoft.UI.Xaml.Data;
using Windows.Foundation;

namespace AutoPaper.Models;

/// <summary>History, newest first, a page at a time as the grid scrolls (history_by_mood(filter, mood, limit, offset);
/// no mood = every mood).</summary>
internal sealed partial class HistoryCollection(HistoryFilter filter, string? moodId) : ObservableCollection<HistoryItem>, ISupportIncrementalLoading
{
    public const uint PageSize = 48;

    /// <summary>One read at a time: a page being added and a refresh would otherwise both change the list.</summary>
    private readonly SemaphoreSlim gate = new(1, 1);

    public HistoryFilter Filter { get; } = filter;

    /// <summary>Only wallpapers made in this mood, or null for all.</summary>
    public string? MoodId { get; } = moodId;

    public bool HasMoreItems { get; private set; } = true;

    /// <summary>Raised after each page arrives (the page shows or hides its empty state).</summary>
    public event EventHandler? PageLoaded;

    public IAsyncOperation<LoadMoreItemsResult> LoadMoreItemsAsync(uint count) => AsyncInfo.Run(async _ =>
    {
        // A refresh is running: it brings the list up to date, and the grid asks for more again afterwards.
        if (!App.Model.IsReady || !gate.Wait(0))
        {
            return new LoadMoreItemsResult { Count = 0 };
        }
        try
        {
            var offset = (uint)Count;
            var page = await App.Model.Call(e => Describe(e, e.HistoryByMood(Filter, MoodId, PageSize, offset)));
            foreach (var (generation, description, hasEchoes) in page)
            {
                Add(new HistoryItem(generation, description, hasEchoes));
            }
            HasMoreItems = page.Count == PageSize;
            PageLoaded?.Invoke(this, EventArgs.Empty);
            return new LoadMoreItemsResult { Count = (uint)page.Count };
        }
        catch (Exception)
        {
            HasMoreItems = false;
            PageLoaded?.Invoke(this, EventArgs.Empty);
            return new LoadMoreItemsResult { Count = 0 };
        }
        finally
        {
            gate.Release();
        }
    });

    /// <summary>
    /// Brings the loaded items up to date (a rating, a new wallpaper, an echo, a deletion) in place: items that are
    /// still there keep their objects and their places in the grid, new ones are inserted where they belong and
    /// deleted ones removed. Rebuilding the list instead would hand the focused item's container to another
    /// wallpaper and scroll the grid back to the top (WCAG 2.4.3). Waits for a page being added.
    /// </summary>
    public async Task RefreshAsync()
    {
        if (!App.Model.IsReady)
        {
            return;
        }
        await gate.WaitAsync();
        try
        {
            var limit = Math.Max((uint)Count, PageSize);
            var fresh = await App.Model.Call(e => Describe(e, e.HistoryByMood(Filter, MoodId, limit, 0)));
            Apply(fresh);
            HasMoreItems = fresh.Count == limit;
            PageLoaded?.Invoke(this, EventArgs.Empty);
        }
        catch (Exception)
        {
            // The next refresh or scroll tries again.
        }
        finally
        {
            gate.Release();
        }
    }

    private void Apply(List<(Generation Generation, string Description, bool HasEchoes)> fresh) =>
        ListSync.Apply(
            this,
            fresh,
            item => item.Id,
            entry => entry.Generation.Id,
            entry => new HistoryItem(entry.Generation, entry.Description, entry.HasEchoes),
            (kept, entry) =>
            {
                kept.HasEchoes = entry.HasEchoes;
                kept.Generation = entry.Generation;
                kept.Description = entry.Description;
            });

    /// <summary>Each wallpaper's describe(id) and has_echoes(id), read with the page (off the UI thread).</summary>
    private static List<(Generation Generation, string Description, bool HasEchoes)> Describe(Engine engine, Generation[] page) =>
        page.Select(generation => (generation, engine.Describe(generation.Id), HasEchoes(engine, generation.Id))).ToList();

    /// <summary>False for a wallpaper deleted since the page was read (NotFound): it's gone at the next refresh.</summary>
    private static bool HasEchoes(Engine engine, string id)
    {
        try
        {
            return engine.HasEchoes(id);
        }
        catch (AutoPaperException.NotFound)
        {
            return false;
        }
    }
}
