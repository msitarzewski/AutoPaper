using AutoPaper.Core;
using AutoPaper.Models;
using AutoPaper.Services;
using AutoPaper.Views;
using AutoPaper.Views.Panes;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Animation;
using Microsoft.UI.Xaml.Navigation;
using Windows.Win32;
using Windows.Win32.Foundation;
using WinUIEx;
using WinUIEx.Messaging;
using TitleBar = Microsoft.UI.Xaml.Controls.TitleBar;

namespace AutoPaper;

/// <summary>
/// The main window: Now · Moods (a group with each mood under it) · History · Console in a NavigationView, Settings in its
/// footer (Windows' own place for it; Ctrl+,), Mica behind, the Windows App SDK TitleBar (back and pane buttons) on
/// top. Closing it hides it; AutoPaper keeps running in the notification area (spec "Always there"). Also listens for
/// display changes and session unlock.
/// </summary>
public sealed partial class MainWindow : Window
{
    private const uint WM_DISPLAYCHANGE = 0x007E;
    private const uint WM_SETTINGCHANGE = 0x001A;
    private const uint WM_WTSSESSION_CHANGE = 0x02B1;
    private const nuint WTS_SESSION_UNLOCK = 0x8;
    /// <summary>The navigation pane's usual width threshold for staying open (NavigationView's default).</summary>
    private const double ExpandedPaneThreshold = 1008;
    /// <summary>Moods shows three columns (pane, moods, mood): the pane stays open only from this width, so the mood
    /// has room at the default window size (as Mail and Outlook do with three panes).</summary>
    private const double MoodsExpandedPaneThreshold = 1300;

    private readonly WindowMessageMonitor messages;
    private readonly Microsoft.UI.Dispatching.DispatcherQueueTimer displayDebounce;
    private readonly Microsoft.UI.Dispatching.DispatcherQueueTimer statusRefresh;
    private bool active;
    private bool activatedOnce;
    /// <summary>Set while navigating: keyboard focus was in the page being left (a Settings card, a breadcrumb, an
    /// InfoBar's button), so it follows into the new page instead of falling back to the title bar.</summary>
    private FocusState? carryFocus;
    private bool carryFromTitleBar;
    /// <summary>The page shown with an inner Back level of its own (Moods), listened to for its changes.</summary>
    private IInnerBack? innerBack;
    /// <summary>The Moods page shown, listened to for the mood it shows (the navigation follows).</summary>
    private MoodsPage? moodsPage;
    /// <summary>Each mood's item under Moods in the navigation, by mood id.</summary>
    private readonly Dictionary<string, NavigationViewItem> moodItems = [];
    /// <summary>The items under Moods, in order (its MenuItemsSource: NavigationView shows changes to an observable
    /// source; items added to MenuItems after it's loaded don't appear).</summary>
    private readonly System.Collections.ObjectModel.ObservableCollection<NavigationViewItem> moodNavItems = [];

