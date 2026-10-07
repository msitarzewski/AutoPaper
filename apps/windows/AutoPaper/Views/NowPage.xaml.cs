using System.ComponentModel;
using AutoPaper.Core;
using AutoPaper.Models;
using AutoPaper.Services;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media.Animation;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.UI.ViewManagement;
using Windows.Win32;
using Windows.Win32.Foundation;
using Windows.Win32.UI.WindowsAndMessaging;

namespace AutoPaper.Views;

/// <summary>Now: the wallpaper showing, its words and which models made it, rating, New wallpaper now / Stop in the
/// command bar, progress (a ring, the stage and the time left, and the picture veiled while the next one is made) and
/// status lines, and problems and notices at the top, each problem with a link to its fix.</summary>
public sealed partial class NowPage : Page, IDefaultFocus
{
    private readonly UISettings uiSettings = new();
    private string? shownPath;
    private bool veiled;

    public NowPage()
    {
        InitializeComponent();
        Loaded += (_, _) =>
        {
            Model.PropertyChanged += OnModelChanged;
            uiSettings.AnimationsEnabledChanged += OnAnimationsChanged;
            App.Window.SystemSettingChanged += OnSystemSettingChanged;
            _ = ShowPictureAsync();
            ShowProvenance();
            ShowMaking();
            ShowRing();
            DescribeRing();
            ShowEmptyState();
            ShowBar(StartupBar, Model.StartupProblem is not null);
            ShowBar(ProblemBar, Model.HasProblem);
            ShowBar(NoticeBar, Model.HasNotice);
        };
        Unloaded += (_, _) =>
        {
            Model.PropertyChanged -= OnModelChanged;
            uiSettings.AnimationsEnabledChanged -= OnAnimationsChanged;
            App.Window.SystemSettingChanged -= OnSystemSettingChanged;
        };
    }

    /// <summary>The primary action: New wallpaper now, or Stop while one is being made (one button).</summary>
    public UIElement? DefaultFocusElement => NowMakeOrStop.IsEnabled ? NowMakeOrStop : null;

    /// <summary>The command bar's reload/stop (Ctrl+R, F5 and Esc act on it while Now shows).</summary>
    internal Controls.MakeOrStopButton MakeOrStop => NowMakeOrStop;

    internal AppModel Model => App.Model;

    internal string TitleOf(Generation? generation) => generation?.Concept.Title ?? "";

    internal string SummaryOf(Generation? generation) => generation?.Concept.Summary ?? "";

    internal string EchoNoteOf(Generation? generation) => generation?.EchoNote ?? "";

    internal Visibility EchoNoteVisibility(Generation? generation) =>
        string.IsNullOrEmpty(generation?.EchoNote) ? Visibility.Collapsed : Visibility.Visible;

    private void OnModelChanged(object? sender, PropertyChangedEventArgs args)
    {
        switch (args.PropertyName)
        {
            case nameof(AppModel.Current):
                _ = ShowPictureAsync();
                ShowProvenance();
                ShowVeil();
                break;
            case nameof(AppModel.ModelNames):
                ShowProvenance();
                break;
            case nameof(AppModel.NextText):
                RaiseLiveRegion(NextLine);
                break;
            case nameof(AppModel.IsGenerating) or nameof(AppModel.IsReady):
                ShowMaking();
                break;
            case nameof(AppModel.ProgressFraction):
                ShowRing();
                FollowMotionSetting();
                break;
            case nameof(AppModel.StageText) or nameof(AppModel.TimeLeftText):
                DescribeRing();
                FollowMotionSetting();
                break;
            case nameof(AppModel.DesktopKeepsPicture):
                ShowEmptyState();
                break;
            case nameof(AppModel.StartupProblem):
                ShowBar(StartupBar, Model.StartupProblem is not null);
                break;
            case nameof(AppModel.HasProblem):
                ShowBar(ProblemBar, Model.HasProblem);
                break;
            case nameof(AppModel.HasNotice):
                ShowBar(NoticeBar, Model.HasNotice);
                break;
        }
    }

    /// <summary>
    /// The stage, ring and time left while one is being made. New wallpaper now and Stop are one button in the command
    /// bar that changes in place, so keyboard focus stays on it (WCAG 2.4.3).
    /// </summary>
    private void ShowMaking()
    {
        MakingPanel.Visibility = Model.IsGenerating ? Visibility.Visible : Visibility.Collapsed;
        ShowVeil();
    }

