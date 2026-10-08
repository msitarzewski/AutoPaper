using AutoPaper.Core;
using AutoPaper.Models;
using AutoPaper.Services;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Windows.Foundation;
using Windows.UI.ViewManagement;

namespace AutoPaper.Views;

/// <summary>
/// History: every wallpaper, newest first, a page at a time, filtered (All · Liked · Disliked · Echoes, and a mood),
/// as a Grid or a Gallery (the selected one large, a filmstrip, its details and actions; remembered). Each item is
/// named by describe(id) and its rating. A click selects it; double-click or Enter shows it on the desktop (as File
/// Explorer opens an item); its context menu (right-click, Shift+F10, the Menu key) and its "More actions" button
/// offer Show on desktop, Like, Dislike, Make an echo, Show original and echoes, Show in File Explorer and Delete…
/// (WallpaperActions: the same menu as a wallpaper's anywhere else).
/// </summary>
public sealed partial class HistoryPage : Page, IWallpaperHost
{
    /// <summary>What to show when the page opens (a mood's "Show in History").</summary>
    internal sealed record Request(string? MoodId);

    /// <summary>The template's item width less its padding.</summary>
    private const double TextWidth = 240;

    private readonly UISettings uiSettings = new();
    private HistoryCollection? items;
    /// <summary>The wallpaper the Gallery shows.</summary>
    private HistoryItem? galleryItem;
    private string? galleryPicture;
    private readonly MenuFlyout itemMenu;
    private ItemLayout layout = new(40, 2, 16, 16, Orientation.Horizontal);
    /// <summary>The filter was just changed: an empty result is said, not only shown.</summary>
    private bool announceEmpty;
    /// <summary>The mood filter's choices (null = All moods), in its order.</summary>
    private readonly List<string?> moodChoices = [];
    private string? moodId;
    private bool fillingMoods;
    /// <summary>The problem the status line's link fixes.</summary>
    private Problem? statusProblem;

    public HistoryPage()
    {
        InitializeComponent();
        itemMenu = WallpaperActions.Menu(this, WallpaperActions.ItemOf);
        // The grid and the filmstrip mark Enter handled (selection): listened to anyway, for the default action.
        Wallpapers.AddHandler(KeyDownEvent, new KeyEventHandler(OnGalleryKeyDown), handledEventsToo: true);
        Filmstrip.AddHandler(KeyDownEvent, new KeyEventHandler(OnGalleryKeyDown), handledEventsToo: true);
        ViewSwitch.SelectedItem = Preferences.HistoryLayout == "Gallery" ? GalleryViewItem : GridViewItem;
        Loaded += (_, _) =>
        {
            Model.HistoryChanged += OnHistoryChanged;
            Model.PropertyChanged += OnModelChanged;
            Model.MoodsChanged += OnMoodsChanged;
            uiSettings.TextScaleFactorChanged += OnTextScaleChanged;
            FillMoods();
            Load();
        };
        Unloaded += (_, _) =>
        {
            Model.HistoryChanged -= OnHistoryChanged;
            Model.PropertyChanged -= OnModelChanged;
            Model.MoodsChanged -= OnMoodsChanged;
            uiSettings.TextScaleFactorChanged -= OnTextScaleChanged;
        };
    }

    protected override void OnNavigatedTo(Microsoft.UI.Xaml.Navigation.NavigationEventArgs e)
    {
        base.OnNavigatedTo(e);
        if (e.Parameter is Request request)
        {
            moodId = request.MoodId;
        }
    }

    /// <summary>Shows one mood's wallpapers (or every mood's), with the other filter on All.</summary>
    public void ShowMood(string? mood)
    {
        moodId = mood;
        Filter.SelectedItem = Filter.Items[0];
        FillMoods();
        Load();
    }