    public MainWindow()
    {
        InitializeComponent();
        ExtendsContentIntoTitleBar = true;
        SetTitleBar(AppTitleBar);
        AppWindow.SetIcon(Path.Combine(AppContext.BaseDirectory, "Assets", "AppIcon.ico"));
        AppWindow.TitleBar.PreferredHeightOption = TitleBarHeightOption.Tall;

        // First open: 1100 × 780, within the work area, centred. Afterwards WinUIEx restores the size and place the
        // person left it at (loaded when the window is first shown).
        var work = DisplayArea.GetFromWindowId(AppWindow.Id, DisplayAreaFallback.Primary).WorkArea;
        var scale = this.GetDpiForWindow() / 96.0;
        var width = (int)Math.Min(1100 * scale, work.Width * 0.94);
        var height = (int)Math.Min(780 * scale, work.Height * 0.94);
        AppWindow.MoveAndResize(new Windows.Graphics.RectInt32(work.X + (work.Width - width) / 2, work.Y + (work.Height - height) / 2, width, height));
        var manager = WindowManager.Get(this);
        manager.MinWidth = 500;
        manager.MinHeight = 520;
        manager.PersistenceId = "MainWindow";

        AppWindow.Closing += (_, args) =>
        {
            if (!App.Current.IsQuitting)
            {
                args.Cancel = true;
                AppWindow.Hide();
            }
        };
        Activated += (_, args) =>
        {
            active = args.WindowActivationState != WindowActivationState.Deactivated;
            if (active)
            {
                _ = App.Model.RefreshStatusAsync();
            }
            if (active && !activatedOnce)
            {
                // Launch: start in the page (its primary action), not on the title bar.
                activatedOnce = true;
                FocusPageIfLost();
            }
        };

        // Display changes (re-render for the new sizes) and unlock (a due wallpaper), via window messages.
        displayDebounce = DispatcherQueue.CreateTimer();
        displayDebounce.Interval = TimeSpan.FromSeconds(2);
        displayDebounce.IsRepeating = false;
        displayDebounce.Tick += async (_, _) => await App.Model.DisplaysChangedAsync();
        // A manual or paused schedule has no generation timer. Keep the budget notice current across the month
        // boundary while the window is visible, too.
        statusRefresh = DispatcherQueue.CreateTimer();
        statusRefresh.Interval = TimeSpan.FromMinutes(1);
        statusRefresh.Tick += async (_, _) =>
        {
            if (AppWindow.IsVisible)
            {
                await App.Model.RefreshStatusAsync();
            }
        };
        statusRefresh.Start();
        messages = new WindowMessageMonitor(this);
        messages.WindowMessageReceived += OnWindowMessage;
        PInvoke.WTSRegisterSessionNotification(new HWND(this.GetWindowHandle()), 0);

        App.Model.Announced += (_, announcement) => Announce(announcement);
        App.Model.PropertyChanged += (_, changed) =>
        {
            if (changed.PropertyName == nameof(AppModel.BudgetStatus))
            {
                ShowBudgetNotice();
            }
            if (changed.PropertyName == nameof(AppModel.IsReady) && App.Model.IsReady)
            {
                OnEngineReady();
                SyncMoodItems();
                // The page's primary action was disabled until now (Now's New wallpaper now): focus that was left on the
                // title bar at launch goes to it, once it's enabled.
                DispatcherQueue.TryEnqueue(Microsoft.UI.Dispatching.DispatcherQueuePriority.Low, FocusPageIfLost);
            }
        };
        App.Model.MoodsChanged += (_, _) => SyncMoodItems();
        MoodsItem.MenuItemsSource = moodNavItems;
        // Settings: Ctrl+, (the comma key has no VirtualKey name), said on the navigation's Settings item.
        var settingsKey = new KeyboardAccelerator { Key = (Windows.System.VirtualKey)0xBC, Modifiers = Windows.System.VirtualKeyModifiers.Control };
        settingsKey.Invoked += OnSettingsAccelerator;
        Root.KeyboardAccelerators.Add(settingsKey);
        Nav.Loaded += (_, _) =>
        {
            if (Nav.SettingsItem is NavigationViewItem settings)
            {
                ToolTipService.SetToolTip(settings, Loc.Get("Nav_SettingsTip"));
                Microsoft.UI.Xaml.Automation.AutomationProperties.SetAcceleratorKey(settings, Loc.Get("Nav_SettingsKeys"));
            }
        };
        // The first page is shown before the window has a XamlRoot (there's no focus to carry yet), so Navigating is
        // listened to from the second navigation on.
        ContentFrame.Navigate(typeof(NowPage), null, new SuppressNavigationTransitionInfo());
        ContentFrame.Navigating += OnNavigating;
    }

    /// <summary>The window is open and in front (a new wallpaper then needs no notification).</summary>
    public bool IsActiveWindow => active && AppWindow.IsVisible;

    /// <summary>A system setting changed (WM_SETTINGCHANGE), such as Windows' animation effects (Settings ›
    /// Accessibility › Visual effects, SPI_SETCLIENTAREAANIMATION): this reaches a desktop window even when UISettings'
    /// AnimationsEnabledChanged doesn't. Listeners read the setting they care about again.</summary>
    public event EventHandler? SystemSettingChanged;

    public void ShowWindow()
    {
        AppWindow.Show();
        this.SetForegroundWindow();
        Activate();
    }

    /// <summary>Opens a main section: Now, Moods, History or Console.</summary>
    public void ShowPage(string tag)
    {
        if (tag == "Moods")
        {
            ShowMoods(null);
            return;
        }
        ShowWindow();
        if (FirstRunFrame.Visibility == Visibility.Visible)
        {
            return;
        }
        Navigate(tag switch
        {
            "History" => typeof(HistoryPage),
            "Console" => typeof(ConsolePage),
            _ => typeof(NowPage),
        });
        FocusPageIfLost();
    }

