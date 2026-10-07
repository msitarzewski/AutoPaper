using System.Diagnostics;
using AutoPaper.Core;
using AutoPaper.Models;
using AutoPaper.Services;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.Windows.AppLifecycle;
using Microsoft.Windows.AppNotifications;
using WinUIEx;

namespace AutoPaper;

/// <summary>
/// AutoPaper lives in the notification area: the main window can close and the app keeps making wallpapers.
/// Start-up order follows the app-notification quickstart: create the window (not shown), register for
/// notification activation, then read how this launch was activated (a normal launch shows the window; a
/// sign-in start stays in the notification area; a notification button does what it says).
/// </summary>
public partial class App : Application
{
    private readonly DispatcherQueue ui;
    private TrayIcon? tray;
    private bool quitting;
    /// <summary>The window, the notification-area icon and the model exist (OnLaunched got through).</summary>
    private bool started;

    public App()
    {
        InitializeComponent();
        // The strings code builds come from the package's resources (MRT); set before any page or message exists.
        var strings = new Microsoft.Windows.ApplicationModel.Resources.ResourceLoader();
        // A key MRT doesn't have throws (NamedResource Not Found): Loc then shows the key, never fails the page.
        Loc.Source = key =>
        {
            try
            {
                return strings.GetString(key);
            }
            catch (Exception)
            {
                return "";
            }
        };
        // Set here (on the UI thread, inside Application.Start) so a launch redirected to this instance can be queued
        // even before OnLaunched runs.
        ui = DispatcherQueue.GetForCurrentThread();
        UnhandledException += (_, args) =>
        {
            Log.Error("Unhandled", args.Exception);
            if (!started)
            {
                // Before the window, the notification-area icon and the engine exist, nothing can work and nothing
                // would show it: say so and exit, rather than leave an invisible process holding the single-instance
                // key (a second launch would be redirected to it).
                args.Handled = true;
                FailedToStart();
                return;
            }
#if DEBUG
            // Development builds stop here, so a failing binding, layout or handler is found rather than hidden.
            args.Handled = false;
#else
            // Release: a tray app shouldn't vanish over one failed handler; the failure is logged for support.
            args.Handled = true;
#endif
        };
    }

    internal static new App Current => (App)Application.Current;

    internal static AppModel Model { get; private set; } = null!;

    internal static MainWindow Window { get; private set; } = null!;

    protected override async void OnLaunched(LaunchActivatedEventArgs args)
    {
        try
        {
            Model = new AppModel(ui);
            Window = new MainWindow();
            Model.WindowIsActive = () => Window.IsActiveWindow;

            Notifications.Register(arguments => ui.TryEnqueue(() => OnNotification(arguments)));
            var activation = AppInstance.GetCurrent().GetActivatedEventArgs();
            CreateTray();

            switch (activation.Kind)
            {
                case ExtendedActivationKind.StartupTask:
                    break; // Signed in: stay in the notification area.
                case ExtendedActivationKind.AppNotification:
                    OnNotification(((AppNotificationActivatedEventArgs)activation.Data).Arguments);
                    break;
                default:
                    Window.ShowWindow();
                    break;
            }
            started = true;
        }
        catch (Exception error)
        {
            Log.Error("Start-up", error);
            FailedToStart();
            return;
        }
        // The engine's own failures are shown in the window (AppModel.StartupProblem), not here.
        await Model.OpenAsync();
    }

    /// <summary>Start-up failed before AutoPaper could show anything (already logged): a message (the window may not
    /// exist, so it's Windows' own message box), then exit, so the single-instance key is free for the next launch.</summary>
    private void FailedToStart()
    {
        if (quitting)
        {
            return;
        }
        quitting = true;
        var log = Path.Combine(Windows.Storage.ApplicationData.Current.LocalFolder.Path, "logs", "autopaper.log");
        _ = Windows.Win32.PInvoke.MessageBox(
            Windows.Win32.Foundation.HWND.Null,
            Loc.Format("Fatal_Body", log),
            Loc.Get("Fatal_Title"),
            Windows.Win32.UI.WindowsAndMessaging.MESSAGEBOX_STYLE.MB_OK | Windows.Win32.UI.WindowsAndMessaging.MESSAGEBOX_STYLE.MB_ICONERROR);
        try
        {
            trayKeyboard?.Dispose();
            tray?.Dispose();
            tray = null;
            Notifications.Unregister();
            Model?.Close();
        }
        catch (Exception cleanup)
        {
            Log.Error("Start-up clean-up", cleanup);
        }
        Exit();
    }

