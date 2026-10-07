using AutoPaper.Core;
using AutoPaper.Models;
using AutoPaper.Services;
using CommunityToolkit.WinUI.Controls;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Navigation;
using CoreSettings = AutoPaper.Core.Settings;

namespace AutoPaper.Views.Panes;

/// <summary>Settings › Memory: quiet period, echoes, memory status, what AutoPaper has learned (taste_summary: what you
/// tend to like and dislike, shared by every mood) with Reset…, storage and its limit, Clear history….</summary>
public sealed partial class MemoryPage : Page
{
    private static readonly QuietPeriod[] Quiet =
        [QuietPeriod.OneMonth, QuietPeriod.ThreeMonths, QuietPeriod.SixMonths, QuietPeriod.OneYear, QuietPeriod.TwoYears];
    private static readonly EchoFrequency[] Echoes = [EchoFrequency.Off, EchoFrequency.Rarely, EchoFrequency.Sometimes, EchoFrequency.Often];
    private static readonly uint[] Limits = [256, 512, 1024, 2048, 5120];

    private bool loading = true;

    public MemoryPage()
    {
        InitializeComponent();
        foreach (var quiet in Quiet)
        {
            QuietBox.Items.Add(Loc.Get($"Quiet_{quiet}"));
        }
        foreach (var echoes in Echoes)
        {
            EchoesBox.Items.Add(Loc.Get($"Echoes_{echoes}"));
        }
        foreach (var limit in Limits)
        {
            LimitBox.Items.Add(Text.Size((ulong)limit * 1024 * 1024));
        }
    }

    private static AppModel Model => App.Model;

    protected override async void OnNavigatedTo(NavigationEventArgs e)
    {
        base.OnNavigatedTo(e);
        PaneHeader.Attach(Crumbs, "Memory", Frame);
        if (!Model.IsReady)
        {
            return;
        }
        ShowSettings();
        ShowMemoryStatus(Model.Memory);
        loading = false;
        await ShowLearnedAsync();
        await ShowUsageAsync();
    }

    /// <summary>The saved settings in the controls: on opening, and after a change that didn't save.</summary>
    private void ShowSettings()
    {
        var was = loading;
        loading = true;
        var settings = Model.Settings;
        QuietBox.SelectedIndex = Array.IndexOf(Quiet, settings.QuietPeriod);
        EchoesBox.SelectedIndex = Array.IndexOf(Echoes, settings.Echoes);
        var limit = Array.IndexOf(Limits, settings.StorageLimitMb);
        if (limit < 0)
        {
            // A limit set elsewhere (the CLI): show it as it is.
            if (LimitBox.Items.Count == Limits.Length)
            {
                LimitBox.Items.Add(Text.Size((ulong)settings.StorageLimitMb * 1024 * 1024));
            }
            limit = LimitBox.Items.Count - 1;
        }
        LimitBox.SelectedIndex = limit;
        loading = was;
    }

    /// <summary>Full or reduced, in the app's words; why it's reduced (the core's English, maybe a path) is in the log
    /// (AppModel writes it there at start-up), not here.</summary>
    private void ShowMemoryStatus(MemoryStatus? memory)
    {
        if (memory is null)
        {
            return;
        }
        StatusCard.Description = memory.Reduced
            ? Loc.Get("Memory_Reduced")
            : Loc.Format("Memory_Full", memory.EmbeddingModel);
    }

    /// <summary>What AutoPaper has learned from ratings, in the words the wallpapers used ("warm ivory").</summary>
    private async Task ShowLearnedAsync()
    {
        try
        {
            var taste = await Model.Call(engine => engine.TasteSummary());
            LikedCard.Description = taste.Liked.Length == 0 ? Loc.Get("LearnedNone/Text") : string.Join(", ", taste.Liked);
            DislikedCard.Description = taste.Disliked.Length == 0 ? Loc.Get("LearnedNone/Text") : string.Join(", ", taste.Disliked);
            var nothing = taste.Liked.Length == 0 && taste.Disliked.Length == 0;
            if (nothing)
            {
                LikedCard.Description = Loc.Get("LearnedEmpty/Text");
                DislikedCard.Description = Loc.Get("LearnedNone/Text");
            }
        }
        catch (Exception error)
        {
            LikedCard.Description = Text.Error(error, Model.Settings);
            DislikedCard.Description = "";
        }
    }