    /// <summary>Which models made the wallpaper: "Written by … · Painted by … · size · cost", Painted by a link to
    /// Settings › Providers (where painting is chosen). A screen reader reads the line, then the link.</summary>
    private void ShowProvenance()
    {
        ProvenanceLine.Inlines.Clear();
        if (Model.Current is not { } current)
        {
            ProvenanceLine.Visibility = Visibility.Collapsed;
            return;
        }
        var line = Provenance.Of(current, Model.ModelNames);
        ProvenanceLine.Inlines.Add(new Microsoft.UI.Xaml.Documents.Run { Text = line.Head });
        var painted = new Microsoft.UI.Xaml.Documents.Hyperlink();
        painted.Inlines.Add(new Microsoft.UI.Xaml.Documents.Run { Text = line.Painted });
        AutomationProperties.SetHelpText(painted, Loc.Get("Provenance_PaintedHelp"));
        painted.Click += (_, _) => App.Window.ShowSettings("Providers");
        ProvenanceLine.Inlines.Add(painted);
        if (line.Tail.Length > 0)
        {
            ProvenanceLine.Inlines.Add(new Microsoft.UI.Xaml.Documents.Run { Text = line.Tail });
        }
        AutomationProperties.SetName(ProvenanceLine, line.Spoken);
        ProvenanceLine.Visibility = Visibility.Visible;
    }

    /// <summary>The ring: determinate with the painting's fraction when the engine has one, else spinning.</summary>
    private void ShowRing()
    {
        Ring.IsIndeterminate = !Model.ProgressKnown;
        Ring.Value = Model.ProgressPercent;
    }

    /// <summary>The ring's help text is the stage and the time left ("Painting…, about 15 seconds left"), so a screen
    /// reader reading the ring hears them with its percentage. Not a live region: the stage is announced once by the
    /// window, and the time left isn't announced as it counts down.</summary>
    private void DescribeRing() =>
        AutomationProperties.SetHelpText(Ring, Text.StageWithTimeLeft(Model.StageText, Model.SecondsLeft));

    /// <summary>
    /// The picture showing is veiled while the next one is made ("being remade"): in-app acrylic over it, breathing
    /// slowly. When Windows' animation effects are off (Settings › Accessibility › Visual effects) the veil holds
    /// still; the Storyboard changes only opacity, so it costs the compositor almost nothing.
    /// </summary>
    private void ShowVeil()
    {
        var show = Model.IsGenerating && Model.HasCurrent;
        var breathing = (Storyboard)Resources["Breathing"];
        var fadeOut = (Storyboard)Resources["VeilOut"];
        if (show)
        {
            fadeOut.Stop();
            Veil.Visibility = Visibility.Visible;
            if (AnimationsOn())
            {
                if (!veiled || breathing.GetCurrentState() != ClockState.Active)
                {
                    breathing.Begin();
                }
            }
            else
            {
                breathing.Stop();
                Veil.Opacity = 0.6;
            }
            veiled = true;
        }
        else if (veiled)
        {
            veiled = false;
            breathing.Stop();
            if (AnimationsOn())
            {
                Veil.Opacity = 0.6;
                fadeOut.Completed -= OnVeilFaded;
                fadeOut.Completed += OnVeilFaded;
                fadeOut.Begin();
            }
            else
            {
                Veil.Opacity = 0;
                Veil.Visibility = Visibility.Collapsed;
            }
        }
    }

    private void OnVeilFaded(object? sender, object e)
    {
        if (!veiled)
        {
            Veil.Visibility = Visibility.Collapsed;
        }
    }

    /// <summary>Windows' animation effects, read from the system each time (SPI_GETCLIENTAREAANIMATION, the setting
    /// UISettings.AnimationsEnabled reports): a value cached by UISettings could still say on just after it's turned
    /// off.</summary>
    private static unsafe bool AnimationsOn()
    {
        BOOL on = true;
        return !PInvoke.SystemParametersInfo((SYSTEM_PARAMETERS_INFO_ACTION)0x1042 /* SPI_GETCLIENTAREAANIMATION */, 0, &on, 0) || on;
    }