    /// <summary>The mood filter: All moods, then each mood by name (a deleted mood's wallpapers are under All).</summary>
    private void FillMoods()
    {
        fillingMoods = true;
        try
        {
            moodChoices.Clear();
            MoodFilter.Items.Clear();
            moodChoices.Add(null);
            MoodFilter.Items.Add(Loc.Get("History_AllMoods"));
            foreach (var mood in Model.Moods)
            {
                moodChoices.Add(mood.Id);
                MoodFilter.Items.Add(mood.Name);
            }
            if (!moodChoices.Contains(moodId))
            {
                moodId = null;
            }
            MoodFilter.SelectedIndex = moodChoices.IndexOf(moodId);
        }
        finally
        {
            fillingMoods = false;
        }
    }

    private void OnMoodsChanged(object? sender, EventArgs e)
    {
        var before = moodId;
        FillMoods();
        if (before != moodId)
        {
            Load(); // The mood shown was deleted: every mood's.
        }
    }

    private void OnMoodFilterChanged(object sender, SelectionChangedEventArgs e)
    {
        if (fillingMoods || MoodFilter.SelectedIndex < 0)
        {
            return;
        }
        var chosen = moodChoices[MoodFilter.SelectedIndex];
        if (chosen != moodId)
        {
            moodId = chosen;
            announceEmpty = true;
            Load();
        }
    }

    /// <summary>Row heights for every item at the text size in use (GridView sizes all items like the first).</summary>
    private sealed record ItemLayout(double TitleHeight, int TitleLines, double DateHeight, double BadgeHeight, Orientation Badges);

    /// <summary>Measures the item's text at the size in use: the title's lines, the longest date this format
    /// makes, and whether two badges fit side by side.</summary>
    private ItemLayout MeasureLayout()
    {
        var scale = uiSettings.TextScaleFactor;
        var titleLines = scale >= 1.5 ? 3 : 2;
        // The title shares its row with the More actions button.
        var titleHeight = TextHeight(MeasureTitle, string.Join("\n", Enumerable.Repeat("Ag", titleLines)), TextWidth - 40);
        var dateHeight = Enumerable.Range(1, 12)
            .Select(month => TextHeight(MeasureCaption, Text.WhenMade(new DateTimeOffset(2026, month, 28, 12, 59, 0, TimeSpan.Zero).ToUnixTimeSeconds()), TextWidth))
            .Max();
        var line = TextHeight(MeasureCaption, "Ag", TextWidth);
        var icon = 12 * scale + 4;
        var pair = Math.Max(TextWidthOf(MeasureCaption, Loc.Get("BadgeLiked/Text")), TextWidthOf(MeasureCaption, Loc.Get("BadgeDisliked/Text")))
            + TextWidthOf(MeasureCaption, Loc.Get("BadgeEcho/Text")) + 2 * icon + 12;
        var stacked = pair > TextWidth;
        return new ItemLayout(titleHeight, titleLines, dateHeight, stacked ? 2 * line : line, stacked ? Orientation.Vertical : Orientation.Horizontal);
    }

    private static double TextHeight(TextBlock block, string text, double width)
    {
        block.Text = text;
        block.Measure(new Size(width, double.PositiveInfinity));
        return Math.Ceiling(block.DesiredSize.Height);
    }

    private static double TextWidthOf(TextBlock block, string text)
    {
        block.Text = text;
        block.Measure(new Size(double.PositiveInfinity, double.PositiveInfinity));
        return Math.Ceiling(block.DesiredSize.Width);
    }

    /// <summary>Windows' text size changed: measure again and lay the grid out again.</summary>
    private void OnTextScaleChanged(UISettings sender, object args) => DispatcherQueue.TryEnqueue(Load);

    internal AppModel Model => App.Model;

    /// <summary>The Gallery is showing (else the Grid).</summary>
    private bool InGallery => ViewSwitch.SelectedItem == GalleryViewItem;

    /// <summary>The list the person is working in: the grid, or the Gallery's filmstrip.</summary>
    private ListViewBase ActiveList => InGallery ? Filmstrip : Wallpapers;

    private HistoryFilter SelectedFilter => (Filter.SelectedItem?.Tag as string) switch
    {
        "Liked" => HistoryFilter.Liked,
        "Disliked" => HistoryFilter.Disliked,
        "Echoes" => HistoryFilter.Echoes,
        _ => HistoryFilter.All,
    };

