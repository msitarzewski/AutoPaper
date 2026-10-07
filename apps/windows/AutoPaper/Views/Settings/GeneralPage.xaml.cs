using AutoPaper.Core;
using AutoPaper.Models;
using AutoPaper.Services;
using CommunityToolkit.WinUI.Controls;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Navigation;
using Windows.ApplicationModel;
using Windows.System.UserProfile;
using CoreSettings = AutoPaper.Core.Settings;

namespace AutoPaper.Views.Panes;

/// <summary>Settings › General: cadence, pause (it keeps the wallpaper showing), the person's own wallpaper and lock
/// screen (Restore my wallpaper), lock screen, replace dislikes, the fallback, sign-in, notifications.</summary>
public sealed partial class GeneralPage : Page
{
    private static readonly Cadence[] Cadences =
        [Core.Cadence.Hourly, Core.Cadence.Every3Hours, Core.Cadence.Every6Hours, Core.Cadence.Every12Hours,
         Core.Cadence.Daily, Core.Cadence.Weekly, Core.Cadence.Manual];

    private bool loading = true;

    public GeneralPage()
    {
        InitializeComponent();
        foreach (var cadence in Cadences)
        {
            CadenceBox.Items.Add(new ComboBoxItem { Content = Text.Cadence(cadence), Tag = cadence });
        }
        // The person's own wallpaper can change while the pane is open: put back from the notification area, or
        // Spotlight turned on again in Windows' Settings (the link opens it; coming back activates this window, and
        // Windows may say WM_SETTINGCHANGE).
        Loaded += (_, _) =>
        {
            Model.PropertyChanged += OnModelChanged;
            App.Window.SystemSettingChanged += OnSystemSettingChanged;
            App.Window.Activated += OnWindowActivated;
        };
        Unloaded += (_, _) =>
        {
            Model.PropertyChanged -= OnModelChanged;
            App.Window.SystemSettingChanged -= OnSystemSettingChanged;
            App.Window.Activated -= OnWindowActivated;
        };
    }

    private void OnWindowActivated(object sender, WindowActivatedEventArgs args)
    {
        if (args.WindowActivationState != WindowActivationState.Deactivated)
        {
            ShowOwnWallpaper();
            ShowLockScreen();
        }
    }

    private void OnModelChanged(object? sender, System.ComponentModel.PropertyChangedEventArgs args)
    {
        if (args.PropertyName is nameof(AppModel.DesktopShowsOwn) or nameof(AppModel.CanRestoreWallpaper) or nameof(AppModel.SpotlightLeftOff))
        {
            ShowOwnWallpaper();
        }
        else if (args.PropertyName == nameof(AppModel.LockScreenNote))
        {
            ShowLockScreen();
        }
        else if (args.PropertyName == nameof(AppModel.Settings) && Model.Settings.Paused != Paused.IsOn)
        {
            ShowSettings(); // Paused or resumed from the notification area.
        }
    }

    private void OnSystemSettingChanged(object? sender, EventArgs args)
    {
        ShowOwnWallpaper();
        ShowLockScreen();
    }

    private static AppModel Model => App.Model;

    protected override async void OnNavigatedTo(NavigationEventArgs e)
    {
        base.OnNavigatedTo(e);
        PaneHeader.Attach(Crumbs, "General", Frame);
        if (!Model.IsReady)
        {
            return;
        }
        ShowOwnWallpaper();
        ShowSettings();
        Notify.IsOn = Preferences.NotifyNewWallpapers;

        // The lock screen, where Windows lets an app set it.
        if (Desktop.LockScreenIsManaged())
        {
            LockScreen.IsOn = false;
            LockScreen.IsEnabled = false;
        }
        ShowLockScreen();

        ShowSignIn(await AutoPaper.Services.SignIn.StateAsync());
        loading = false;
    }

    /// <summary>The engine's settings in the controls: on opening, and after a change that didn't save (so a switch
    /// never shows a value that isn't the saved one).</summary>
    private void ShowSettings()
    {
        var was = loading;
        loading = true;
        var settings = Model.Settings;
        CadenceBox.SelectedIndex = Array.IndexOf(Cadences, settings.Cadence);
        Paused.IsOn = settings.Paused;
        ReplaceDisliked.IsOn = settings.ReplaceDisliked;
        FallbackBox.SelectedIndex = settings.Fallback == Core.Fallback.RevisitLiked ? 0 : 1;
        LockScreen.IsOn = settings.SetLockScreen && LockScreen.IsEnabled;
        loading = was;
    }

    private void ShowSignIn(StartupTaskState state)
    {
        SignInSwitch.IsOn = state is StartupTaskState.Enabled or StartupTaskState.EnabledByPolicy;
        SignInSwitch.IsEnabled = state is not (StartupTaskState.DisabledByPolicy or StartupTaskState.EnabledByPolicy);
        SignInCard.Description = state switch
        {
            StartupTaskState.DisabledByUser => Loc.Get("SignIn_DisabledByUser"),
            StartupTaskState.DisabledByPolicy or StartupTaskState.EnabledByPolicy => Loc.Get("SignIn_Policy"),
            _ => Loc.Get("SignIn_Description"),
        };
        StartupAppsLink.Visibility = state == StartupTaskState.DisabledByUser ? Visibility.Visible : Visibility.Collapsed;
    }

    /// <summary>
    /// Saves one change. <paramref name="change"/> runs on the thread pool, inside the engine call
    /// (<see cref="AppModel.UpdateSettingsAsync"/>), so it only uses values read here, on the UI thread, before the
    /// call: never a control's property. A change that didn't save is said on its card, and the controls go back to
    /// the saved settings.
    /// </summary>
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