    /// <summary>Opens Moods on one mood (a mood in the navigation, the Now view's narrow-keywords note, a problem's link,
    /// Edit moods in the menu), or on the summary of every mood (null).</summary>
    public void ShowMoods(string? moodId)
    {
        ShowWindow();
        if (FirstRunFrame.Visibility == Visibility.Visible)
        {
            return;
        }
        if (ContentFrame.Content is MoodsPage open)
        {
            open.Select(moodId);
        }
        else
        {
            ContentFrame.Navigate(typeof(MoodsPage), moodId, new EntranceNavigationTransitionInfo());
        }
        FocusPageIfLost();
    }

    /// <summary>Opens History showing one mood's wallpapers.</summary>
    public void ShowHistory(string? moodId)
    {
        ShowWindow();
        if (ContentFrame.Content is HistoryPage open)
        {
            open.ShowMood(moodId);
        }
        else
        {
            Navigate(typeof(HistoryPage), new HistoryPage.Request(moodId));
        }
        FocusPageIfLost();
    }

    /// <summary>Opens Settings on <paramref name="pane"/>, or on the pane used last (as AudioPaper's Settings do).
    /// <paramref name="focusKey"/>: Keys opens with that key's field focused (a link such as "Add your OpenAI key").</summary>
    public void ShowSettings(string? pane, string? focusKey = null)
    {
        ShowWindow();
        if (FirstRunFrame.Visibility == Visibility.Visible)
        {
            return;
        }
        OpenSettings(pane ?? Preferences.SettingsPane, focusKey);
        if (focusKey is not null && ContentFrame.Content is KeysPage keys)
        {
            keys.FocusKey(focusKey);
            return;
        }
        FocusPageIfLost();
    }

    /// <summary>Goes where a problem is fixed (docs/app-spec.md 6a): Keys with the key's field focused, Providers,
    /// Budget, or the mood in use.</summary>
    internal void ShowFix(Problem problem)
    {
        switch (problem.Fix)
        {
            case Fix.Keys:
                ShowSettings("Keys", problem.KeyAccount);
                break;
            case Fix.Providers:
                ShowSettings("Providers");
                break;
            case Fix.Budget:
                ShowSettings("Budget");
                break;
            case Fix.Moods:
                ShowMoods(problem.MoodId ?? App.Model.ActiveMood?.Id);
                break;
        }
    }

    /// <summary>Settings' home, then the pane (so its breadcrumb goes back home); nothing when the pane is open.</summary>
    private void OpenSettings(string pane, object? parameter = null)
    {
        var page = SettingsPage.PageFor(pane);
        if (page is not null && ContentFrame.CurrentSourcePageType == page)
        {
            return;
        }
        if (ContentFrame.CurrentSourcePageType != typeof(SettingsPage))
        {
            Navigate(typeof(SettingsPage));
        }
        if (page is not null)
        {
            Navigate(page, parameter);
        }
    }

    private void Navigate(Type page, object? parameter = null)
    {
        if (ContentFrame.CurrentSourcePageType != page)
        {
            ContentFrame.Navigate(page, parameter, new EntranceNavigationTransitionInfo());
        }
    }

    private void OnNavItemInvoked(NavigationView sender, NavigationViewItemInvokedEventArgs args)
    {
        if (args.IsSettingsInvoked)
        {
            OpenSettings(Preferences.SettingsPane);
            return;
        }
        if (args.InvokedItemContainer?.Tag is string tag)
        {
            if (tag.StartsWith(MoodTag, StringComparison.Ordinal))
            {
                ShowMoods(tag[MoodTag.Length..]);
            }
            else
            {
                ShowPage(tag);
            }
        }
    }

    private void OnNavigating(object sender, NavigatingCancelEventArgs args)
    {
        // Focus in the page being left (it's about to be unloaded), or on the title bar's Back button when going
        // back to the first page (it's about to be disabled): either way it would land on the title bar.
        // FocusManager needs the window's XamlRoot, which exists only once its content is loaded.
        if (Content?.XamlRoot is not { } root)
        {
            carryFocus = null;
            carryFromTitleBar = false;
            return;
        }
        var focused = FocusManager.GetFocusedElement(root) as UIElement;
        carryFromTitleBar = focused is not null && IsWithin(focused, AppTitleBar);
        var leaving = focused is not null && (IsWithin(focused, ContentFrame) || carryFromTitleBar);
        carryFocus = leaving ? (focused!.FocusState == FocusState.Keyboard ? FocusState.Keyboard : FocusState.Programmatic) : null;
    }