    private void Load()
    {
        if (!Model.IsReady)
        {
            return;
        }
        layout = MeasureLayout();
        items = new HistoryCollection(SelectedFilter, moodId);
        items.PageLoaded += (_, _) => UpdateStatus();
        ShowLayout();
    }

    /// <summary>Grid or Gallery: the list in use gets the wallpapers (the other none, so only one pages them in).</summary>
    private void ShowLayout()
    {
        var gallery = InGallery;
        var selected = (Wallpapers.SelectedItem ?? Filmstrip.SelectedItem) as HistoryItem;
        Wallpapers.Visibility = gallery ? Visibility.Collapsed : Visibility.Visible;
        GalleryView.Visibility = gallery ? Visibility.Visible : Visibility.Collapsed;
        Wallpapers.ItemsSource = gallery ? null : items;
        Filmstrip.ItemsSource = gallery ? items : null;
        if (gallery)
        {
            if (selected is not null && items?.Contains(selected) == true)
            {
                Filmstrip.SelectedItem = selected;
            }
            else
            {
                SelectFirstWhenLoaded();
            }
            ShowGalleryItem(Filmstrip.SelectedItem as HistoryItem);
        }
        else if (selected is not null && items?.Contains(selected) == true)
        {
            Wallpapers.SelectedItem = selected;
        }
    }

    /// <summary>The Gallery shows the newest wallpaper until one is chosen.</summary>
    private void SelectFirstWhenLoaded()
    {
        if (items is not { } list)
        {
            return;
        }
        if (list.Count > 0)
        {
            Filmstrip.SelectedIndex = 0;
            return;
        }
        void OnPage(object? sender, EventArgs args)
        {
            list.PageLoaded -= OnPage;
            if (ReferenceEquals(list, items) && InGallery && Filmstrip.SelectedItem is null && list.Count > 0)
            {
                Filmstrip.SelectedIndex = 0;
            }
        }
        list.PageLoaded += OnPage;
    }

    private void OnViewChanged(SelectorBar sender, SelectorBarSelectionChangedEventArgs args)
    {
        Preferences.HistoryLayout = InGallery ? "Gallery" : "Grid";
        if (items is not null)
        {
            ShowLayout();
        }
    }

    private void OnModelChanged(object? sender, System.ComponentModel.PropertyChangedEventArgs args)
    {
        if (args.PropertyName == nameof(AppModel.IsReady) && items is null)
        {
            Load();
        }
    }

    private async void OnHistoryChanged(object? sender, EventArgs e)
    {
        if (items is not null)
        {
            await RefreshKeepingFocusAsync(items);
        }
    }

    /// <summary>
    /// Brings the grid up to date in place, and keeps keyboard focus where the person expects it (WCAG 2.4.3): on the
    /// same wallpaper when it's still listed (after an echo of it, a rating), else on its neighbour at the same place
    /// (after Delete…, or a rating that took it out of the Liked or Disliked filter), else on the filter. Focus is only
    /// moved when it was on a wallpaper in the grid.
    /// </summary>
    private async Task RefreshKeepingFocusAsync(HistoryCollection list)
    {
        var (focused, state) = FocusedItem();
        var index = focused is null ? -1 : list.IndexOf(focused);
        await list.RefreshAsync();
        if (focused is null || !ReferenceEquals(list, items) || XamlRoot is null)
        {
            return;
        }
        if (list.Contains(focused))
        {
            if (FocusedItem().Item != focused)
            {
                await FocusItemAsync(focused, state);
            }
        }
        else if (list.Count > 0)
        {
            await FocusItemAsync(list[Math.Clamp(index, 0, list.Count - 1)], state);
        }
        else if (Filter.SelectedItem is { } filter)
        {
            await FocusManager.TryFocusAsync(filter, state);
        }
    }