    private async void OnCadenceChanged(object sender, SelectionChangedEventArgs e)
    {
        if (CadenceBox.SelectedItem is ComboBoxItem { Tag: Cadence cadence })
        {
            await SaveAsync(CadenceCard, settings => settings with { Cadence = cadence });
        }
    }

    private async void OnPausedToggled(object sender, RoutedEventArgs e)
    {
        var paused = Paused.IsOn;
        await SaveAsync(PauseCard, settings => settings with { Paused = paused });
        ShowOwnWallpaper();
    }

    /// <summary>Restore my wallpaper: enabled while AutoPaper's is showing and it kept the person's own. When the
    /// person's own was Windows Spotlight and is back as its last picture, the card says so, with the link to turn
    /// Spotlight back on.</summary>
    private void ShowOwnWallpaper()
    {
        if (!Model.IsReady)
        {
            return;
        }
        var can = Model.CanRestoreWallpaper;
        var spotlightOff = Model.SpotlightLeftOff;
        SpotlightLink.Visibility = spotlightOff ? Visibility.Visible : Visibility.Collapsed;
        foreach (var leaving in new Control[] { RestoreWallpaperButton, SpotlightLink })
        {
            var stays = leaving == RestoreWallpaperButton ? can : spotlightOff;
            if (!stays && leaving.FocusState != FocusState.Unfocused)
            {
                // About to be disabled or hidden: focus goes to what's next (the link to turn Spotlight back on, when
                // that's just appeared), else this card's neighbour, never the title bar.
                if (!(leaving == RestoreWallpaperButton && spotlightOff && FocusShown(SpotlightLink, leaving.FocusState)))
                {
                    Paused.Focus(leaving.FocusState);
                }
            }
        }
        RestoreWallpaperButton.IsEnabled = can;
        OwnWallpaperCard.Description = Loc.Get(spotlightOff ? "OwnWallpaper_SpotlightOff"
            : Model.DesktopShowsOwn ? "OwnWallpaper_Showing" : "OwnWallpaper_Description");
    }

    /// <summary>Focuses a control that was just shown (laid out first, so it can take focus).</summary>
    private bool FocusShown(Control control, FocusState state)
    {
        UpdateLayout();
        return control.Focus(state);
    }

    private async void OnRestoreWallpaper(object sender, RoutedEventArgs e)
    {
        await Model.RestoreOwnWallpaperAsync();
        ShowOwnWallpaper();
    }

    private async void OnLockScreenToggled(object sender, RoutedEventArgs e)
    {
        var setLockScreen = LockScreen.IsOn;
        await SaveAsync(LockScreenCard, settings => settings with { SetLockScreen = setLockScreen });
        ShowLockScreen();
    }

    /// <summary>
    /// The lock screen card says what AutoPaper can and can't do there, honestly: managed by the organisation; maybe
    /// not allowed on this PC; that it puts the person's own back on quit and Restore my wallpaper; that Windows
    /// Spotlight and a slideshow can only be turned on again by the person (when that's what they have now); and,
    /// after a restore that couldn't put everything back, what's left to do, with the link to Windows' lock screen
    /// settings (docs/app-spec.md 6a).
    /// </summary>
    private void ShowLockScreen()
    {
        if (!Model.IsReady)
        {
            return;
        }
        var note = Model.LockScreenNote;
        var link = Text.LockScreenLink(note);
        if (LockScreenLink.FocusState != FocusState.Unfocused && link is null)
        {
            LockScreen.Focus(LockScreenLink.FocusState);
        }
        LockScreenLink.Content = link ?? "";
        LockScreenLink.Visibility = link is null ? Visibility.Collapsed : Visibility.Visible;
        if (Desktop.LockScreenIsManaged())
        {
            LockScreenCard.Description = Loc.Get("LockScreen_ManagedSetting");
            return;
        }
        if (note != LockScreenNote.None)
        {
            LockScreenCard.Description = Text.LockScreenRestored(note);
            return;
        }
        var description = Loc.Get(UserProfilePersonalizationSettings.IsSupported() ? "LockScreen_Description" : "LockScreen_MaybeUnsupported");
        if (LockScreen.IsOn && Desktop.LockScreenKindNow() is LockScreenKind.Spotlight or LockScreenKind.Slideshow)
        {
            description += " " + Loc.Get(Desktop.LockScreenKindNow() == LockScreenKind.Spotlight ? "LockScreen_SpotlightCaveat" : "LockScreen_SlideshowCaveat");
        }
        LockScreenCard.Description = description;
    }

    private async void OnReplaceDislikedToggled(object sender, RoutedEventArgs e)
    {
        var replace = ReplaceDisliked.IsOn;
        await SaveAsync(ReplaceDislikedCard, settings => settings with { ReplaceDisliked = replace });
    }

    private async void OnFallbackChanged(object sender, SelectionChangedEventArgs e)
    {
        var fallback = FallbackBox.SelectedIndex == 1 ? Core.Fallback.KeepCurrent : Core.Fallback.RevisitLiked;
        await SaveAsync(FallbackCard, settings => settings with { Fallback = fallback });
    }

    private async void OnSignInToggled(object sender, RoutedEventArgs e)
    {
        if (loading)
        {
            return;
        }
        loading = true;
        try
        {
            ShowSignIn(await AutoPaper.Services.SignIn.SetAsync(SignInSwitch.IsOn));
        }
        finally
        {
            loading = false;
        }
    }

    private void OnNotifyToggled(object sender, RoutedEventArgs e)
    {
        if (!loading)
        {
            Preferences.NotifyNewWallpapers = Notify.IsOn;
        }
    }
}