    private void OnNavigated(object sender, NavigationEventArgs args)
    {
        // From the title bar only when Back is about to be disabled (back on the first page); otherwise focus stays
        // on Back, for pressing it again.
        if (carryFocus is { } state && args.Content is Page page && (!carryFromTitleBar || !ContentFrame.CanGoBack))
        {
            _ = PageFocus.FocusAsync(page, state);
        }
        carryFocus = null;
        if (innerBack is not null)
        {
            innerBack.CanGoBackInsideChanged -= OnInnerBackChanged;
        }
        innerBack = args.Content as IInnerBack;
        if (innerBack is not null)
        {
            innerBack.CanGoBackInsideChanged += OnInnerBackChanged;
        }
        if (moodsPage is not null)
        {
            moodsPage.ShownMoodChanged -= OnShownMoodChanged;
        }
        moodsPage = args.Content as MoodsPage;
        if (moodsPage is not null)
        {
            moodsPage.ShownMoodChanged += OnShownMoodChanged;
        }
        UpdateBackButton();
        // Moods' three columns: the pane folds to its icons below a wider window than elsewhere.
        Nav.ExpandedModeThresholdWidth = args.SourcePageType == typeof(MoodsPage) ? MoodsExpandedPaneThreshold : ExpandedPaneThreshold;
        Nav.SelectedItem = args.SourcePageType switch
        {
            var type when type == typeof(MoodsPage) => MoodItemFor(moodsPage?.ShownMoodId ?? args.Parameter as string),
            var type when type == typeof(HistoryPage) => HistoryItem,
            var type when type == typeof(ConsolePage) => ConsoleItem,
            var type when type == typeof(NowPage) => NowItem,
            _ => Nav.SettingsItem,
        };
    }

    private void ShowBudgetNotice()
    {
        if (BudgetNoticeBar.Message == App.Model.BudgetNotice && BudgetNoticeBar.IsOpen == App.Model.BudgetBlocked)
        {
            return;
        }
        BudgetNoticeBar.Message = App.Model.BudgetNotice;
        BudgetNoticeBar.Visibility = App.Model.BudgetBlocked ? Visibility.Visible : Visibility.Collapsed;
        BudgetNoticeBar.IsOpen = App.Model.BudgetBlocked;
    }

    private void OnBudgetNoticeLink(object sender, RoutedEventArgs e) => ShowSettings("Budget");

    // ── Moods in the navigation ─────────────────────────────────────────────────────────────────

    private const string MoodTag = "Mood:";

    /// <summary>The navigation item for a mood, or Moods itself (the summary) for none.</summary>
    private NavigationViewItem MoodItemFor(string? moodId) =>
        moodId is not null && moodItems.TryGetValue(moodId, out var item) ? item : MoodsItem;

    /// <summary>Moods shows another mood (or the summary): its item is selected in the navigation.</summary>
    private void OnShownMoodChanged(object? sender, string? moodId)
    {
        if (ContentFrame.Content is MoodsPage)
        {
            Nav.SelectedItem = MoodItemFor(moodId);
        }
    }