    /// <summary>The wallpaper whose item (or its More actions button) has focus, and how it got it.</summary>
    private (HistoryItem? Item, FocusState State) FocusedItem()
    {
        if (XamlRoot is null || FocusManager.GetFocusedElement(XamlRoot) is not UIElement element)
        {
            return (null, FocusState.Unfocused);
        }
        var state = element.FocusState == FocusState.Keyboard ? FocusState.Keyboard : FocusState.Programmatic;
        for (DependencyObject? node = element; node is not null && node != ActiveList; node = VisualTreeHelper.GetParent(node))
        {
            if (node is SelectorItem { Content: HistoryItem item })
            {
                return (item, state);
            }
        }
        return (null, FocusState.Unfocused);
    }

    private async Task FocusItemAsync(HistoryItem item, FocusState state)
    {
        var list = ActiveList;
        list.ScrollIntoView(item);
        list.UpdateLayout();
        if (list.ContainerFromItem(item) is SelectorItem container)
        {
            await FocusManager.TryFocusAsync(container, state);
        }
    }

    /// <summary>The filters and the mood picker on one line when they fit, else the mood under the filters (a narrow
    /// window or larger text), so the last filter never runs into the mood's label.</summary>
    private void OnFilterRowSizeChanged(object sender, SizeChangedEventArgs e)
    {
        var unbounded = new Windows.Foundation.Size(double.PositiveInfinity, double.PositiveInfinity);
        Filter.Measure(unbounded);
        MoodFilterPanel.Measure(unbounded);
        var stacked = e.NewSize.Width < Filter.DesiredSize.Width + FilterRow.ColumnSpacing + MoodFilterPanel.DesiredSize.Width;
        if ((Grid.GetRow(MoodFilterPanel) == 1) == stacked)
        {
            return;
        }
        Grid.SetRow(MoodFilterPanel, stacked ? 1 : 0);
        Grid.SetColumn(MoodFilterPanel, stacked ? 0 : 1);
        Grid.SetColumnSpan(MoodFilterPanel, stacked ? 2 : 1);
    }

    private void OnFilterChanged(SelectorBar sender, SelectorBarSelectionChangedEventArgs args)
    {
        announceEmpty = true;
        Load();
    }

    private void UpdateStatus()
    {
        if (items is null || items.Count > 0 || items.HasMoreItems)
        {
            if (statusProblem is null)
            {
                StatusLine.Visibility = Visibility.Collapsed;
            }
            return;
        }
        var empty = Loc.Get(items.Filter switch
        {
            HistoryFilter.Liked => "History_EmptyLiked",
            HistoryFilter.Disliked => "History_EmptyDisliked",
            HistoryFilter.Echoes => "History_EmptyEchoes",
            _ when items.MoodId is not null => "History_EmptyMood",
            _ => "History_Empty",
        });
        ShowStatus(empty, announce: announceEmpty);
        announceEmpty = false;
    }

    /// <summary>Names each item for screen readers, gives it the actions menu and its full title as a tooltip (shown
    /// on keyboard focus too), sizes its rows for the text size in use, and loads its thumbnail.</summary>
    private void OnContainerContentChanging(ListViewBase sender, ContainerContentChangingEventArgs args)
    {
        if (args.Item is not HistoryItem item)
        {
            return;
        }
        AutomationProperties.SetName(args.ItemContainer, item.SpokenName);
        ToolTipService.SetToolTip(args.ItemContainer, item.Title);
        args.ItemContainer.ContextFlyout = itemMenu;
        if (args.ItemContainer.ContentTemplateRoot is Grid root && root.RowDefinitions.Count == 4)
        {
            root.RowDefinitions[1].MinHeight = layout.TitleHeight;
            root.RowDefinitions[2].MinHeight = layout.DateHeight;
            root.RowDefinitions[3].MinHeight = layout.BadgeHeight;
            if (root.FindName("ItemTitle") is TextBlock title)
            {
                title.MaxLines = layout.TitleLines;
            }
            if (root.FindName("Badges") is StackPanel badges)
            {
                badges.Orientation = layout.Badges;
                badges.Spacing = layout.Badges == Orientation.Vertical ? 0 : 12;
            }
        }
        item.PropertyChanged -= OnItemChanged;
        item.PropertyChanged += OnItemChanged;
        var scale = XamlRoot?.RasterizationScale ?? 1.0;
        _ = item.LoadThumbnailAsync((int)(248 * scale));
    }

