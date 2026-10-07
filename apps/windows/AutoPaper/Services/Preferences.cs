using Windows.Foundation.Collections;
using Windows.Storage;

namespace AutoPaper.Services;

/// <summary>The Windows app's own settings (the engine keeps everything else): ApplicationData.LocalSettings.</summary>
internal static class Preferences
{
    private static IPropertySet Values => ApplicationData.Current.LocalSettings.Values;

    private static T Read<T>(string key, T fallback) =>
        Values.TryGetValue(key, out var value) && value is T typed ? typed : fallback;

    /// <summary>Settings > General > Notify me about new wallpapers (on by default on Windows, per the spec).</summary>
    public static bool NotifyNewWallpapers
    {
        get => Read(nameof(NotifyNewWallpapers), true);
        set => Values[nameof(NotifyNewWallpapers)] = value;
    }

    /// <summary>The first-run page was finished or skipped.</summary>
    public static bool FirstRunDone
    {
        get => Read(nameof(FirstRunDone), false);
        set => Values[nameof(FirstRunDone)] = value;
    }

    /// <summary>Settings reopen on the last pane (as AudioPaper's do).</summary>
    public static string SettingsPane
    {
        get => Read(nameof(SettingsPane), "");
        set => Values[nameof(SettingsPane)] = value;
    }

    /// <summary>The month ("yyyy-MM", UTC as the engine counts) whose "budget is spent" notification was shown.</summary>
    public static string BudgetNotifiedMonth
    {
        get => Read(nameof(BudgetNotifiedMonth), "");
        set => Values[nameof(BudgetNotifiedMonth)] = value;
    }

    /// <summary>"Your key stopped working" was shown, and no wallpaper has been made since.</summary>
    public static bool KeyProblemNotified
    {
        get => Read(nameof(KeyProblemNotified), false);
        set => Values[nameof(KeyProblemNotified)] = value;
    }

    /// <summary>History was cleared while a wallpaper was on the desktop, and none has been shown since: the desktop
    /// keeps that picture (the engine keeps its display renders), so Now doesn't say "No wallpaper yet".</summary>
    public static bool DesktopKeepsPicture
    {
        get => Read(nameof(DesktopKeepsPicture), false);
        set => Values[nameof(DesktopKeepsPicture)] = value;
    }

    /// <summary>The person's own wallpaper is on the desktop (AutoPaper put it back on pause, quit or Restore my
    /// wallpaper), so the next launch or Resume puts AutoPaper's back.</summary>
    public static bool DesktopShowsOwn
    {
        get => Read(nameof(DesktopShowsOwn), false);
        set => Values[nameof(DesktopShowsOwn)] = value;
    }

    /// <summary>The person's own wallpaper was put back because AutoPaper quit (not by Restore my wallpaper): the next
    /// launch puts AutoPaper's back.</summary>
    public static bool UncoveredByQuit
    {
        get => Read(nameof(UncoveredByQuit), false);
        set => Values[nameof(UncoveredByQuit)] = value;
    }

    /// <summary>What putting the person's own lock screen back couldn't do (Settings › General says it, with a link).</summary>
    public static LockScreenNote LockScreenNote
    {
        get => (LockScreenNote)Read(nameof(LockScreenNote), (int)LockScreenNote.None);
        set => Values[nameof(LockScreenNote)] = (int)value;
    }

    /// <summary>History's layout: "Grid" or "Gallery" (remembered, as on the Mac).</summary>
    public static string HistoryLayout
    {
        get => Read(nameof(HistoryLayout), "Grid");
        set => Values[nameof(HistoryLayout)] = value;
    }

    /// <summary>The file name of the person's own ComfyUI workflow ("flux-dev-api.json"), shown in Settings ›
    /// Providers. Its text is kept in <see cref="OwnWorkflowFile"/> (LocalSettings holds at most 8 KB a value), so
    /// switching back to AutoPaper's workflow and to it again is one choice.</summary>
    public static string OwnWorkflowName
    {
        get => Read(nameof(OwnWorkflowName), "");
        set => Values[nameof(OwnWorkflowName)] = value;
    }

    /// <summary>Where the person's own workflow's text is kept while AutoPaper's is chosen.</summary>
    public static string OwnWorkflowFile => Path.Combine(ApplicationData.Current.LocalFolder.Path, "workflows", "own-workflow.json");
}
