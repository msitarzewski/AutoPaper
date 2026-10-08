using System.Collections.ObjectModel;
using System.ComponentModel;
using AutoPaper.Controls;
using AutoPaper.Core;
using AutoPaper.Models;
using AutoPaper.Services;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Navigation;
using Windows.System;

namespace AutoPaper.Views;

/// <summary>
/// Moods (docs/app-spec.md, "Moods"; the user's layout, 2026-10-06: navigation › list › detail): the list of moods
/// beside the selected one (Fluent list/details). The list has each mood's name, its keywords in short and "Current"
/// or Use; New mood in its command bar; right-click (Shift+F10) for Use, Duplicate, Rename…, Move up/down, Delete…;
/// double-click or Enter uses one; Delete deletes (with a confirmation); drag or Alt+Up/Alt+Down reorders.
/// With no mood selected (the navigation's Moods item) the detail is "Your moods": totals, the keywords by weight, the
/// last 30 days as a chart, and every mood from most to least used (mood_stats, activity). A selected mood's detail
/// has its name in the title position (renamed in place), "Current mood" or Use this mood and Now's reload/stop in its
/// command bar, a header with what it has made, its keywords (edit inline, Must · Maybe · Avoid, remove, reorder), its
/// Surprise, its recent wallpapers (a click shows one on the desktop; History's menu), Delete mood…. Every change goes
/// to the engine at once.
/// </summary>
public sealed partial class MoodsPage : Page, IDefaultFocus, IInnerBack, IWallpaperHost
{
    /// <summary>Narrower than this, the page shows the list or a mood, not both.</summary>
    private const double WideWidth = 720;
    private const uint RecentCount = 6;

    private readonly DispatcherTimerLite surpriseSave;
    /// <summary>History changed (a wallpaper made, rated, deleted): the shown mood's recent wallpapers, read again.</summary>
    private readonly DispatcherTimerLite recentRefresh;
    private bool loading;
    /// <summary>A row's Must · Maybe · Avoid is being set from its keyword (not a choice the person made).</summary>
    private bool showingWeight;
    private bool wide = true;
    /// <summary>Narrow layout: the selected mood is showing (not the list).</summary>
    private bool showingDetail;
    /// <summary><see cref="CanGoBackInside"/> as the window was last told it.</summary>
    private bool toldCanGoBackInside;
    /// <summary>A mood's rename on its way to the engine (leaving the field and showing another mood can both save
    /// the same typed name: it's saved once).</summary>
    private (string Id, string Name)? renaming;
    /// <summary>The mood in the detail.</summary>
    private string? shownId;
    /// <summary>The Surprise waiting to be saved, and whose it is (a selection change mustn't move it to another mood).</summary>
    private (string MoodId, float Surprise)? pendingSurprise;
    private string? requested;
    /// <summary>What each mood has made (mood_stats), by mood.</summary>
    private Dictionary<string, MoodStats> stats = [];
    /// <summary>History changed or a mood did: the summary and the shown mood's numbers, read again.</summary>
    private readonly DispatcherTimerLite statsRefresh;
    /// <summary>History's menu, for the thumbnails here.</summary>
    private readonly MenuFlyout wallpaperMenu;

    public MoodsPage()
    {
        InitializeComponent();
        MenuNames.Name((MenuFlyout)Resources["RowMenu"], Loc.Get("Menu_KeywordActions"));
        MenuNames.Name(MoodMenu, Loc.Get("Menu_MoodActions"));
        // The list marks Enter handled (selection): listened to anyway, for Use.
        MoodList.AddHandler(KeyDownEvent, new KeyEventHandler(OnMoodListKeyDown), handledEventsToo: true);
        surpriseSave = new DispatcherTimerLite(DispatcherQueue, TimeSpan.FromMilliseconds(400), SaveSurpriseAsync);
        recentRefresh = new DispatcherTimerLite(DispatcherQueue, TimeSpan.FromMilliseconds(300), async () =>
        {
            if (shownId is { } id)
            {
                await ShowRecentAsync(id);
            }
        });
        statsRefresh = new DispatcherTimerLite(DispatcherQueue, TimeSpan.FromMilliseconds(300), RefreshStatsAsync);
        wallpaperMenu = WallpaperActions.Menu(this, WallpaperActions.ItemOf);
        Loaded += async (_, _) =>
        {
            Model.MoodsChanged += OnMoodsChanged;
            Model.PropertyChanged += OnModelChanged;
            Model.HistoryChanged += OnHistoryChanged;
            await LoadAsync();
        };
        Unloaded += (_, _) =>
        {
            Model.MoodsChanged -= OnMoodsChanged;
            Model.PropertyChanged -= OnModelChanged;
            Model.HistoryChanged -= OnHistoryChanged;
            recentRefresh.Stop();
            statsRefresh.Stop();
            if (pendingSurprise is not null)
            {
                surpriseSave.Stop();
                _ = SaveSurpriseAsync();
            }
        };
    }

    internal AppModel Model => App.Model;

    internal ObservableCollection<MoodItem> MoodItems { get; } = [];

    /// <summary>The shown mood's keywords.</summary>
    internal ObservableCollection<KeywordItem> Items { get; } = [];

    private MenuFlyout MoodMenu => (MenuFlyout)Resources["MoodMenu"];

    private Mood? Shown => Model.Moods.FirstOrDefault(mood => mood.Id == shownId);

    /// <summary>Raised when a narrow window starts or stops showing a mood (the window's Back follows).</summary>
    public event EventHandler? CanGoBackInsideChanged;

    /// <summary>A narrow window showing a mood: Back (the title bar's, Alt+Left) goes to the list first.</summary>
    public bool CanGoBackInside => !wide && showingDetail;

    /// <summary>The mood shown changed (null: the summary of every mood): the window's navigation follows.</summary>
    public event EventHandler<string?>? ShownMoodChanged;

    /// <summary>The mood in the detail, or null while the summary shows.</summary>
    public string? ShownMoodId => shownId;

    /// <summary>The detail's reload/stop (Ctrl+R, F5, Esc act on it while a mood shows).</summary>
    internal MakeOrStopButton? MakeOrStop => shownId is null ? null : MoodMakeOrStop;

    public bool GoBackInside()
    {
        if (!CanGoBackInside)
        {
            return false;
        }
        ShowDetailPane(false);
        return true;
    }

    /// <summary>The selected mood's row; on a narrow window showing a mood, its name; with no mood selected, New mood
    /// (focusing a row would select it, and show that mood instead of the summary).</summary>
    public UIElement? DefaultFocusElement => !wide && showingDetail
        ? MoodName
        : MoodList.SelectedItem is { } selected ? MoodList.ContainerFromItem(selected) as UIElement : NewMoodButton;

    protected override void OnNavigatedTo(NavigationEventArgs e)
    {
        base.OnNavigatedTo(e);
        requested = e.Parameter as string;
    }

    /// <summary>Shows a mood (the navigation, the Now view's narrow-keywords note, a problem's link, Edit moods in the
    /// menu), or, with null (or a mood that's gone), the summary of every mood.</summary>
    public void Select(string? moodId)
    {
        if (moodId is not null && MoodItems.FirstOrDefault(item => item.Id == moodId) is { } item)
        {
            MoodList.SelectedItem = item;
            MoodList.ScrollIntoView(item);
            if (!wide)
            {
                ShowDetailPane(true);
            }
        }
        else
        {
            MoodList.SelectedItem = null;
            if (!wide && showingDetail)
            {
                ShowDetailPane(false);
            }
            _ = ShowSummaryAsync();
        }
    }