    private void OnItemChanged(object? sender, System.ComponentModel.PropertyChangedEventArgs e)
    {
        if (sender is HistoryItem item && ActiveList.ContainerFromItem(item) is SelectorItem container)
        {
            AutomationProperties.SetName(container, item.SpokenName);
            ToolTipService.SetToolTip(container, item.Title);
        }
        if (sender == galleryItem && e.PropertyName != nameof(HistoryItem.Thumbnail))
        {
            ShowGalleryItem(galleryItem);
        }
    }

    // ── Actions ─────────────────────────────────────────────────────────────────────────────────

    /// <summary>Double-click: the default action, Show on desktop (not on the More actions button).</summary>
    private async void OnItemDoubleTapped(object sender, DoubleTappedRoutedEventArgs e)
    {
        if (!IsInButton(e.OriginalSource as DependencyObject) && (e.OriginalSource as FrameworkElement)?.DataContext is HistoryItem item)
        {
            e.Handled = true;
            await WallpaperActions.ShowOnDesktopAsync(item, this);
        }
    }

    /// <summary>Enter on an item: Show on desktop.</summary>
    private async void OnGalleryKeyDown(object sender, KeyRoutedEventArgs e)
    {
        if (e.Key == Windows.System.VirtualKey.Enter && e.OriginalSource is SelectorItem { Content: HistoryItem item })
        {
            e.Handled = true;
            await WallpaperActions.ShowOnDesktopAsync(item, this);
        }
    }

    private static bool IsInButton(DependencyObject? element)
    {
        for (var node = element; node is not null and not SelectorItem; node = VisualTreeHelper.GetParent(node))
        {
            if (node is ButtonBase)
            {
                return true;
            }
        }
        return false;
    }

    private void OnMoreActions(object sender, RoutedEventArgs e)
    {
        if (sender is FrameworkElement button)
        {
            WallpaperActions.ShowMenuAt(itemMenu, button);
        }
    }

    // ── Gallery ─────────────────────────────────────────────────────────────────────────────────

    private void OnFilmstripSelectionChanged(object sender, SelectionChangedEventArgs e) =>
        ShowGalleryItem(Filmstrip.SelectedItem as HistoryItem);

    /// <summary>The Gallery's wallpaper: large, its words, which models made it, its rating and actions.</summary>
    private void ShowGalleryItem(HistoryItem? item)
    {
        galleryItem = item;
        GalleryDetails.Visibility = item is null ? Visibility.Collapsed : Visibility.Visible;
        GalleryFrame.Visibility = item is null ? Visibility.Collapsed : Visibility.Visible;
        if (item is null)
        {
            GalleryPicture.Source = null;
            galleryPicture = null;
            return;
        }
        var generation = item.Generation;
        GalleryTitle.Text = item.Title;
        GallerySummary.Text = generation.Concept.Summary;
        GalleryImagePrompt.Text = string.IsNullOrEmpty(generation.Concept.Prompt)
            ? Loc.Get("Console_NotRecorded") : generation.Concept.Prompt;
        GalleryEcho.Visibility = item.EchoNote.Length > 0 || item.HasEchoes ? Visibility.Visible : Visibility.Collapsed;
        GalleryEchoNote.Text = item.EchoNote;
        GalleryEchoNote.Visibility = item.EchoNote.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
        GalleryLineage.Visibility = item.HasEchoes ? Visibility.Visible : Visibility.Collapsed;
        var facts = new List<string> { Loc.Format("Gallery_Made", item.When) };
        if (generation.MoodName is { Length: > 0 } mood)
        {
            facts.Add(Loc.Format("Gallery_Mood", mood));
        }
        if (MoodText.KeywordGroups(generation.Keywords) is { Count: > 0 } groups)
        {
            facts.Add(Loc.Format("Gallery_Keywords", string.Join("; ", groups)));
        }
        GalleryFacts.Text = string.Join("\n", facts);
        ShowGalleryProvenance(generation);
        GalleryLike.IsChecked = item.Rating == Rating.Liked;
        GalleryDislike.IsChecked = item.Rating == Rating.Disliked;
        var hasImage = WallpaperActions.HasImage(item);
        GalleryShow.IsEnabled = hasImage;
        GalleryReveal.IsEnabled = hasImage || generation.ThumbPath is not null;
        AutomationProperties.SetName(GalleryPicture, item.SpokenName);
        _ = ShowGalleryPictureAsync(item);
    }

