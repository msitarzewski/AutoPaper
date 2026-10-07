using AutoPaper.Core;
using AutoPaper.Models;
using AutoPaper.Services;
using Microsoft.UI.Xaml.Controls;

namespace AutoPaper;

/// <summary>
/// The notification-area menu (spec "Always there"), built fresh each time it opens: the current wallpaper's
/// title (and echo note), Like and Dislike (checked for the current rating; choosing again clears it), Mood ▸ (the
/// moods, the current one checked, then Edit moods), New wallpaper now, the stage (with the time left) and Cancel
/// while making one, Pause/Resume, Restore my wallpaper, Show AutoPaper, Settings, Help, Quit.
/// A WinUI MenuFlyout shown by WinUIEx's TrayIcon: Windows 11's menu look, theme and contrast, UI Automation.
/// Sentence case (Windows); no ellipsis on Settings (it opens a page, no further input needed).
/// </summary>
internal static class TrayMenu
{
    public static MenuFlyout Build(AppModel model)
    {
        var menu = new MenuFlyout();
        Views.MenuNames.Name(menu, "AutoPaper");
        if (!model.IsReady)
        {
            menu.Items.Add(Disabled(model.StartupProblem ?? Loc.Get("Menu_Opening")));
        }
        else
        {
            if (model.Current is { } current)
            {
                menu.Items.Add(Disabled(current.Concept.Title));
                if (current.EchoNote is { Length: > 0 } note)
                {
                    menu.Items.Add(Disabled(note));
                }
                menu.Items.Add(Toggle("Menu_Like", model.IsLiked, () => model.RateAsync(current.Id, Rating.Liked)));
                menu.Items.Add(Toggle("Menu_Dislike", model.IsDisliked, () => model.RateAsync(current.Id, Rating.Disliked)));
                menu.Items.Add(new MenuFlyoutSeparator());
            }
            menu.Items.Add(MoodMenu(model));
            var make = Item("Menu_NewWallpaper", () => model.NewWallpaperAsync());
            make.IsEnabled = !model.IsGenerating;
            menu.Items.Add(make);
            if (model.IsGenerating)
            {
                menu.Items.Add(Disabled(Text.StageWithTimeLeft(model.StageText, model.SecondsLeft)));
                menu.Items.Add(Item("Menu_Cancel", () =>
                {
                    model.Cancel();
                    return Task.CompletedTask;
                }));
            }
            var paused = model.Settings.Paused;
            menu.Items.Add(Item(paused ? "Menu_Resume" : "Menu_Pause", async () =>
            {
                try
                {
                    await model.SetPausedAsync(!paused);
                }
                catch (Exception error)
                {
                    model.AnnounceError(Text.Error(error, model.Settings));
                }
            }));
            // The person's own wallpaper back (docs/app-spec.md 3a); AutoPaper's returns with the next new one.
            var restore = Item("Menu_RestoreWallpaper", () => model.RestoreOwnWallpaperAsync());
            restore.IsEnabled = model.CanRestoreWallpaper;
            menu.Items.Add(restore);
        }
        menu.Items.Add(new MenuFlyoutSeparator());
        menu.Items.Add(Item("Menu_Show", () =>
        {
            App.Window.ShowWindow();
            return Task.CompletedTask;
        }));
        menu.Items.Add(Item("Menu_Settings", () =>
        {
            App.Window.ShowSettings(null);
            return Task.CompletedTask;
        }));
        // AutoPaper's help, on its website (F1 in the window).
        menu.Items.Add(Item("Menu_Help", () => Links.OpenAsync(Links.Help)));
        menu.Items.Add(new MenuFlyoutSeparator());
        menu.Items.Add(Item("Menu_Quit", () =>
        {
            App.Current.Quit();
            return Task.CompletedTask;
        }));
        return menu;
    }

    /// <summary>Mood ▸: each mood (the current one checked; choosing one makes it current, nothing is made by itself),
    /// then Edit moods (opens Moods on the current one).</summary>
    private static MenuFlyoutSubItem MoodMenu(AppModel model)
    {
        var moods = new MenuFlyoutSubItem { Text = Loc.Get("Menu_Mood") };
        foreach (var mood in model.Moods)
        {
            var item = new RadioMenuFlyoutItem { Text = mood.Name, GroupName = "moods", IsChecked = mood.Active };
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(item, MoodText.SpokenName(mood));
            var id = mood.Id;
            item.Click += async (_, _) =>
            {
                try
                {
                    await model.UseMoodAsync(id);
                }
                catch (Exception error)
                {
                    model.AnnounceError(Text.Error(error, model.Settings));
                }
            };
            moods.Items.Add(item);
        }
        moods.Items.Add(new MenuFlyoutSeparator());
        moods.Items.Add(Item("Menu_EditMoods", () =>
        {
            App.Window.ShowMoods(model.ActiveMood?.Id);
            return Task.CompletedTask;
        }));
        return moods;
    }

    private static MenuFlyoutItem Disabled(string text) => new() { Text = text, IsEnabled = false };

    private static MenuFlyoutItem Item(string key, Func<Task> action)
    {
        var item = new MenuFlyoutItem { Text = Loc.Get(key) };
        item.Click += async (_, _) => await action();
        return item;
    }

    private static ToggleMenuFlyoutItem Toggle(string key, bool isChecked, Func<Task> action)
    {
        var item = new ToggleMenuFlyoutItem { Text = Loc.Get(key), IsChecked = isChecked };
        item.Click += async (_, _) => await action();
        return item;
    }
}