    private async Task LoadAsync()
    {
        if (!Model.IsReady)
        {
            return;
        }
        // What's known now first (so focus has a row to go to), then the engine's current list.
        SyncList();
        SelectInitial();
        await Model.MoodsEditedAsync();
    }

    /// <summary>The mood asked for (a mood in the navigation, a link), else the summary of every mood.</summary>
    private void SelectInitial()
    {
        var id = requested;
        requested = null;
        Select(id);
    }

    private void OnModelChanged(object? sender, PropertyChangedEventArgs args)
    {
        if (args.PropertyName == nameof(AppModel.KeywordsNarrow))
        {
            ShowNarrowNote();
        }
        else if (args.PropertyName == nameof(AppModel.IsReady) && Model.IsReady)
        {
            _ = LoadAsync();
        }
    }

    /// <summary>The engine's moods changed (here, in the notification-area menu, or a keyword added on the first-run
    /// page): the rows update in place (focus and scroll stay), and the shown mood's name and "Current" with them.</summary>
    private void OnMoodsChanged(object? sender, EventArgs e)
    {
        SyncList();
        if (shownId is not null && Shown is null)
        {
            Select(null); // The mood shown was deleted (here or in the navigation).
        }
        ShowHeader();
        statsRefresh.Restart();
    }

    /// <summary>A wallpaper was made, rated or deleted while the page is open: the shown mood's recent wallpapers and
    /// numbers, or the summary, follow.</summary>
    private void OnHistoryChanged(object? sender, EventArgs e)
    {
        recentRefresh.Restart();
        statsRefresh.Restart();
    }

    private void SyncList()
    {
        ListSync.Apply(MoodItems, Model.Moods.ToList(), item => item.Id, mood => mood.Id, mood => new MoodItem(mood), (item, mood) => item.Mood = mood);
        foreach (var item in MoodItems)
        {
            if (MoodList.ContainerFromItem(item) is ListViewItem container)
            {
                AutomationProperties.SetName(container, item.SpokenName);
            }
        }
    }

    private void Say(string text, Exception? error = null)
    {
        if (error is null)
        {
            Model.Announce(text);
        }
        else
        {
            Model.AnnounceError(text);
        }
    }

    // ── Layout: list beside the mood, or one at a time ──────────────────────────────────────────

    private void OnLayoutSizeChanged(object sender, SizeChangedEventArgs e)
    {
        var nowWide = e.NewSize.Width >= WideWidth;
        if (nowWide != wide || e.PreviousSize.Width == 0)
        {
            wide = nowWide;
            ApplyLayout();
        }
    }

    private void ApplyLayout()
    {
        if (shownId is null)
        {
            showingDetail = false;
        }
        if (wide)
        {
            ListColumn.Width = new GridLength(320);
            DetailColumn.Width = new GridLength(1, GridUnitType.Star);
            Grid.SetColumn(DetailPane, 1);
            Grid.SetColumnSpan(DetailPane, 1);
            Grid.SetColumnSpan(ListPane, 1);
            ListPane.Visibility = Visibility.Visible;
            DetailPane.Visibility = Visibility.Visible;
        }
        else
        {
            ListColumn.Width = new GridLength(1, GridUnitType.Star);
            DetailColumn.Width = new GridLength(0);
            Grid.SetColumn(DetailPane, 0);
            Grid.SetColumnSpan(ListPane, 2);
            Grid.SetColumnSpan(DetailPane, 2);
            ListPane.Visibility = showingDetail ? Visibility.Collapsed : Visibility.Visible;
            DetailPane.Visibility = showingDetail ? Visibility.Visible : Visibility.Collapsed;
        }
        if (CanGoBackInside != toldCanGoBackInside)
        {
            toldCanGoBackInside = CanGoBackInside;
            CanGoBackInsideChanged?.Invoke(this, EventArgs.Empty);
        }
    }

    /// <summary>Narrow windows: the mood (true) or the list (false), with focus going along. The side that's coming
    /// is shown and focused before the other goes, so focus never passes through the title bar on the way (a screen
    /// reader would say "Back" first).</summary>
    private void ShowDetailPane(bool show)
    {
        if (wide)
        {
            return;
        }
        showingDetail = show;
        var focused = false;
        if (show)
        {
            DetailPane.Visibility = Visibility.Visible;
            Layout.UpdateLayout();
            focused = MoodName.Focus(FocusState.Programmatic);
        }
        else if (MoodList.SelectedItem is { } selected)
        {
            ListPane.Visibility = Visibility.Visible;
            Layout.UpdateLayout();
            MoodList.ScrollIntoView(selected);
            MoodList.UpdateLayout();
            focused = (MoodList.ContainerFromItem(selected) as Control)?.Focus(FocusState.Keyboard) == true;
        }
        ApplyLayout();
        if (!focused && !show && MoodList.SelectedItem is { } row)
        {
            // The row's container didn't exist yet: focus it once the list has laid out.
            DispatcherQueue.TryEnqueue(() => (MoodList.ContainerFromItem(row) as Control)?.Focus(FocusState.Keyboard));
        }
    }

    // ── The list ────────────────────────────────────────────────────────────────────────────────

    private void OnMoodContainerChanging(ListViewBase sender, ContainerContentChangingEventArgs args)
    {
        if (args.Item is MoodItem item)
        {
            AutomationProperties.SetName(args.ItemContainer, item.SpokenName);
            args.ItemContainer.ContextFlyout = MoodMenu;
        }
    }