    private void ShowGalleryProvenance(Generation generation)
    {
        GalleryProvenance.Inlines.Clear();
        var line = Provenance.Of(generation, Model.ModelNames);
        GalleryProvenance.Inlines.Add(new Microsoft.UI.Xaml.Documents.Run { Text = line.Head });
        var painted = new Microsoft.UI.Xaml.Documents.Hyperlink();
        painted.Inlines.Add(new Microsoft.UI.Xaml.Documents.Run { Text = line.Painted });
        AutomationProperties.SetHelpText(painted, Loc.Get("Provenance_PaintedHelp"));
        painted.Click += (_, _) => App.Window.ShowSettings("Providers");
        GalleryProvenance.Inlines.Add(painted);
        if (line.Tail.Length > 0)
        {
            GalleryProvenance.Inlines.Add(new Microsoft.UI.Xaml.Documents.Run { Text = line.Tail });
        }
        AutomationProperties.SetName(GalleryProvenance, line.Spoken);
    }

    /// <summary>The original, decoded at the size it's shown; the thumbnail when the original was pruned.</summary>
    private async Task ShowGalleryPictureAsync(HistoryItem item)
    {
        var generation = item.Generation;
        var path = generation.ImagePath is { } original && File.Exists(original) ? original : generation.ThumbPath;
        if (path is null || !File.Exists(path))
        {
            GalleryPicture.Source = null;
            galleryPicture = null;
            return;
        }
        if (path == galleryPicture)
        {
            return;
        }
        galleryPicture = path;
        try
        {
            var scale = XamlRoot?.RasterizationScale ?? 1.0;
            var width = Math.Max(320, GalleryFrame.ActualWidth * scale);
            var bitmap = new Microsoft.UI.Xaml.Media.Imaging.BitmapImage { DecodePixelWidth = (int)Math.Min(generation.Width == 0 ? 1920 : generation.Width, width) };
            using var stream = File.OpenRead(path);
            await bitmap.SetSourceAsync(stream.AsRandomAccessStream());
            if (galleryPicture == path)
            {
                GalleryPicture.Source = bitmap;
            }
        }
        catch (Exception)
        {
            GalleryPicture.Source = null; // A file another app holds, or a decode failure: the words still show.
        }
    }

    /// <summary>Narrow (or larger text): the details go under the picture and the filmstrip.</summary>
    private void OnGallerySizeChanged(object sender, SizeChangedEventArgs e)
    {
        var stacked = e.NewSize.Width < 640;
        if ((Grid.GetColumn(GalleryDetailsPane) == 0) == stacked)
        {
            return;
        }
        if (stacked)
        {
            GalleryDetailsColumn.Width = new GridLength(0);
            GalleryView.ColumnSpacing = 0;
            if (GalleryView.RowDefinitions.Count < 3)
            {
                GalleryView.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            }
            GalleryView.RowDefinitions[0].Height = new GridLength(240);
            Grid.SetColumn(GalleryDetailsPane, 0);
            Grid.SetRow(GalleryDetailsPane, 2);
            Grid.SetRowSpan(GalleryDetailsPane, 1);
        }
        else
        {
            GalleryDetailsColumn.Width = new GridLength(300);
            GalleryView.ColumnSpacing = 24;
            GalleryView.RowDefinitions[0].Height = new GridLength(1, GridUnitType.Star);
            Grid.SetColumn(GalleryDetailsPane, 1);
            Grid.SetRow(GalleryDetailsPane, 0);
            Grid.SetRowSpan(GalleryDetailsPane, 2);
        }
    }

