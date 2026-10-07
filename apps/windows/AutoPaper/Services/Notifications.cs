using AutoPaper.Core;
using Microsoft.Windows.AppNotifications;
using Microsoft.Windows.AppNotifications.Builder;
using Windows.Storage;

namespace AutoPaper.Services;

/// <summary>
/// App notifications (Windows App SDK AppNotificationManager): "New wallpaper: &lt;title&gt;" with Like, Dislike and
/// Show; one when the month's budget runs out; one when a key stops working. Never for routine failures.
/// Buttons arrive as arguments: action = like | dislike | show | budget | keys, id = the wallpaper, account = the key
/// whose field Settings › Keys focuses.
/// </summary>
internal static class Notifications
{
    private const string Group = "autopaper";

    public static void Register(Action<IDictionary<string, string>> invoked)
    {
        AppNotificationManager.Default.NotificationInvoked += (_, args) => invoked(args.Arguments);
        AppNotificationManager.Default.Register();
    }

    public static void Unregister()
    {
        try
        {
            AppNotificationManager.Default.Unregister();
        }
        catch (Exception)
        {
            // Not registered (start-up failed before it); nothing to undo.
        }
    }

    public static void NewWallpaper(Generation generation)
    {
        var builder = new AppNotificationBuilder()
            .AddArgument("action", "show")
            .AddArgument("id", generation.Id)
            .AddText(Loc.Format("Notification_NewWallpaper", generation.Concept.Title))
            .AddText(generation.EchoNote ?? generation.Concept.Summary)
            .AddButton(new AppNotificationButton(Loc.Get("Notification_Like"))
                .AddArgument("action", "like").AddArgument("id", generation.Id))
            .AddButton(new AppNotificationButton(Loc.Get("Notification_Dislike"))
                .AddArgument("action", "dislike").AddArgument("id", generation.Id))
            .AddButton(new AppNotificationButton(Loc.Get("Notification_Show"))
                .AddArgument("action", "show").AddArgument("id", generation.Id));
        if (ImageUri(generation.ThumbPath) is { } thumbnail)
        {
            // Its alternative text names the picture (the toast's own text has the title and summary).
            builder.SetHeroImage(thumbnail, Loc.Format("Notification_ImageAlt", generation.Concept.Title));
        }
        Show(builder, "new-wallpaper");
    }

    public static void BudgetSpent(Fallback fallback)
    {
        var builder = new AppNotificationBuilder()
            .AddArgument("action", "budget")
            .AddText(Loc.Get(fallback == Fallback.RevisitLiked ? "Error_BudgetReached" : "Error_BudgetReachedKeep"))
            .AddButton(new AppNotificationButton(Loc.Get("Notification_OpenBudget")).AddArgument("action", "budget"));
        Show(builder, "budget");
    }

    /// <summary>A key stopped working on the schedule (once, until a wallpaper is made again): the link's words ("Add
    /// your OpenAI key", docs/app-spec.md 6a), and the notification and its button open Settings › Keys with that
    /// key's field focused.</summary>
    public static void KeyProblem(Problem problem)
    {
        var account = problem.KeyAccount ?? "";
        var builder = new AppNotificationBuilder()
            .AddArgument("action", "keys")
            .AddArgument("account", account)
            .AddText(problem.Spoken)
            .AddText(Loc.Get("Notification_KeyBody"))
            .AddButton(new AppNotificationButton(Loc.Get("Notification_OpenKeys"))
                .AddArgument("action", "keys").AddArgument("account", account));
        Show(builder, "keys");
    }

    /// <summary>
    /// The person's Windows Spotlight background (desktop), or their lock screen's Spotlight or slideshow, came back
    /// as its last picture (Restore my wallpaper from the notification area, or quit), and stays off: Windows lets only
    /// the person turn it on. The notification and its button open the Settings page where it's turned on (protocol
    /// activation: it works after AutoPaper has quit, without starting it again): Background for the desktop's, Lock
    /// screen when only the lock screen's is off.
    /// </summary>
    public static void SpotlightOff(bool desktop, LockScreenNote? lockScreen)
    {
        var lockOff = lockScreen is LockScreenNote.Spotlight or LockScreenNote.Slideshow;
        var fix = desktop ? "ms-settings:personalization-background" : "ms-settings:lockscreen";
        var (title, body, button) = (desktop, lockOff) switch
        {
            (true, true) => ("Notification_SpotlightBothTitle", "Notification_SpotlightBothBody", "Spotlight_TurnOn"),
            (false, true) => (lockScreen == LockScreenNote.Slideshow ? "Notification_LockSlideshowTitle" : "Notification_LockSpotlightTitle",
                "Notification_LockSpotlightBody", lockScreen == LockScreenNote.Slideshow ? "OwnLockScreen_TurnOnSlideshow" : "OwnLockScreen_TurnOnSpotlight"),
            _ => ("Notification_SpotlightTitle", "Notification_SpotlightBody", "Spotlight_TurnOn"),
        };
        var payload =
            $"<toast launch=\"{fix}\" activationType=\"protocol\"><visual><binding template=\"ToastGeneric\">" +
            $"<text>{Escape(Loc.Get(title))}</text>" +
            $"<text>{Escape(Loc.Get(body))}</text>" +
            $"</binding></visual><actions><action content=\"{Escape(Loc.Get(button))}\" activationType=\"protocol\" arguments=\"{fix}\" /></actions></toast>";
        try
        {
            var notification = new AppNotification(payload) { Tag = "spotlight", Group = Group };
            AppNotificationManager.Default.Show(notification);
        }
        catch (Exception error)
        {
            System.Diagnostics.Debug.WriteLine($"Notification not shown: {error.Message}");
        }
    }

    private static string Escape(string text) => System.Security.SecurityElement.Escape(text) ?? "";

    /// <summary>Removes the "new wallpaper" notification (rated or seen in the window already).</summary>
    public static void ClearNewWallpaper() =>
        _ = AppNotificationManager.Default.RemoveByTagAndGroupAsync("new-wallpaper", Group);

    private static void Show(AppNotificationBuilder builder, string tag)
    {
        try
        {
            var notification = builder.BuildNotification();
            notification.Tag = tag;
            notification.Group = Group;
            AppNotificationManager.Default.Show(notification);
        }
        catch (Exception error)
        {
            // Notifications turned off for the app, or the platform refused: nothing else to do.
            System.Diagnostics.Debug.WriteLine($"Notification not shown: {error.Message}");
        }
    }

    /// <summary>An image in LocalState as ms-appdata:///local/… (packaged apps' own storage).</summary>
    private static Uri? ImageUri(string? path)
    {
        if (string.IsNullOrEmpty(path) || !File.Exists(path))
        {
            return null;
        }
        var local = ApplicationData.Current.LocalFolder.Path.TrimEnd('\\') + "\\";
        if (path.StartsWith(local, StringComparison.OrdinalIgnoreCase))
        {
            var relative = path[local.Length..].Replace('\\', '/');
            return new Uri("ms-appdata:///local/" + string.Join('/', relative.Split('/').Select(Uri.EscapeDataString)));
        }
        return new Uri(path);
    }
}