    /// <summary>Animation effects turned on or off while the page is open: the veil follows at once. Windows says so
    /// with WM_SETTINGCHANGE (MainWindow); UISettings' event is listened to as well (it's raised off the UI thread).</summary>
    private void OnAnimationsChanged(UISettings sender, UISettingsAnimationsEnabledChangedEventArgs args) =>
        DispatcherQueue.TryEnqueue(FollowMotionSetting);

    private void OnSystemSettingChanged(object? sender, EventArgs args) => FollowMotionSetting();

    /// <summary>While veiled: the veil breathes or holds still as the setting is now (also looked at with every stage
    /// and progress report, so a change that came without a message is followed within a moment).</summary>
    private void FollowMotionSetting()
    {
        if (veiled)
        {
            ShowVeil();
        }
    }

    /// <summary>Nothing showing: "No wallpaper yet", or, after Clear history, that the desktop keeps its picture.</summary>
    private void ShowEmptyState()
    {
        if (Model.DesktopKeepsPicture)
        {
            EmptyTitle.Text = Loc.Get("Now_ClearedTitle");
            EmptyBody.Text = Loc.Get("Now_ClearedBody");
        }
        else
        {
            EmptyTitle.Text = Loc.Get("NowEmptyTitle/Text");
            EmptyBody.Text = Loc.Get("NowEmptyBody/Text");
        }
    }

    /// <summary>Opens or closes one of the bars at the top. Shown before it opens (so its own announcement reaches
    /// screen readers) and brought into view; hidden once closed, so it leaves no gap.</summary>
    private void ShowBar(InfoBar bar, bool open)
    {
        if (open)
        {
            Bars.Visibility = Visibility.Visible;
            bar.Visibility = Visibility.Visible;
            bar.IsOpen = true;
            bar.StartBringIntoView();
        }
        else
        {
            bar.IsOpen = false;
            HideClosed(bar);
        }
    }

    private void OnBarClosed(InfoBar sender, InfoBarClosedEventArgs args) => HideClosed(sender);

    private void HideClosed(InfoBar bar)
    {
        bar.Visibility = Visibility.Collapsed;
        if (StartupBar.Visibility == Visibility.Collapsed && ProblemBar.Visibility == Visibility.Collapsed && NoticeBar.Visibility == Visibility.Collapsed)
        {
            Bars.Visibility = Visibility.Collapsed;
        }
    }

    /// <summary>The original, decoded at the size it's shown (sharper than the 640 px thumbnail on a large
    /// window); the thumbnail when the original was pruned.</summary>
    private async Task ShowPictureAsync()
    {
        var current = Model.Current;
        var path = current?.ImagePath is { } original && File.Exists(original) ? original : current?.ThumbPath;
        if (path is null || !File.Exists(path))
        {
            Picture.Source = null;
            shownPath = null;
            return;
        }
        if (path == shownPath)
        {
            return;
        }
        shownPath = path;
        try
        {
            var scale = XamlRoot?.RasterizationScale ?? 1.0;
            var bitmap = new BitmapImage { DecodePixelWidth = (int)Math.Min(current!.Width == 0 ? 1920 : current.Width, 1100 * scale) };
            using var stream = File.OpenRead(path);
            await bitmap.SetSourceAsync(stream.AsRandomAccessStream());
            if (shownPath == path)
            {
                Picture.Source = bitmap;
            }
        }
        catch (Exception)
        {
            Picture.Source = null; // A file another app holds or a decode failure: the words still show.
        }
    }

    private static void RaiseLiveRegion(UIElement element) =>
        FrameworkElementAutomationPeer.FromElement(element)?.RaiseAutomationEvent(AutomationEvents.LiveRegionChanged);

    private async void OnLike(object sender, RoutedEventArgs e) => await RateAsync(Rating.Liked);

    private async void OnDislike(object sender, RoutedEventArgs e) => await RateAsync(Rating.Disliked);

    private async Task RateAsync(Rating rating)
    {
        if (Model.Current is { } current)
        {
            await Model.RateAsync(current.Id, rating);
        }
        // The buttons show the engine's rating, whatever the click toggled.
        LikeButton.IsChecked = Model.IsLiked;
        DislikeButton.IsChecked = Model.IsDisliked;
    }

    private void OnFixProblem(object sender, RoutedEventArgs e)
    {
        if (Model.Problem is { } problem)
        {
            App.Window.ShowFix(problem);
        }
    }

    private void OnOpenMoods(object sender, RoutedEventArgs e) => App.Window.ShowMoods(Model.ActiveMood?.Id);
}