    private async void OnGalleryShow(object sender, RoutedEventArgs e)
    {
        if (galleryItem is { } item)
        {
            await WallpaperActions.ShowOnDesktopAsync(item, this);
        }
    }

    private void OnGalleryOpenConsole(object sender, RoutedEventArgs e) => App.Window.ShowPage("Console");

    private async void OnGalleryLike(object sender, RoutedEventArgs e) => await RateGalleryAsync(Rating.Liked);

    private async void OnGalleryDislike(object sender, RoutedEventArgs e) => await RateGalleryAsync(Rating.Disliked);

    private async Task RateGalleryAsync(Rating rating)
    {
        if (galleryItem is { } item)
        {
            await Model.RateAsync(item.Id, rating);
            // The buttons show the engine's rating once History is read again (OnItemChanged); until then, as it was.
            GalleryLike.IsChecked = item.Rating == Rating.Liked;
            GalleryDislike.IsChecked = item.Rating == Rating.Disliked;
        }
    }

    private async void OnGalleryEcho(object sender, RoutedEventArgs e)
    {
        if (galleryItem is { } item)
        {
            await WallpaperActions.MakeEchoAsync(item, this);
        }
    }

    private async void OnGalleryLineage(object sender, RoutedEventArgs e)
    {
        if (galleryItem is { } item)
        {
            await WallpaperActions.ShowLineageAsync(item, this);
        }
    }

    private void OnGalleryReveal(object sender, RoutedEventArgs e)
    {
        if (galleryItem is { } item)
        {
            WallpaperActions.Reveal(item);
        }
    }

    private async void OnGalleryDelete(object sender, RoutedEventArgs e)
    {
        if (galleryItem is { } item)
        {
            await WallpaperActions.DeleteAsync(item, this);
        }
    }

    // ── IWallpaperHost ──────────────────────────────────────────────────────────────────────────

    UIElement IWallpaperHost.DialogAnchor => this;

    void IWallpaperHost.ShowStatus(string text, bool announce) => ShowStatus(text, announce);

    void IWallpaperHost.ShowProblem(Problem problem, bool announce) => ShowProblem(problem, announce);

    /// <summary>After Delete…: the list brought up to date, focus on the neighbour (the Gallery shows it).</summary>
    async Task IWallpaperHost.DeletedAsync(HistoryItem item)
    {
        if (items is not null)
        {
            var index = items.IndexOf(item);
            await RefreshKeepingFocusAsync(items);
            if (InGallery && Filmstrip.SelectedItem is null && items.Count > 0)
            {
                Filmstrip.SelectedIndex = Math.Clamp(index, 0, items.Count - 1);
            }
        }
    }

    private void ShowStatus(string text, bool announce = true)
    {
        statusProblem = null;
        StatusText.Text = text;
        StatusText.Visibility = Visibility.Visible;
        StatusLink.Visibility = Visibility.Collapsed;
        StatusLine.Visibility = string.IsNullOrEmpty(text) ? Visibility.Collapsed : Visibility.Visible;
        if (announce)
        {
            Model.Announce(text);
        }
    }

    /// <summary>A problem: its sentence and its link to the fix (docs/app-spec.md 6a).</summary>
    private void ShowProblem(Problem problem, bool announce)
    {
        statusProblem = problem.Link is null ? null : problem;
        StatusText.Text = problem.Sentence;
        StatusText.Visibility = problem.Sentence.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
        StatusLink.Content = problem.Link ?? "";
        StatusLink.Visibility = problem.Link is null ? Visibility.Collapsed : Visibility.Visible;
        StatusLine.Visibility = Visibility.Visible;
        if (announce)
        {
            Model.AnnounceError(problem.Spoken);
        }
    }

    private void OnStatusLink(object sender, RoutedEventArgs e)
    {
        if (statusProblem is { } problem)
        {
            App.Window.ShowFix(problem);
        }
    }

}