    /// <summary>Another launch was redirected here (Program): on the AppInstance's thread, so marshal. Ignored if
    /// start-up hasn't made the window (it failed, and the process is exiting).</summary>
    internal void OnRedirectedActivation(AppActivationArguments activation) => ui.TryEnqueue(() =>
    {
        if (!started)
        {
            return;
        }
        switch (activation.Kind)
        {
            case ExtendedActivationKind.AppNotification:
                OnNotification(((AppNotificationActivatedEventArgs)activation.Data).Arguments);
                break;
            case ExtendedActivationKind.StartupTask:
                break;
            default:
                Window.ShowWindow();
                break;
        }
    });

    private void OnNotification(IDictionary<string, string> arguments)
    {
        if (Window is null || Model is null)
        {
            return;
        }
        arguments.TryGetValue("action", out var action);
        arguments.TryGetValue("id", out var id);
        switch (action)
        {
            case "like" when id is not null:
                Notifications.ClearNewWallpaper();
                _ = Model.WhenReadyAsync(() => Model.RateAsync(id, Rating.Liked, toggle: false));
                break;
            case "dislike" when id is not null:
                Notifications.ClearNewWallpaper();
                _ = Model.WhenReadyAsync(() => Model.RateAsync(id, Rating.Disliked, toggle: false));
                break;
            case "budget":
                Window.ShowSettings("Budget");
                break;
            case "keys":
                arguments.TryGetValue("account", out var account);
                Window.ShowSettings("Keys", string.IsNullOrEmpty(account) ? null : account);
                break;
            default:
                Window.ShowPage("Now");
                break;
        }
    }

    private void CreateTray()
    {
        tray = new TrayIcon(1, Path.Combine(AppContext.BaseDirectory, "Assets", "AppIcon.ico"), "AutoPaper");
        tray.Selected += (_, _) => Window?.ShowWindow();
        tray.ContextMenu += (_, args) => args.Flyout = TrayMenu.Build(Model);
        tray.IsVisible = true;
        ListenForKeyboardSelect();
        Model.PropertyChanged += (_, changed) =>
        {
            if (changed.PropertyName is nameof(AppModel.Current) or nameof(AppModel.StageText) or nameof(AppModel.IsGenerating) or nameof(AppModel.TimeLeftText))
            {
                UpdateTooltip();
            }
        };
    }

    // WinUIEx 2.9.3's TrayIcon raises Selected for a click (NIN_SELECT) but not for Enter or Space on the focused
    // icon (NIN_KEYSELECT), so keyboard users couldn't open the window from the icon itself (Shift+F10 and the Menu key
    // do open its menu). Its hidden window receives the icon's callback message too; listening there closes the gap.
    private const uint TrayCallbackMessage = 0x8765; // WinUIEx TrayIcon.TrayIconCallbackId (private; 2.9.3, pinned)
    private const uint NinKeySelect = 0x0401;        // WM_USER + 1
    private WinUIEx.Messaging.WindowMessageMonitor? trayKeyboard;

    private void ListenForKeyboardSelect()
    {
        var main = Window.GetWindowHandle();
        var trayWindow = TrayWindows.Find(main);
        if (trayWindow == IntPtr.Zero)
        {
            return;
        }
        trayKeyboard = new WinUIEx.Messaging.WindowMessageMonitor(trayWindow);
        trayKeyboard.WindowMessageReceived += (_, args) =>
        {
            var lParam = (long)args.Message.LParam;
            if (args.Message.MessageId == TrayCallbackMessage && (lParam & 0xFFFF) == NinKeySelect && ((lParam >> 16) & 0xFFFF) == 1)
            {
                ui.TryEnqueue(() => Window?.ShowWindow());
            }
        };
    }

    private void UpdateTooltip()
    {
        if (tray is null)
        {
            return;
        }
        var text = Model.IsGenerating && Model.StageText.Length > 0
            ? $"AutoPaper: {Text.StageWithTimeLeft(Model.StageText, Model.SecondsLeft)}"
            : Model.Current is { } current ? $"AutoPaper: {current.Concept.Title}" : "AutoPaper";
        tray.Tooltip = text.Length > 127 ? text[..126] + "…" : text;
    }

    internal bool IsQuitting => quitting;

    /// <summary>Quit AutoPaper: the only way out (closing the window keeps it in the notification area). The person's
    /// own wallpaper goes back on the desktop first (docs/app-spec.md 3a).</summary>
    internal async void Quit()
    {
        if (quitting)
        {
            return;
        }
        quitting = true;
        try
        {
            await Model.BeforeQuitAsync();
        }
        catch (Exception error)
        {
            Log.Error("Quitting", error);
        }
        trayKeyboard?.Dispose();
        tray?.Dispose();
        tray = null;
        Notifications.Unregister();
        Model.Close();
        Window.Close();
        Exit();
    }
}