    private async Task ShowUsageAsync()
    {
        try
        {
            var usage = await Model.Call(engine => engine.StorageUsage());
            UsageCard.Description = Loc.Format("Memory_Usage", Text.Count(usage.ImagesOnDisk, "Image"), Text.Size(usage.ImageBytes), Text.Count(usage.Generations, "Wallpaper"));
        }
        catch (Exception error)
        {
            UsageCard.Description = Text.Error(error, Model.Settings);
        }
    }

    /// <summary>Saves one change (<paramref name="change"/> runs on the thread pool: it uses values read before);
    /// one that didn't save is said on its card, and the controls go back to the saved settings.</summary>
    private async Task SaveAsync(SettingsCard card, Func<CoreSettings, CoreSettings> change)
    {
        if (loading)
        {
            return;
        }
        try
        {
            await Model.UpdateSettingsAsync(change);
            PaneHeader.ClearProblem(card);
        }
        catch (Exception error)
        {
            ShowSettings();
            PaneHeader.ShowProblem(card, error, Text.Error(error, Model.Settings));
        }
    }

    private void Show(string message, bool error = false)
    {
        Result.Text = message;
        Result.Visibility = Visibility.Visible;
        if (error)
        {
            Model.AnnounceError(message);
        }
        else
        {
            Model.Announce(message);
        }
    }

    private async void OnQuietChanged(object sender, SelectionChangedEventArgs e)
    {
        if (QuietBox.SelectedIndex >= 0)
        {
            var quiet = Quiet[QuietBox.SelectedIndex];
            await SaveAsync(QuietCard, settings => settings with { QuietPeriod = quiet });
        }
    }

    private async void OnEchoesChanged(object sender, SelectionChangedEventArgs e)
    {
        if (EchoesBox.SelectedIndex >= 0)
        {
            var echoes = Echoes[EchoesBox.SelectedIndex];
            await SaveAsync(EchoesCard, settings => settings with { Echoes = echoes });
        }
    }

    private async void OnLimitChanged(object sender, SelectionChangedEventArgs e)
    {
        if (LimitBox.SelectedIndex >= 0 && LimitBox.SelectedIndex < Limits.Length)
        {
            var limit = Limits[LimitBox.SelectedIndex];
            await SaveAsync(LimitCard, settings => settings with { StorageLimitMb = limit });
            if (!loading)
            {
                // Apply the new limit now rather than after the next wallpaper.
                await Model.Call(engine => engine.Prune());
                await ShowUsageAsync();
            }
        }
    }

    /// <summary>Clear History…: keep memory (AutoPaper still avoids repeats) or forget everything.</summary>
    private async void OnClearHistory(object sender, RoutedEventArgs e)
    {
        var choice = await PaneHeader.ConfirmAsync(this,
            Loc.Get("ClearHistory_Title"),
            Loc.Get("ClearHistory_Body"),
            Loc.Get("ClearHistory_KeepMemory"),
            Loc.Get("ClearHistory_Everything"));
        if (choice == ContentDialogResult.None)
        {
            return;
        }
        var keepMemory = choice == ContentDialogResult.Primary;
        try
        {
            var hadCurrent = Model.HasCurrent;
            await Model.Call(engine => engine.ClearHistory(keepMemory));
            await Model.HistoryClearedAsync(hadCurrent);
            Show(Loc.Get(keepMemory ? "ClearHistory_DoneKept" : "ClearHistory_DoneAll"));
            await ShowLearnedAsync();
            await ShowUsageAsync();
        }
        catch (Exception error)
        {
            Show(Text.Error(error, Model.Settings), error: true);
        }
    }

    private async void OnResetLearned(object sender, RoutedEventArgs e)
    {
        var choice = await PaneHeader.ConfirmAsync(this,
            Loc.Get("ResetLearned_Title"), Loc.Get("ResetLearned_Body"), Loc.Get("ResetLearned_Confirm"));
        if (choice != ContentDialogResult.Primary)
        {
            return;
        }
        try
        {
            await Model.Call(engine => engine.ResetTaste());
            await Model.HistoryEditedAsync();
            await ShowLearnedAsync();
            Show(Loc.Get("ResetLearned_Done"));
        }
        catch (Exception error)
        {
            Show(Text.Error(error, Model.Settings), error: true);
        }
    }
}