    /// <summary>The moods under Moods in the navigation, in the person's order, the current one checkmarked ("Rainy
    /// beach, current mood" to a screen reader), each with the same menu as the Moods list. Items are updated in place
    /// (selection and focus stay).</summary>
    private void SyncMoodItems()
    {
        var moods = App.Model.Moods;
        foreach (var gone in moodItems.Keys.Where(id => moods.All(mood => mood.Id != id)).ToList())
        {
            moodNavItems.Remove(moodItems[gone]);
            moodItems.Remove(gone);
        }
        for (var index = 0; index < moods.Count; index++)
        {
            var mood = moods[index];
            if (!moodItems.TryGetValue(mood.Id, out var item))
            {
                item = new NavigationViewItem { Tag = MoodTag + mood.Id, Icon = new FontIcon { Glyph = "\uE790" } };
                item.ContextFlyout = MoodNavMenu(mood.Id);
                moodItems[mood.Id] = item;
            }
            var content = new Grid { ColumnSpacing = 8 };
            content.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            content.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            content.Children.Add(new TextBlock { Text = mood.Name, TextTrimming = TextTrimming.CharacterEllipsis, VerticalAlignment = VerticalAlignment.Center });
            if (mood.Active)
            {
                var check = new FontIcon { Glyph = "\uE73E", VerticalAlignment = VerticalAlignment.Center };
                Microsoft.UI.Xaml.Automation.AutomationProperties.SetAccessibilityView(check, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
                Grid.SetColumn(check, 1);
                content.Children.Add(check);
            }
            item.Content = content;
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(item, MoodText.SpokenName(mood));
            ToolTipService.SetToolTip(item, mood.Name);
            var at = moodNavItems.IndexOf(item);
            if (at < 0)
            {
                moodNavItems.Insert(Math.Min(index, moodNavItems.Count), item);
            }
            else if (at != index)
            {
                moodNavItems.Move(at, Math.Min(index, moodNavItems.Count - 1));
            }
        }
        if (!moodsExpanded && moodItems.Count > 0)
        {
            // Expanded by default (as the Mac's sidebar), once it has children to show.
            moodsExpanded = true;
            MoodsItem.IsExpanded = true;
        }
        if (ContentFrame.Content is MoodsPage page)
        {
            Nav.SelectedItem = MoodItemFor(page.ShownMoodId);
        }
    }

    private bool moodsExpanded;

    /// <summary>A mood's menu in the navigation: the Moods list's (Use, Duplicate, Rename…, Move up/down, Delete…). Use
    /// is done at once; the others open the mood in Moods first, where they're done.</summary>
    private MenuFlyout MoodNavMenu(string moodId)
    {
        var menu = new MenuFlyout();
        MenuNames.Name(menu, Loc.Get("Menu_MoodActions"));
        menu.Opening += (_, _) =>
        {
            menu.Items.Clear();
            var moods = App.Model.Moods;
            var index = moods.ToList().FindIndex(mood => mood.Id == moodId);
            if (index < 0)
            {
                return;
            }
            menu.Items.Add(Command("MoodMenuUse/Text", "\uE8AB", "Use", !moods[index].Active));
            menu.Items.Add(Command("MoodMenuDuplicate/Text", "\uE8C8", "Duplicate", true));
            menu.Items.Add(Command("MoodMenuRename/Text", "\uE8AC", "Rename", true));
            menu.Items.Add(new MenuFlyoutSeparator());
            menu.Items.Add(Command("MoodMenuMoveUp/Text", "\uE74A", "MoveUp", index > 0));
            menu.Items.Add(Command("MoodMenuMoveDown/Text", "\uE74B", "MoveDown", index < moods.Count - 1));
            menu.Items.Add(new MenuFlyoutSeparator());
            menu.Items.Add(Command("MoodMenuDelete/Text", "\uE74D", "Delete", moods.Count > 1));
        };
        MenuFlyoutItem Command(string key, string glyph, string command, bool enabled)
        {
            var entry = new MenuFlyoutItem { Text = Loc.Get(key), Icon = new FontIcon { Glyph = glyph }, IsEnabled = enabled };
            entry.Click += async (_, _) => await RunMoodCommandAsync(command, moodId);
            return entry;
        }
        return menu;
    }

    private async Task RunMoodCommandAsync(string command, string moodId)
    {
        if (command == "Use")
        {
            try
            {
                await App.Model.UseMoodAsync(moodId);
            }
            catch (Exception error)
            {
                App.Model.AnnounceError(Text.Error(error, App.Model.Settings));
            }
            return;
        }
        ShowMoods(moodId);
        if (ContentFrame.Content is MoodsPage page)
        {
            if (!page.IsLoaded)
            {
                var loaded = new TaskCompletionSource();
                void OnLoaded(object sender, RoutedEventArgs args)
                {
                    page.Loaded -= OnLoaded;
                    loaded.TrySetResult();
                }
                page.Loaded += OnLoaded;
                await loaded.Task;
                await Task.Yield();
            }
            await page.RunMoodCommandAsync(command, moodId);
        }
    }

    private void OnBackRequested(TitleBar sender, object args) => GoBack();

    private void OnInnerBackChanged(object? sender, EventArgs args) => UpdateBackButton();

    /// <summary>Back is enabled when the page has a level to leave (Moods' mood on a narrow window) or there's a
    /// previous page.</summary>
    private void UpdateBackButton() =>
        AppTitleBar.IsBackButtonEnabled = ContentFrame.CanGoBack || innerBack is { CanGoBackInside: true };

    private void OnPaneToggleRequested(TitleBar sender, object args) => Nav.IsPaneOpen = !Nav.IsPaneOpen;

    private void OnBackAccelerator(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args) => args.Handled = GoBack();

    /// <summary>The title bar's Back (also Alt+Left): the page's own inner level first (a mood back to the list of
    /// moods on a narrow window), else the previous page, when there is one.</summary>
    public bool GoBack()
    {
        if (FirstRunFrame.Visibility == Visibility.Visible)
        {
            return false;
        }
        if (ContentFrame.Content is IInnerBack inner && inner.GoBackInside())
        {
            return true;
        }
        if (ContentFrame.CanGoBack)
        {
            ContentFrame.GoBack();
            return true;
        }
        return false;
    }

    /// <summary>Ctrl+N while Moods is showing, wherever focus is in the window: New mood (its New mood button says
    /// Ctrl+N; docs/app-spec.md "Moods", Keyboard). New wallpaper now is Ctrl+R and F5.</summary>
    private async void OnNewMoodAccelerator(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        if (FirstRunFrame.Visibility == Visibility.Visible || ContentFrame.Content is not MoodsPage moods)
        {
            return;
        }
        args.Handled = true;
        await moods.NewMoodAsync();
    }

    /// <summary>The reload/stop on screen (Now's, or the shown mood's), or null where there's none (History, Settings).</summary>
    private Controls.MakeOrStopButton? ShownMakeOrStop() => FirstRunFrame.Visibility == Visibility.Visible ? null : ContentFrame.Content switch
    {
        NowPage now => now.MakeOrStop,
        MoodsPage moods => moods.MakeOrStop,
        _ => null,
    };

    /// <summary>Ctrl+R and F5 on Now and Moods: New wallpaper now (for a mood that isn't in use, it's made current first,
    /// as its button says). Nothing while one is being made (the button is Stop then).</summary>
    private async void OnMakeAccelerator(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        if (ContentFrame.Content is not (NowPage or MoodsPage) || FirstRunFrame.Visibility == Visibility.Visible)
        {
            return;
        }
        args.Handled = true;
        if (App.Model.IsGenerating)
        {
            return;
        }
        if (ShownMakeOrStop() is { } button)
        {
            await button.InvokeAsync();
        }
        else
        {
            await App.Model.NewWallpaperAsync(); // Moods' summary: the mood in use.
        }
    }

    /// <summary>Esc on Now and Moods while a wallpaper is being made: Stop. A text field being typed in, an open menu,
    /// list or dialog gets Esc first (it puts the text back, or closes).</summary>
    private void OnStopAccelerator(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        if (!App.Model.IsGenerating || ContentFrame.Content is not (NowPage or MoodsPage) || FirstRunFrame.Visibility == Visibility.Visible)
        {
            return;
        }
        if (Content?.XamlRoot is { } root && FocusManager.GetFocusedElement(root) is DependencyObject focused
            && (focused is TextBox or PasswordBox or AutoSuggestBox || focused is ComboBox { IsDropDownOpen: true } || !IsWithin(focused, Root)))
        {
            return;
        }
        args.Handled = true;
        App.Model.Cancel();
    }

    private void OnSettingsAccelerator(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        ShowSettings(null);
    }

    /// <summary>F1: AutoPaper's help, on its website.</summary>
    private async void OnHelpAccelerator(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        await Links.OpenAsync(Links.Help);
    }

    // ── First run ───────────────────────────────────────────────────────────────────────────────

    /// <summary>First run (spec): when no provider is set up beyond Demo and it hasn't been done or skipped.</summary>
    private void OnEngineReady()
    {
        var settings = App.Model.Settings;
        var onlyDemo = settings.TextProvider.Kind == ProviderKind.Demo && settings.ImageProvider.Kind == ProviderKind.Demo;
        if (Preferences.FirstRunDone || !onlyDemo)
        {
            Preferences.FirstRunDone = true;
            return;
        }
        App.Model.ScheduleHeld = true;
        Nav.Visibility = Visibility.Collapsed;
        AppTitleBar.IsBackButtonVisible = false;
        AppTitleBar.IsPaneToggleButtonVisible = false;
        FirstRunFrame.Visibility = Visibility.Visible;
        FirstRunFrame.Navigate(typeof(FirstRunPage), null, new SuppressNavigationTransitionInfo());
        if (FirstRunFrame.Content is Page firstRun)
        {
            _ = PageFocus.FocusAsync(firstRun, FocusState.Programmatic);
        }
    }

    /// <summary>The first-run page is done (or skipped): the normal window, on Now, and the schedule starts.
    /// <paramref name="makingOne"/>: a wallpaper is being made right away (the schedule then counts from it).</summary>
    public void FinishFirstRun(bool makingOne)
    {
        Preferences.FirstRunDone = true;
        App.Model.ScheduleHeld = false;
        if (!makingOne)
        {
            _ = App.Model.CheckScheduleAsync();
        }
        FirstRunFrame.Visibility = Visibility.Collapsed;
        FirstRunFrame.Content = null;
        Nav.Visibility = Visibility.Visible;
        AppTitleBar.IsBackButtonVisible = true;
        AppTitleBar.IsPaneToggleButtonVisible = true;
        Navigate(typeof(NowPage));
        // The first-run page took focus with it: Now's primary action (or Cancel, when its wallpaper is under way).
        if (ContentFrame.Content is Page now)
        {
            _ = PageFocus.FocusAsync(now, FocusState.Programmatic);
        }
    }

    /// <summary>When nothing in the window has focus, or only the title bar does (launch, a page opened from the
    /// notification area), focus goes to the page's default control.</summary>
    private void FocusPageIfLost()
    {
        var page = (FirstRunFrame.Visibility == Visibility.Visible ? FirstRunFrame.Content : ContentFrame.Content) as Page;
        if (page is null || Content?.XamlRoot is not { } root)
        {
            return;
        }
        var focused = FocusManager.GetFocusedElement(root) as DependencyObject;
        if (focused is null || IsWithin(focused, AppTitleBar) || focused == Root)
        {
            _ = PageFocus.FocusAsync(page, FocusState.Programmatic);
        }
    }

    private static bool IsWithin(DependencyObject element, DependencyObject ancestor)
    {
        for (var node = element; node is not null; node = VisualTreeHelper.GetParent(node))
        {
            if (node == ancestor)
            {
                return true;
            }
        }
        return false;
    }

    // ── Accessibility and system messages ───────────────────────────────────────────────────────

    /// <summary>
    /// Says a sentence to screen readers (progress stages, results, errors) while the window is open, as a UI
    /// Automation notification. It's raised from the title bar: an element that is always in the UI Automation tree,
    /// on every page and on the first-run page (a notification raised from an element with no peer in the tree, such
    /// as a Grid, never reaches clients). Stages and results are polite (what's being said finishes; a newer one
    /// replaces any still waiting); errors interrupt. The Now view's problem and notice InfoBars announce themselves
    /// when they open, so those are only spoken here when Now isn't showing.
    /// </summary>
    private void Announce(Announcement announcement)
    {
        if (!AppWindow.IsVisible)
        {
            return;
        }
        var nowShowing = FirstRunFrame.Visibility == Visibility.Collapsed && ContentFrame.Content is NowPage;
        if (nowShowing && announcement.Kind is AnnouncementKind.NowProblem or AnnouncementKind.NowNotice)
        {
            return;
        }
        var important = announcement.Kind is AnnouncementKind.Error or AnnouncementKind.NowProblem;
        var peer = FrameworkElementAutomationPeer.FromElement(AppTitleBar) ?? FrameworkElementAutomationPeer.CreatePeerForElement(AppTitleBar);
        peer?.RaiseNotificationEvent(
            AutomationNotificationKind.Other,
            important ? AutomationNotificationProcessing.ImportantMostRecent : AutomationNotificationProcessing.CurrentThenMostRecent,
            announcement.Text,
            important ? "AutoPaperProblem" : "AutoPaperStatus");
    }

    private void OnWindowMessage(object? sender, WindowMessageEventArgs args)
    {
        switch (args.Message.MessageId)
        {
            case WM_DISPLAYCHANGE:
                // Coalesce the burst Windows sends while monitors rearrange.
                displayDebounce.Stop();
                displayDebounce.Start();
                break;
            case WM_WTSSESSION_CHANGE when args.Message.WParam == WTS_SESSION_UNLOCK:
                DispatcherQueue.TryEnqueue(async () => await App.Model.CheckScheduleAsync());
                break;
            case WM_SETTINGCHANGE:
                DispatcherQueue.TryEnqueue(() => SystemSettingChanged?.Invoke(this, EventArgs.Empty));
                break;
        }
    }
}