    private void OnMoodSelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (MoodList.SelectedItem is MoodItem item)
        {
            if (item.Id != shownId)
            {
                _ = ShowMoodAsync(item.Id);
            }
        }
        else if (shownId is not null)
        {
            _ = ShowSummaryAsync();
        }
    }

    /// <summary>Narrow windows: a tap on a row opens it (not on its Use button).</summary>
    private void OnMoodTapped(object sender, TappedRoutedEventArgs e)
    {
        if (!wide && !IsInButton(e.OriginalSource as DependencyObject) && ItemOf(e.OriginalSource) is { } item)
        {
            MoodList.SelectedItem = item;
            ShowDetailPane(true);
        }
    }

    /// <summary>Double-click uses the mood.</summary>
    private async void OnMoodDoubleTapped(object sender, DoubleTappedRoutedEventArgs e)
    {
        if (!IsInButton(e.OriginalSource as DependencyObject) && ItemOf(e.OriginalSource) is { } item)
        {
            e.Handled = true;
            await UseAsync(item);
        }
    }

    /// <summary>Enter uses the focused mood (on a narrow window, it opens it); Delete deletes it (with a confirmation).</summary>
    private async void OnMoodListKeyDown(object sender, KeyRoutedEventArgs e)
    {
        if (e.OriginalSource is not ListViewItem { Content: MoodItem item })
        {
            return;
        }
        if (e.Key == VirtualKey.Enter)
        {
            e.Handled = true;
            if (wide)
            {
                await UseAsync(item);
            }
            else
            {
                MoodList.SelectedItem = item;
                ShowDetailPane(true);
            }
        }
        else if (e.Key == VirtualKey.Delete)
        {
            e.Handled = true;
            await DeleteAsync(item);
        }
    }

    private static bool IsInButton(DependencyObject? element)
    {
        for (var node = element; node is not null and not ListViewItem; node = Microsoft.UI.Xaml.Media.VisualTreeHelper.GetParent(node))
        {
            if (node is ButtonBase)
            {
                return true;
            }
        }
        return false;
    }

    private static MoodItem? ItemOf(object? source) => source is FrameworkElement { DataContext: MoodItem item } ? item : null;

    /// <summary>The mood row with keyboard focus, else the selected one: a key acts on the row it's pressed on (focus
    /// moved by a screen reader or UI Automation doesn't move the selection).</summary>
    private MoodItem? FocusedMood()
    {
        for (var node = FocusManager.GetFocusedElement(XamlRoot) as DependencyObject; node is not null; node = Microsoft.UI.Xaml.Media.VisualTreeHelper.GetParent(node))
        {
            if (node is ListViewItem { Content: MoodItem item })
            {
                return item;
            }
            if (node == MoodList)
            {
                break;
            }
        }
        return MoodList.SelectedItem as MoodItem;
    }

    private async void OnUseRow(object sender, RoutedEventArgs e)
    {
        if (sender is FrameworkElement { DataContext: MoodItem item } button)
        {
            // The button goes once the mood is current: focus moves to its row first, not to the title bar.
            var state = button is Control { FocusState: FocusState.Keyboard } ? FocusState.Keyboard : FocusState.Programmatic;
            MoodList.SelectedItem = item;
            (MoodList.ContainerFromItem(item) as Control)?.Focus(state);
            await UseAsync(item);
        }
    }

    private async Task UseAsync(MoodItem item)
    {
        if (item.IsActive)
        {
            Say(Loc.Format("Mood_AlreadyCurrent", item.Name));
            return;
        }
        try
        {
            await Model.UseMoodAsync(item.Id);
        }
        catch (Exception error)
        {
            Say(Text.Error(error, Model.Settings), error);
            await Model.MoodsEditedAsync();
        }
    }

    private async void OnNewMood(object sender, RoutedEventArgs e) => await NewMoodAsync();

    /// <summary>New mood (its button, and Ctrl+N anywhere in the window while Moods shows: MainWindow): "New mood" (or
    /// "New mood 2"), selected, its name ready to type over.</summary>
    internal async Task NewMoodAsync()
    {
        if (!Model.IsReady)
        {
            return;
        }
        try
        {
            var name = MoodText.NewName(Model.Moods.Select(mood => mood.Name));
            var created = await Model.Call(e => e.CreateMood(name, null));
            await Model.MoodsEditedAsync();
            await EditNameAsync(created.Id);
            Say(Loc.Format("Mood_Created", created.Name));
        }
        catch (Exception error)
        {
            Say(Text.Error(error, Model.Settings), error);
        }
    }

    /// <summary>Shows a mood with its name field focused and selected (New mood, Rename…, F2).</summary>
    private async Task EditNameAsync(string id)
    {
        Select(id);
        if (shownId != id)
        {
            await ShowMoodAsync(id);
        }
        ShowDetailPane(true);
        MoodName.Focus(FocusState.Keyboard);
        MoodName.SelectAll();
    }

    private MoodItem? menuMood;

    private void OnMoodMenuOpening(object? sender, object e)
    {
        menuMood = ((MenuFlyout)sender!).Target switch
        {
            ListViewItem { Content: MoodItem item } => item,
            FrameworkElement { DataContext: MoodItem item } => item,
            // A card in the summary of every mood.
            FrameworkElement { Tag: string id } => MoodItems.FirstOrDefault(item => item.Id == id),
            _ => null,
        };
        var index = menuMood is null ? -1 : MoodItems.IndexOf(menuMood);
        MoodUseItem.IsEnabled = menuMood is { IsActive: false };
        MoodMoveUpItem.IsEnabled = index > 0;
        MoodMoveDownItem.IsEnabled = index >= 0 && index < MoodItems.Count - 1;
        // There's always a mood: the last one can't be deleted.
        MoodDeleteItem.IsEnabled = MoodItems.Count > 1;
    }

    private async void OnUseFromMenu(object sender, RoutedEventArgs e)
    {
        if (menuMood is { } item)
        {
            await UseAsync(item);
        }
    }

    /// <summary>Duplicate: the mood's keywords and Surprise under "&lt;name&gt; copy", selected.</summary>
    private async void OnDuplicate(object sender, RoutedEventArgs e)
    {
        if (menuMood is not { } item)
        {
            return;
        }
        try
        {
            var name = MoodText.CopyName(item.Name, Model.Moods.Select(mood => mood.Name));
            var copy = await Model.Call(engine => engine.CreateMood(name, item.Id));
            await Model.MoodsEditedAsync();
            Select(copy.Id);
            if (MoodList.SelectedItem is { } selected && MoodList.ContainerFromItem(selected) is Control row)
            {
                row.Focus(FocusState.Keyboard);
            }
            Say(Loc.Format("Mood_Duplicated", copy.Name));
        }
        catch (Exception error)
        {
            Say(Text.Error(error, Model.Settings), error);
        }
    }

    private async void OnRenameFromMenu(object sender, RoutedEventArgs e)
    {
        if (menuMood is { } item)
        {
            await EditNameAsync(item.Id);
        }
    }

    private async void OnRenameAccelerator(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        if (FocusedMood() is { } item)
        {
            args.Handled = true;
            await EditNameAsync(item.Id);
        }
    }

    private async void OnDeleteFromMenu(object sender, RoutedEventArgs e)
    {
        if (menuMood is { } item)
        {
            await DeleteAsync(item);
        }
    }

    private async void OnDeleteThis(object sender, RoutedEventArgs e)
    {
        if (MoodItems.FirstOrDefault(item => item.Id == shownId) is { } item)
        {
            await DeleteAsync(item);
        }
    }

    /// <summary>Delete…: a confirmation (Cancel is the default), then the mood and its keywords go; its wallpapers stay
    /// in History. Focus moves to the mood that takes its place in the list.</summary>
    private async Task DeleteAsync(MoodItem item)
    {
        if (MoodItems.Count <= 1)
        {
            Say(Text.InvalidInput(InvalidInputReason.LastMood));
            return;
        }
        var dialog = new ContentDialog
        {
            Title = Loc.Format("MoodDelete_Title", item.Name),
            Content = new TextBlock { Text = Loc.Get("MoodDelete_Body"), TextWrapping = TextWrapping.Wrap },
            PrimaryButtonText = Loc.Get("MoodDelete_Confirm"),
            CloseButtonText = Loc.Get("Dialog_Cancel"),
            DefaultButton = ContentDialogButton.Close,
        };
        if (await Dialogs.ShowAsync(dialog, this) != ContentDialogResult.Primary)
        {
            return;
        }
        var index = MoodItems.IndexOf(item);
        var wasActive = item.IsActive;
        try
        {
            await Model.Call(engine => engine.DeleteMood(item.Id));
            await Model.MoodsEditedAsync();
            if (MoodItems.Count > 0)
            {
                var next = MoodItems[Math.Clamp(index, 0, MoodItems.Count - 1)];
                MoodList.SelectedItem = next;
                await ShowMoodAsync(next.Id);
                if (!wide)
                {
                    ShowDetailPane(false);
                }
                else
                {
                    MoodList.ScrollIntoView(next);
                    MoodList.UpdateLayout();
                    (MoodList.ContainerFromItem(next) as Control)?.Focus(FocusState.Keyboard);
                }
            }
            Say(wasActive && Model.ActiveMood is { } now
                ? Loc.Format("Mood_DeletedCurrent", item.Name, now.Name)
                : Loc.Format("Mood_Deleted", item.Name));
        }
        catch (Exception error)
        {
            Say(Text.Error(error, Model.Settings), error);
        }
    }

    // Order: drag, Alt+Up/Alt+Down, or the row menu.
    private async void OnMoodDragCompleted(ListViewBase sender, DragItemsCompletedEventArgs args)
    {
        if (args.Items.FirstOrDefault() is MoodItem moved)
        {
            await SaveMoodOrderAsync(moved);
        }
    }

    private async Task SaveMoodOrderAsync(MoodItem moved)
    {
        var position = MoodItems.IndexOf(moved);
        try
        {
            await Model.Call(e => e.MoveMood(moved.Id, (uint)position));
            Say(Loc.Format("Keyword_Moved", moved.Name, position + 1, MoodItems.Count));
        }
        catch (Exception error)
        {
            Say(Text.Error(error, Model.Settings), error);
        }
        await Model.MoodsEditedAsync();
    }

    private async Task MoveMoodAsync(MoodItem item, int by)
    {
        var from = MoodItems.IndexOf(item);
        var to = from + by;
        if (from < 0 || to < 0 || to >= MoodItems.Count)
        {
            return;
        }
        MoodItems.Move(from, to);
        MoodList.SelectedItem = item;
        await SaveMoodOrderAsync(item);
        if (MoodList.ContainerFromItem(item) is ListViewItem container)
        {
            container.Focus(FocusState.Keyboard);
        }
    }

    private async void OnMoodUpAccelerator(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        if (FocusedMood() is { } item)
        {
            args.Handled = true;
            await MoveMoodAsync(item, -1);
        }
    }

    private async void OnMoodDownAccelerator(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        if (FocusedMood() is { } item)
        {
            args.Handled = true;
            await MoveMoodAsync(item, +1);
        }
    }

    private async void OnMoodMoveUp(object sender, RoutedEventArgs e)
    {
        if (menuMood is { } item)
        {
            await MoveMoodAsync(item, -1);
        }
    }

    private async void OnMoodMoveDown(object sender, RoutedEventArgs e)
    {
        if (menuMood is { } item)
        {
            await MoveMoodAsync(item, +1);
        }
    }

    // ── The selected mood ───────────────────────────────────────────────────────────────────────

    /// <summary>Shows a mood in the detail: its name, Current or Use this mood, keywords, Surprise, recent wallpapers.</summary>
    private async Task ShowMoodAsync(string id)
    {
        if (pendingSurprise is not null)
        {
            surpriseSave.Stop();
            await SaveSurpriseAsync();
        }
        if (Model.Moods.FirstOrDefault(mood => mood.Id == id) is not { } mood)
        {
            return;
        }
        // A name being typed for the mood that's going (another mood was clicked, or a new one made) is saved first,
        // as leaving the field does: the field is about to show the next mood's name, so the edit would be lost.
        if (shownId != id && Shown is { } leaving && MoodText.Normalise(MoodName.Text) is var typed && typed != leaving.Name)
        {
            _ = RenameMoodAsync(leaving, typed, keepTyping: false);
        }
        var changed = shownId != id;
        shownId = id;
        MoodMakeOrStop.MoodId = id;
        SummaryPane.Visibility = Visibility.Collapsed;
        MoodPane.Visibility = Visibility.Visible;
        if (changed)
        {
            MoodScroller.ChangeView(null, 0, null, disableAnimation: true);
            ShownMoodChanged?.Invoke(this, id);
        }
        loading = true;
        try
        {
            ShowNameMessage(null);
            AddMessage.Visibility = Visibility.Collapsed;
            NewKeyword.Text = "";
            ShowHeader();
            Items.Clear();
            foreach (var keyword in mood.Keywords)
            {
                Items.Add(new KeywordItem(keyword));
            }
            UpdateEmpty();
            var percent = (int)Math.Round(mood.Surprise * 100);
            Surprise.Value = percent;
            ShowBand(percent);
        }
        finally
        {
            loading = false;
        }
        ShowMoodTiles();
        await ShowRecentAsync(id);
    }

    /// <summary>The name, Current or Use this mood, the narrow note and Delete's state, from the engine's moods.</summary>
    private void ShowHeader()
    {
        if (Shown is not { } mood)
        {
            return;
        }
        if (MoodName.FocusState == FocusState.Unfocused)
        {
            MoodName.Text = mood.Name;
        }
        if (mood.Active && UseButton.FocusState != FocusState.Unfocused)
        {
            // Use this mood is about to go: its row in the list (or the name, when the list isn't showing) gets focus.
            if (wide && MoodList.SelectedItem is { } selected && MoodList.ContainerFromItem(selected) is Control row)
            {
                row.Focus(UseButton.FocusState);
            }
            else
            {
                MoodName.Focus(UseButton.FocusState);
            }
        }
        CurrentLine.Visibility = mood.Active ? Visibility.Visible : Visibility.Collapsed;
        UseContainer.Visibility = mood.Active ? Visibility.Collapsed : Visibility.Visible;
        InUseLine.Text = Loc.Get(mood.Active ? "Mood_InUse" : "Mood_NotInUse");
        SurpriseLine.Text = MoodText.SurpriseLine(mood.Surprise);
        if (DeleteMoodButton.FocusState != FocusState.Unfocused && Model.Moods.Count <= 1)
        {
            MoodName.Focus(DeleteMoodButton.FocusState);
        }
        DeleteMoodButton.IsEnabled = Model.Moods.Count > 1;
        DeleteNote.Text = Loc.Get(Model.Moods.Count > 1 ? "Mood_DeleteNote" : "Invalid_LastMood");
        ShowNarrowNote();
    }

    /// <summary>The narrow-keywords note (spec 7) is about the mood in use.</summary>
    private void ShowNarrowNote() =>
        NarrowBar.Visibility = Shown is { Active: true } && Model.KeywordsNarrow ? Visibility.Visible : Visibility.Collapsed;

    private async void OnUseThis(object sender, RoutedEventArgs e)
    {
        if (MoodItems.FirstOrDefault(item => item.Id == shownId) is { } item)
        {
            await UseAsync(item);
        }
    }

    private void OnMoodNameKeyDown(object sender, KeyRoutedEventArgs e)
    {
        if (e.Key == VirtualKey.Enter)
        {
            e.Handled = true;
            _ = RenameMoodAsync(keepTyping: true);
        }
        else if (e.Key == VirtualKey.Escape && Shown is { } mood)
        {
            e.Handled = true;
            MoodName.Text = mood.Name;
            ShowNameMessage(null);
        }
    }

    private async void OnMoodNameLostFocus(object sender, RoutedEventArgs e) => await RenameMoodAsync(keepTyping: false);

    /// <summary>Saves the name. A name the engine refuses (empty, over 40 characters, another mood's) is said under the
    /// field; on leaving it, the saved name comes back.</summary>
    private async Task RenameMoodAsync(bool keepTyping)
    {
        if (Shown is { } mood)
        {
            await RenameMoodAsync(mood, MoodText.Normalise(MoodName.Text), keepTyping);
        }
    }

    /// <summary>Saves <paramref name="typed"/> as <paramref name="mood"/>'s name. When that mood is no longer the one
    /// shown by the time the engine answers, a refusal is said with its name, not shown under another mood.</summary>
    private async Task RenameMoodAsync(Mood mood, string typed, bool keepTyping)
    {
        var id = mood.Id;
        if (typed == mood.Name || renaming == (id, typed))
        {
            if (shownId == id && typed == mood.Name)
            {
                ShowNameMessage(null);
            }
            return;
        }
        renaming = (id, typed);
        try
        {
            var renamed = await Model.Call(e => e.RenameMood(id, typed));
            if (shownId == id)
            {
                ShowNameMessage(null);
            }
            await Model.MoodsEditedAsync();
            if (shownId == id)
            {
                MoodName.Text = renamed.Name;
            }
            Say(Loc.Format("Mood_Renamed", renamed.Name));
        }
        catch (Exception error)
        {
            var reason = Text.Error(error, Model.Settings);
            if (shownId == id)
            {
                ShowNameMessage(reason);
                Say(reason, error);
                if (!keepTyping)
                {
                    MoodName.Text = mood.Name;
                }
            }
            else
            {
                // The person has moved on to another mood (clicking it ended the edit): this one keeps its name, said
                // with its name so it isn't taken for the mood now showing.
                Say(Loc.Format("Mood_NotRenamed", mood.Name, reason), error);
            }
        }
        finally
        {
            if (renaming == (id, typed))
            {
                renaming = null;
            }
        }
    }

    /// <summary>The name field's error under it, and its description for screen readers while it shows (a person who
    /// comes back to the field hears why, not only when it was said; WCAG 1.3.1, 3.3.1). Null clears both.</summary>
    private void ShowNameMessage(string? message)
    {
        var describedBy = AutomationProperties.GetDescribedBy(MoodName);
        if (message is null)
        {
            NameMessage.Visibility = Visibility.Collapsed;
            NameMessage.Text = "";
            AutomationProperties.SetFullDescription(MoodName, "");
            describedBy.Remove(NameMessage);
            return;
        }
        NameMessage.Text = message;
        NameMessage.Visibility = Visibility.Visible;
        AutomationProperties.SetFullDescription(MoodName, message);
        if (!describedBy.Contains(NameMessage))
        {
            describedBy.Add(NameMessage);
        }
    }

    /// <summary>The mood's newest wallpapers, as thumbnails that show themselves on the desktop when clicked and have
    /// History's menu.</summary>
    private async Task ShowRecentAsync(string id)
    {
        try
        {
            var recent = await Model.Call(e => e.HistoryByMood(HistoryFilter.All, id, RecentCount, 0)
                .Select(generation => (generation, e.Describe(generation.Id), HasEchoes(e, generation.Id)))
                .ToList());
            if (shownId != id)
            {
                return;
            }
            var scale = XamlRoot?.RasterizationScale ?? 1.0;
            var focused = FocusManager.GetFocusedElement(XamlRoot) is FrameworkElement { Tag: HistoryItem had } && Recent.Children.OfType<FrameworkElement>().Any(child => child.Tag == had)
                ? (had as HistoryItem)?.Id : null;
            Recent.Children.Clear();
            foreach (var (generation, description, hasEchoes) in recent)
            {
                var item = new HistoryItem(generation, description, hasEchoes);
                var thumbnail = WallpaperActions.Thumbnail(item, this, wallpaperMenu, 128, 72);
                Recent.Children.Add(thumbnail);
                _ = item.LoadThumbnailAsync((int)(128 * scale));
                if (item.Id == focused)
                {
                    thumbnail.Loaded += (_, _) => thumbnail.Focus(FocusState.Keyboard);
                }
            }
            AutomationProperties.SetName(Recent, Loc.Format("Mood_RecentName", Shown?.Name ?? ""));
            RecentNone.Visibility = recent.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
            ShowInHistory.Visibility = recent.Count == 0 ? Visibility.Collapsed : Visibility.Visible;
        }
        catch (Exception error)
        {
            Log.Error("Reading a mood's wallpapers", error);
            Recent.Children.Clear();
            RecentNone.Visibility = Visibility.Visible;
            ShowInHistory.Visibility = Visibility.Collapsed;
        }
    }

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

    private void OnShowInHistory(object sender, RoutedEventArgs e)
    {
        if (shownId is { } id)
        {
            App.Window.ShowHistory(id);
        }
    }

    // ── What moods have made: the summary of every mood, and the shown mood's numbers ─────────────

    /// <summary>The summary of every mood in the detail (nothing selected): its totals, the chart and the cards.</summary>
    private async Task ShowSummaryAsync()
    {
        var changed = shownId is not null || SummaryPane.Visibility != Visibility.Visible;
        if (shownId is not null)
        {
            if (pendingSurprise is not null)
            {
                surpriseSave.Stop();
                await SaveSurpriseAsync();
            }
            if (Shown is { } leaving && MoodText.Normalise(MoodName.Text) is var typed && typed != leaving.Name)
            {
                _ = RenameMoodAsync(leaving, typed, keepTyping: false);
            }
        }
        shownId = null;
        MoodMakeOrStop.MoodId = null;
        MoodPane.Visibility = Visibility.Collapsed;
        SummaryPane.Visibility = Visibility.Visible;
        if (changed)
        {
            SummaryPane.ChangeView(null, 0, null, disableAnimation: true);
            ShownMoodChanged?.Invoke(this, null);
        }
        await RefreshStatsAsync();
    }

    /// <summary>Reads mood_stats (and, for the summary, activity) again, then shows them.</summary>
    private async Task RefreshStatsAsync()
    {
        if (!Model.IsReady)
        {
            return;
        }
        var summary = shownId is null;
        try
        {
            var bounds = MoodActivity.DayBounds();
            var (list, activity) = await Model.Call(e => (e.MoodStats(), summary ? e.Activity(bounds) : Array.Empty<DayCount>()));
            stats = list.GroupBy(entry => entry.MoodId).ToDictionary(group => group.Key, group => group.First());
            if (shownId is null && summary)
            {
                ShowSummary(activity);
            }
            else if (shownId is not null)
            {
                ShowMoodTiles();
            }
        }
        catch (Exception error)
        {
            Log.Error("Reading what moods have made", error);
        }
    }

    /// <summary>The summary's totals, keywords by weight, chart and cards (most wallpapers first; equal ones in the
    /// person's order).</summary>
    private void ShowSummary(DayCount[] activity)
    {
        var moods = Model.Moods;
        TotalTiles.Children.Clear();
        TotalTiles.ColumnDefinitions.Clear();
        TotalTiles.RowDefinitions.Clear();
        string[] glyphs = ["\uE790", "\uE91B", "\uE8E1", "\uE8EE"];
        var totals = MoodText.Totals(moods.Count, stats.Values.Where(entry => moods.Any(mood => mood.Id == entry.MoodId)));
        for (var index = 0; index < totals.Count; index++)
        {
            var total = totals[index];
            TotalTiles.Children.Add(StatTile.Create(total.Caption, glyphs[index], total.Value.ToString("N0", System.Globalization.CultureInfo.CurrentCulture), total.Spoken));
        }
        StatTile.Arrange(TotalTiles, TotalTiles.ActualWidth, 150);
        BreakdownLine.Text = MoodText.KeywordBreakdown(moods);
        Chart.Show(moods, activity);

        var slots = MoodActivity.ColorSlots(moods);
        MostUsed.Children.Clear();
        var ordered = moods.Select((mood, index) => (mood, index))
            .OrderByDescending(entry => stats.GetValueOrDefault(entry.mood.Id)?.Wallpapers ?? 0)
            .ThenBy(entry => entry.index)
            .Select(entry => entry.mood);
        var scale = XamlRoot?.RasterizationScale ?? 1.0;
        foreach (var mood in ordered)
        {
            MostUsed.Children.Add(MoodCard(mood, stats.GetValueOrDefault(mood.Id), Chart.ColorOf(mood.Id, slots), scale));
        }
    }

    /// <summary>One mood in the summary: a dot in its chart colour, its name (opens it), "Current mood" or Use, how many
    /// wallpapers it made, its keywords by weight, its Surprise and what it made, and its latest wallpapers (a click
    /// shows one on the desktop). Right-click, Shift+F10 or the Menu key: the mood's menu.</summary>
    private Border MoodCard(Mood mood, MoodStats? made, Windows.UI.Color color, double scale)
    {
        var card = new Border
        {
            Padding = new Thickness(16, 12, 16, 14),
            Background = WallpaperActions.Brush("CardBackgroundFillColorDefaultBrush"),
            BorderBrush = WallpaperActions.Brush("CardStrokeColorDefaultBrush"),
            BorderThickness = new Thickness(1),
            CornerRadius = (CornerRadius)Application.Current.Resources["OverlayCornerRadius"],
            Tag = mood.Id,
        };
        var stack = new StackPanel { Spacing = 6 };
        var top = new Grid { ColumnSpacing = 10 };
        top.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        top.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        top.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        top.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        top.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var dot = new Microsoft.UI.Xaml.Shapes.Ellipse
        {
            Width = 10,
            Height = 10,
            VerticalAlignment = VerticalAlignment.Center,
            Fill = new Microsoft.UI.Xaml.Media.SolidColorBrush(color),
        };
        AutomationProperties.SetAccessibilityView(dot, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        top.Children.Add(dot);
        var nameRow = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
        nameRow.Children.Add(new TextBlock { Text = mood.Name, Style = WallpaperActions.Style("BodyStrongTextBlockStyle"), TextTrimming = TextTrimming.CharacterEllipsis });
        nameRow.Children.Add(new FontIcon { Glyph = "\uE76C", FontSize = 12, VerticalAlignment = VerticalAlignment.Center });
        var open = new HyperlinkButton { Content = nameRow, Padding = new Thickness(4, 2, 4, 2), VerticalAlignment = VerticalAlignment.Center };
        AutomationProperties.SetName(open, MoodText.SpokenName(mood));
        AutomationProperties.SetHelpText(open, Loc.Get("MoodCard_OpenHelp"));
        ToolTipService.SetToolTip(open, Loc.Format("MoodCard_OpenTip", mood.Name));
        var id = mood.Id;
        open.Click += (_, _) => Select(id);
        Grid.SetColumn(open, 1);
        top.Children.Add(open);
        if (mood.Active)
        {
            var current = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6, VerticalAlignment = VerticalAlignment.Center };
            current.Children.Add(new FontIcon { Glyph = "\uE73E", FontSize = 14 });
            current.Children.Add(new TextBlock { Text = Loc.Get("MoodCurrentLine/Text"), Style = WallpaperActions.Style("BodyTextBlockStyle") });
            // Said with the name ("Rainy beach, current mood").
            AutomationProperties.SetAccessibilityView(current, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
            foreach (var child in current.Children)
            {
                AutomationProperties.SetAccessibilityView(child, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
            }
            Grid.SetColumn(current, 2);
            top.Children.Add(current);
        }
        else
        {
            var use = new Button { Content = Loc.Get("MoodUseButton/Content"), VerticalAlignment = VerticalAlignment.Center };
            AutomationProperties.SetName(use, Loc.Format("Mood_UseName", mood.Name));
            ToolTipService.SetToolTip(use, Loc.Get("MoodCard_UseTip"));
            use.Click += async (_, _) =>
            {
                if (MoodItems.FirstOrDefault(item => item.Id == id) is { } item)
                {
                    await UseAsync(item);
                }
            };
            Grid.SetColumn(use, 2);
            top.Children.Add(use);
        }
        var count = made?.Wallpapers ?? 0;
        var number = new StackPanel { HorizontalAlignment = HorizontalAlignment.Right };
        var value = new TextBlock { Text = count.ToString("N0", System.Globalization.CultureInfo.CurrentCulture), Style = WallpaperActions.Style("SubtitleTextBlockStyle"), HorizontalAlignment = HorizontalAlignment.Right };
        AutomationProperties.SetName(value, Text.Count(count, "Wallpaper"));
        var unit = new TextBlock { Text = Loc.Get(count == 1 ? "MoodCard_WallpaperUnit" : "MoodCard_WallpapersUnit"), Style = WallpaperActions.Style("CaptionTextBlockStyle"), Foreground = WallpaperActions.Brush("TextFillColorSecondaryBrush"), HorizontalAlignment = HorizontalAlignment.Right };
        AutomationProperties.SetAccessibilityView(unit, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        number.Children.Add(value);
        number.Children.Add(unit);
        Grid.SetColumn(number, 4);
        top.Children.Add(number);
        stack.Children.Add(top);

        IReadOnlyList<string> groups = MoodText.KeywordGroups(mood.Keywords) is { Count: > 0 } found ? found : [Loc.Get("Mood_NoKeywords")];
        foreach (var line in groups)
        {
            stack.Children.Add(new TextBlock { Text = line, Style = WallpaperActions.Style("BodyTextBlockStyle"), TextWrapping = TextWrapping.Wrap });
        }
        var madeLine = new TextBlock
        {
            Text = $"{MoodText.SurpriseLine(mood.Surprise)} · {MoodText.MadeLine(count, made?.Liked ?? 0, made?.LastMadeAt)}",
            Style = WallpaperActions.Style("BodyTextBlockStyle"),
            Foreground = WallpaperActions.Brush("TextFillColorSecondaryBrush"),
            TextWrapping = TextWrapping.Wrap,
        };
        AutomationProperties.SetName(madeLine, $"{MoodText.SurpriseLine(mood.Surprise)}, {MoodText.MadeLine(count, made?.Liked ?? 0, made?.LastMadeAt, ", ")}");
        stack.Children.Add(madeLine);
        if (made?.Latest is { Length: > 0 } latest)
        {
            var strip = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, Margin = new Thickness(0, 4, 0, 0) };
            AutomationProperties.SetName(strip, MoodText.SpokenLatest(latest.Select(generation => generation.Concept.Title).ToList()));
            foreach (var generation in latest)
            {
                var item = new HistoryItem(generation, generation.Concept.Title, hasEchoes: false);
                strip.Children.Add(WallpaperActions.Thumbnail(item, this, wallpaperMenu, 96, 60));
                _ = item.LoadThumbnailAsync((int)(96 * scale));
                _ = DescribeAsync(item);
            }
            stack.Children.Add(strip);
        }
        card.Child = stack;
        card.ContextFlyout = MoodMenu;
        // A click on the card (not on its controls) opens the mood, as on the Mac.
        card.Tapped += (_, args) =>
        {
            if (!IsInButton(args.OriginalSource as DependencyObject))
            {
                Select(id);
            }
        };
        return card;
    }

    /// <summary>A summary thumbnail's spoken name, from describe(id) (read once it's shown) and has_echoes.</summary>
    private async Task DescribeAsync(HistoryItem item)
    {
        try
        {
            var (description, hasEchoes) = await Model.Call(e => (e.Describe(item.Id), HasEchoes(e, item.Id)));
            item.Description = description;
            item.HasEchoes = hasEchoes;
            foreach (var button in MostUsed.Children.OfType<Border>().SelectMany(Descendants).OfType<Button>().Where(button => button.Tag == item))
            {
                AutomationProperties.SetName(button, item.SpokenName);
            }
        }
        catch (Exception)
        {
            // Spoken by its title until then.
        }
    }

    private static IEnumerable<DependencyObject> Descendants(DependencyObject root)
    {
        for (var index = 0; index < Microsoft.UI.Xaml.Media.VisualTreeHelper.GetChildrenCount(root); index++)
        {
            var child = Microsoft.UI.Xaml.Media.VisualTreeHelper.GetChild(root, index);
            yield return child;
            foreach (var deeper in Descendants(child))
            {
                yield return deeper;
            }
        }
    }

    /// <summary>The shown mood's header tiles: wallpapers, liked, echoes, last made.</summary>
    private void ShowMoodTiles()
    {
        if (Shown is not { } mood)
        {
            return;
        }
        var made = stats.GetValueOrDefault(mood.Id);
        var wallpapers = made?.Wallpapers ?? 0;
        var liked = made?.Liked ?? 0;
        var echoes = made?.Echoes ?? 0;
        var last = MoodText.LastMadeTile(made?.LastMadeAt);
        MoodTiles.Children.Clear();
        MoodTiles.ColumnDefinitions.Clear();
        MoodTiles.RowDefinitions.Clear();
        MoodTiles.Children.Add(StatTile.Create(Loc.Get("Stat_Wallpapers"), "\uE91B", wallpapers.ToString("N0", System.Globalization.CultureInfo.CurrentCulture), Text.Count(wallpapers, "Wallpaper"), compact: true));
        MoodTiles.Children.Add(StatTile.Create(Loc.Get("Stat_Liked"), "\uE8E1", liked.ToString("N0", System.Globalization.CultureInfo.CurrentCulture), Loc.Format("Mood_Liked", liked), compact: true));
        MoodTiles.Children.Add(StatTile.Create(Loc.Get("Stat_Echoes"), "\uE8EE", echoes.ToString("N0", System.Globalization.CultureInfo.CurrentCulture), Loc.Format(echoes == 1 ? "Stat_EchoesSpokenOne" : "Stat_EchoesSpoken", echoes), compact: true));
        MoodTiles.Children.Add(StatTile.Create(Loc.Get("Stat_LastMade"), "\uE823", last, Loc.Format("Stat_LastMadeSpoken", last), compact: true));
        StatTile.Arrange(MoodTiles, MoodTiles.ActualWidth, 120);
        AutomationProperties.SetName(MoodTiles, Loc.Format("Mood_MadeIn", mood.Name));
    }

    private void OnTilesSizeChanged(object sender, SizeChangedEventArgs e)
    {
        if (sender is Grid tiles)
        {
            StatTile.Arrange(tiles, e.NewSize.Width, tiles == TotalTiles ? 150 : 120);
        }
    }

    // ── IWallpaperHost: the thumbnails' actions ─────────────────────────────────────────────────

    UIElement IWallpaperHost.DialogAnchor => this;

    void IWallpaperHost.ShowStatus(string text, bool announce)
    {
        if (announce)
        {
            Model.Announce(text);
        }
    }

    void IWallpaperHost.ShowProblem(Problem problem, bool announce)
    {
        if (announce)
        {
            Model.AnnounceError(problem.Spoken);
        }
    }

    Task IWallpaperHost.DeletedAsync(HistoryItem item)
    {
        recentRefresh.Restart();
        statsRefresh.Restart();
        return Task.CompletedTask;
    }

    // ── The navigation's moods: the same menu as the list ───────────────────────────────────────

    /// <summary>A command from a mood's menu in the navigation (Use, Duplicate, Rename…, Move up/down, Delete…), done
    /// here as the list's menu does it.</summary>
    internal async Task RunMoodCommandAsync(string command, string moodId)
    {
        if (MoodItems.FirstOrDefault(item => item.Id == moodId) is not { } item)
        {
            return;
        }
        menuMood = item;
        switch (command)
        {
            case "Use":
                await UseAsync(item);
                break;
            case "Duplicate":
                OnDuplicate(this, new RoutedEventArgs());
                break;
            case "Rename":
                await EditNameAsync(item.Id);
                break;
            case "MoveUp":
                await MoveMoodAsync(item, -1);
                break;
            case "MoveDown":
                await MoveMoodAsync(item, +1);
                break;
            case "Delete":
                await DeleteAsync(item);
                break;
        }
    }

    // ── Keywords of the shown mood ──────────────────────────────────────────────────────────────

    private void UpdateEmpty() => EmptyLine.Visibility = Items.Count == 0 ? Visibility.Visible : Visibility.Collapsed;

    /// <summary>Shows a result under the add box and says it (an error interrupts; a result waits its turn).</summary>
    private void SayUnderAdd(string text, Exception? error = null)
    {
        AddMessage.Text = text;
        AddMessage.Visibility = string.IsNullOrEmpty(text) ? Visibility.Collapsed : Visibility.Visible;
        Say(text, error);
    }

    private async void OnNewKeywordKeyDown(object sender, KeyRoutedEventArgs e)
    {
        if (e.Key == VirtualKey.Enter)
        {
            e.Handled = true;
            await AddAsync();
        }
    }

    private async void OnAdd(object sender, RoutedEventArgs e) => await AddAsync();

    private async Task AddAsync()
    {
        var text = KeywordItem.Normalise(NewKeyword.Text);
        if (text.Length == 0 || shownId is not { } moodId)
        {
            return;
        }
        if (Items.FirstOrDefault(item => string.Equals(item.Text, text, StringComparison.CurrentCultureIgnoreCase)) is { } existing)
        {
            // Already there: show that row rather than changing its weight.
            List.SelectedItem = existing;
            List.ScrollIntoView(existing);
            SayUnderAdd(Loc.Format("Keyword_Duplicate", existing.Text, KeywordItem.WeightWord(existing.Weight)));
            return;
        }
        try
        {
            var added = await Model.Call(e => e.AddMoodKeyword(moodId, text, KeywordWeight.Must));
            if (shownId == moodId)
            {
                Items.Add(new KeywordItem(added));
                NewKeyword.Text = "";
                UpdateEmpty();
            }
            SayUnderAdd(Loc.Format("Keyword_Added", added.Text));
            await Model.MoodsEditedAsync();
        }
        catch (Exception error)
        {
            SayUnderAdd(Text.Error(error, Model.Settings), error);
        }
    }

    private async void OnKeywordKeyDown(object sender, KeyRoutedEventArgs e)
    {
        if (sender is TextBox box && box.DataContext is KeywordItem item)
        {
            if (e.Key == VirtualKey.Enter)
            {
                e.Handled = true;
                await RenameAsync(box, item);
            }
            else if (e.Key == VirtualKey.Escape)
            {
                e.Handled = true;
                box.Text = item.Text;
            }
        }
    }

    private async void OnKeywordLostFocus(object sender, RoutedEventArgs e)
    {
        if (sender is TextBox box && box.DataContext is KeywordItem item)
        {
            await RenameAsync(box, item);
        }
    }

    private async Task RenameAsync(TextBox box, KeywordItem item)
    {
        var text = KeywordItem.Normalise(box.Text);
        if (text == item.Text)
        {
            return;
        }
        if (text.Length == 0)
        {
            box.Text = item.Text;
            return;
        }
        try
        {
            var renamed = await Model.Call(e => e.RenameKeyword(item.Id, text));
            item.Text = renamed.Text;
            box.Text = renamed.Text;
            UpdateRowName(item);
            SayUnderAdd(Loc.Format("Keyword_Renamed", renamed.Text));
            await Model.MoodsEditedAsync();
        }
        catch (Exception error)
        {
            box.Text = item.Text;
            SayUnderAdd(Text.Error(error, Model.Settings), error);
        }
    }

    private async void OnWeightChanged(object sender, SelectionChangedEventArgs e)
    {
        if (loading || showingWeight || sender is not RadioButtons radios || radios.DataContext is not KeywordItem item || radios.SelectedIndex < 0)
        {
            return;
        }
        var index = radios.SelectedIndex;
        if (index == item.WeightIndex)
        {
            return;
        }
        var before = item.Weight;
        item.WeightIndex = index;
        try
        {
            await Model.Call(e => e.SetKeywordWeight(item.Id, item.Weight));
            UpdateRowName(item);
            await Model.MoodsEditedAsync();
        }
        catch (Exception error)
        {
            item.Weight = before;
            ShowWeight(radios, item);
            SayUnderAdd(Text.Error(error, Model.Settings), error);
        }
    }

    /// <summary>Selects the row's Must · Maybe · Avoid from its keyword, without counting as the person's choice.</summary>
    private void ShowWeight(RadioButtons radios, KeywordItem item)
    {
        if (radios.SelectedIndex == item.WeightIndex)
        {
            return;
        }
        showingWeight = true;
        try
        {
            radios.SelectedIndex = item.WeightIndex;
        }
        finally
        {
            showingWeight = false;
        }
    }

    private async void OnRemove(object sender, RoutedEventArgs e)
    {
        if (sender is FrameworkElement { DataContext: KeywordItem item })
        {
            await RemoveAsync(item);
        }
    }

    private async Task RemoveAsync(KeywordItem item)
    {
        try
        {
            var index = Items.IndexOf(item);
            await Model.Call(e => e.RemoveKeyword(item.Id));
            Items.Remove(item);
            UpdateEmpty();
            SayUnderAdd(Loc.Format("Keyword_Removed", item.Text));
            // Keep keyboard focus in the list (the removed row's neighbour), or on the add box.
            if (Items.Count > 0)
            {
                var next = Items[Math.Min(index, Items.Count - 1)];
                if (List.ContainerFromItem(next) is ListViewItem container)
                {
                    container.Focus(FocusState.Keyboard);
                }
            }
            else
            {
                NewKeyword.Focus(FocusState.Keyboard);
            }
            await Model.MoodsEditedAsync();
        }
        catch (Exception error)
        {
            SayUnderAdd(Text.Error(error, Model.Settings), error);
        }
    }

    private async void OnDragItemsCompleted(ListViewBase sender, DragItemsCompletedEventArgs args)
    {
        if (args.Items.FirstOrDefault() is KeywordItem moved)
        {
            await SaveOrderAsync(moved);
        }
    }

    private async Task SaveOrderAsync(KeywordItem moved)
    {
        var position = Items.IndexOf(moved);
        try
        {
            await Model.Call(e => e.MoveKeyword(moved.Id, (uint)position));
            SayUnderAdd(Loc.Format("Keyword_Moved", moved.Text, position + 1, Items.Count));
            await Model.MoodsEditedAsync();
        }
        catch (Exception error)
        {
            SayUnderAdd(Text.Error(error, Model.Settings), error);
            if (shownId is { } id)
            {
                await Model.MoodsEditedAsync();
                shownId = null;
                await ShowMoodAsync(id);
            }
        }
    }

    private async Task MoveAsync(KeywordItem item, int by)
    {
        var from = Items.IndexOf(item);
        var to = from + by;
        if (from < 0 || to < 0 || to >= Items.Count)
        {
            return;
        }
        Items.Move(from, to);
        await SaveOrderAsync(item);
        if (List.ContainerFromItem(item) is ListViewItem container)
        {
            container.Focus(FocusState.Keyboard);
        }
    }

    /// <summary>The keyword row with keyboard focus (or the selected one), for Alt+Up/Alt+Down.</summary>
    private KeywordItem? FocusedItem()
    {
        var focused = FocusManager.GetFocusedElement(XamlRoot) as DependencyObject;
        while (focused is not null)
        {
            if (focused is ListViewItem { Content: KeywordItem item })
            {
                return item;
            }
            if (focused is FrameworkElement { DataContext: KeywordItem context })
            {
                return context;
            }
            focused = Microsoft.UI.Xaml.Media.VisualTreeHelper.GetParent(focused);
        }
        return List.SelectedItem as KeywordItem;
    }

    private async void OnMoveUpAccelerator(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        if (FocusedItem() is { } item)
        {
            args.Handled = true;
            await MoveAsync(item, -1);
        }
    }

    private async void OnMoveDownAccelerator(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        if (FocusedItem() is { } item)
        {
            args.Handled = true;
            await MoveAsync(item, +1);
        }
    }

    private KeywordItem? menuTarget;

    private void OnRowMenuOpening(object? sender, object e)
    {
        var menu = (MenuFlyout)sender!;
        menuTarget = menu.Target switch
        {
            ListViewItem { Content: KeywordItem item } => item,
            FrameworkElement { DataContext: KeywordItem item } => item,
            _ => null,
        };
        var index = menuTarget is null ? -1 : Items.IndexOf(menuTarget);
        MoveUpItem.IsEnabled = index > 0;
        MoveDownItem.IsEnabled = index >= 0 && index < Items.Count - 1;
    }

    private async void OnMoveUp(object sender, RoutedEventArgs e)
    {
        if (menuTarget is { } item)
        {
            await MoveAsync(item, -1);
        }
    }

    private async void OnMoveDown(object sender, RoutedEventArgs e)
    {
        if (menuTarget is { } item)
        {
            await MoveAsync(item, +1);
        }
    }

    private async void OnRemoveFromMenu(object sender, RoutedEventArgs e)
    {
        if (menuTarget is { } item)
        {
            await RemoveAsync(item);
        }
    }

    /// <summary>Each row is named "rain, Must", carries the row menu, and shows its weight. The weight is set here
    /// rather than bound in the template: a binding there runs while the list lays out, and selection changes
    /// raised then reach UI Automation clients in the middle of layout.</summary>
    private void OnContainerContentChanging(ListViewBase sender, ContainerContentChangingEventArgs args)
    {
        if (args.Item is KeywordItem item)
        {
            AutomationProperties.SetName(args.ItemContainer, item.SpokenName);
            args.ItemContainer.ContextFlyout = (MenuFlyout)Resources["RowMenu"];
            if (args.ItemContainer.ContentTemplateRoot is FrameworkElement row && row.FindName("Weight") is RadioButtons radios)
            {
                ShowWeight(radios, item);
            }
        }
    }

    /// <summary>A keyword row narrower than its three parts puts Must · Maybe · Avoid under the text (Remove stays at
    /// the end of the text's line), so nothing is cut off at large text sizes or in a narrow pane.</summary>
    private void OnKeywordRowSizeChanged(object sender, SizeChangedEventArgs e)
    {
        if (sender is not Grid row || row.FindName("Weight") is not RadioButtons radios || row.Children.OfType<Button>().FirstOrDefault() is not { } remove)
        {
            return;
        }
        radios.Measure(new Windows.Foundation.Size(double.PositiveInfinity, double.PositiveInfinity));
        remove.Measure(new Windows.Foundation.Size(double.PositiveInfinity, double.PositiveInfinity));
        var stacked = e.NewSize.Width < 120 + radios.DesiredSize.Width + remove.DesiredSize.Width + 2 * row.ColumnSpacing;
        if (Grid.GetRow(radios) == (stacked ? 1 : 0))
        {
            return;
        }
        Grid.SetRow(radios, stacked ? 1 : 0);
        Grid.SetColumn(radios, stacked ? 0 : 1);
    }

    private void UpdateRowName(KeywordItem item)
    {
        if (List.ContainerFromItem(item) is ListViewItem container)
        {
            AutomationProperties.SetName(container, item.SpokenName);
        }
    }

    // ── Surprise (the shown mood's) ─────────────────────────────────────────────────────────────

    private void OnSurpriseChanged(object sender, RangeBaseValueChangedEventArgs e)
    {
        var percent = (int)Math.Round(e.NewValue);
        ShowBand(percent);
        if (!loading && shownId is { } id)
        {
            pendingSurprise = (id, (float)(percent / 100.0));
            surpriseSave.Restart();
        }
    }

    private void ShowBand(int percent)
    {
        var (name, detail) = Text.Band(percent);
        BandLine.Text = Loc.Format("Surprise_Band", percent, name);
        BandDetail.Text = detail;
        Surprise.SetBandDescription(detail);
    }

    /// <summary>set_mood_surprise for the mood it was changed on (for the mood in use, that's Settings' Surprise too).</summary>
    private async Task SaveSurpriseAsync()
    {
        if (pendingSurprise is not { } pending)
        {
            return;
        }
        pendingSurprise = null;
        try
        {
            await Model.Call(e => e.SetMoodSurprise(pending.MoodId, pending.Surprise));
            await Model.MoodsEditedAsync();
        }
        catch (Exception error)
        {
            SayUnderAdd(Text.Error(error, Model.Settings), error);
        }
    }
}
