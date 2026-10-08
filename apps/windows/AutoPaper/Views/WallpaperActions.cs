using System.Diagnostics;
using AutoPaper.Core;
using AutoPaper.Models;
using AutoPaper.Services;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;

namespace AutoPaper.Views;

/// <summary>Where a wallpaper's actions are offered (History's grid and gallery, a mood's wallpapers, the summary of
/// every mood): what they say goes here.</summary>
internal interface IWallpaperHost
{
    /// <summary>An element in the window (dialogs are shown in its XamlRoot).</summary>
    UIElement DialogAnchor { get; }

    /// <summary>A result ("Deleted Harbour at dusk."), shown where the page shows them and said.</summary>
    void ShowStatus(string text, bool announce = true);

    /// <summary>A problem with its link to the fix (docs/app-spec.md 6a).</summary>
    void ShowProblem(Problem problem, bool announce);

    /// <summary>A wallpaper was deleted: the page brings its list up to date (keeping focus where it belongs).</summary>
    Task DeletedAsync(HistoryItem item);
}

/// <summary>
/// A wallpaper's actions, the same wherever a wallpaper is shown (spec: "one shared menu builder", as the macOS app's
/// HistoryActions): Show on desktop, Like, Dislike, Make an echo, Show original and echoes (only when has_echoes),
/// Show in File Explorer, Delete…. <see cref="Menu"/> is its context menu (right-click, Shift+F10, the Menu key) and the
/// "More actions" button's; the actions themselves are here so buttons elsewhere (the gallery's) do the same.
/// </summary>
internal static class WallpaperActions
{
    private static AppModel Model => App.Model;

    /// <summary>A context menu for the wallpapers of one view: <paramref name="target"/> finds the wallpaper the menu
    /// was opened on (from the flyout's target), and the menu is filled for it each time it opens.</summary>
    public static MenuFlyout Menu(IWallpaperHost host, Func<DependencyObject?, HistoryItem?> target)
    {
        var menu = new MenuFlyout();
        MenuNames.Name(menu, Loc.Get("Menu_WallpaperActions"));
        menu.Opening += (_, _) =>
        {
            menu.Items.Clear();
            if (target(menu.Target) is { } item)
            {
                Fill(menu, item, host);
            }
        };
        return menu;
    }

    /// <summary>The menu's items for one wallpaper (only what can be done now is enabled).</summary>
    private static void Fill(MenuFlyout menu, HistoryItem item, IWallpaperHost host)
    {
        var hasImage = HasImage(item);
        menu.Items.Add(Entry("HistoryShowOnDesktop/Text", "\uE7F4", hasImage && !Model.IsGenerating, () => ShowOnDesktopAsync(item, host)));
        menu.Items.Add(new MenuFlyoutSeparator());
        menu.Items.Add(Toggle("HistoryLike/Text", "\uE8E1", item.Rating == Rating.Liked, () => Model.RateAsync(item.Id, Rating.Liked)));
        menu.Items.Add(Toggle("HistoryDislike/Text", "\uE8E0", item.Rating == Rating.Disliked, () => Model.RateAsync(item.Id, Rating.Disliked)));
        menu.Items.Add(new MenuFlyoutSeparator());
        menu.Items.Add(Entry("HistoryMakeEcho/Text", "\uE8EE", !Model.IsGenerating, () => MakeEchoAsync(item, host)));
        if (item.HasEchoes)
        {
            menu.Items.Add(Entry("HistoryLineage/Text", "\uE81C", true, () => ShowLineageAsync(item, host)));
        }
        menu.Items.Add(Entry("HistoryShowInExplorer/Text", "\uE838", hasImage || HasThumbnail(item), () =>
        {
            Reveal(item);
            return Task.CompletedTask;
        }));
        menu.Items.Add(new MenuFlyoutSeparator());
        menu.Items.Add(Entry("HistoryDelete/Text", "\uE74D", true, () => DeleteAsync(item, host)));
    }

    private static MenuFlyoutItem Entry(string key, string glyph, bool enabled, Func<Task> action)
    {
        var entry = new MenuFlyoutItem { Text = Loc.Get(key), Icon = new FontIcon { Glyph = glyph }, IsEnabled = enabled };
        entry.Click += async (_, _) => await action();
        return entry;
    }

    private static ToggleMenuFlyoutItem Toggle(string key, string glyph, bool isChecked, Func<Task> action)
    {
        var entry = new ToggleMenuFlyoutItem { Text = Loc.Get(key), Icon = new FontIcon { Glyph = glyph }, IsChecked = isChecked };
        entry.Click += async (_, _) => await action();
        return entry;
    }

    public static bool HasImage(HistoryItem item) => item.Generation.ImagePath is { } path && File.Exists(path);

    private static bool HasThumbnail(HistoryItem item) => item.Generation.ThumbPath is { } path && File.Exists(path);

    /// <summary>Show on desktop (a click on a thumbnail outside History, Enter, double-click, the menu).</summary>
    public static async Task ShowOnDesktopAsync(HistoryItem item, IWallpaperHost host)
    {
        if (Model.IsGenerating)
        {
            host.ShowStatus(Loc.Get("History_Busy"));
            return;
        }
        if (!HasImage(item))
        {
            host.ShowStatus(Loc.Format("History_NoImage", item.Title));
            return;
        }
        await Model.ShowOnDesktopAsync(item.Generation);
        if (Model.Problem is { } problem)
        {
            host.ShowProblem(problem, announce: false); // The window has said it.
        }
    }

    public static async Task MakeEchoAsync(HistoryItem item, IWallpaperHost host)
    {
        host.ShowStatus(Loc.Format("History_MakingEcho", item.Title));
        // The window has said how it went (the new echo, a problem, or that it was cancelled); shown here too, a
        // problem with its link.
        var outcome = await Model.MakeEchoAsync(item.Id) ?? "";
        if (Model.Problem is { } problem && outcome == problem.Spoken)
        {
            host.ShowProblem(problem, announce: false);
        }
        else
        {
            host.ShowStatus(outcome, announce: false);
        }
    }

    /// <summary>Show original and echoes: a dialog listing the wallpaper's original and its echoes.</summary>
    public static async Task ShowLineageAsync(HistoryItem item, IWallpaperHost host)
    {
        try
        {
            var lineage = await Model.Call(engine => engine.Lineage(item.Id).Select(g => (g, engine.Describe(g.Id))).ToList());
            var entries = lineage.Select(entry => new HistoryItem(entry.g, entry.Item2, hasEchoes: true)).ToList();
            if (entries.Count < 2)
            {
                // Its echoes or original were deleted since the page was read.
                host.ShowStatus(Loc.Get("Lineage_NoneNow"));
                await Model.HistoryEditedAsync();
                return;
            }
            var list = new ListView { SelectionMode = ListViewSelectionMode.None, MaxHeight = 480 };
            AutomationProperties.SetName(list, Loc.Format("Lineage_ListName", item.Title));
            foreach (var entry in entries)
            {
                list.Items.Add(LineageRow(entry));
                _ = entry.LoadThumbnailAsync(256);
            }
            var dialog = new ContentDialog
            {
                Title = Loc.Get("Lineage_Title"),
                Content = list,
                CloseButtonText = Loc.Get("Dialog_Close"),
                DefaultButton = ContentDialogButton.Close,
            };
            await Dialogs.ShowAsync(dialog, host.DialogAnchor);
        }
        catch (Exception error)
        {
            host.ShowProblem(Text.Problem(error, Model.Settings), announce: true);
        }
    }

    /// <summary>Show in File Explorer: the picture selected in its folder (the thumbnail when the original was pruned).</summary>
    public static void Reveal(HistoryItem item)
    {
        var path = HasImage(item) ? item.Generation.ImagePath : item.Generation.ThumbPath;
        if (path is not null && File.Exists(path))
        {
            Process.Start(new ProcessStartInfo("explorer.exe", $"/select,\"{path}\"") { UseShellExecute = true });
        }
    }

    /// <summary>Delete…: a confirmation (Cancel is the default), then the wallpaper and its image go.</summary>
    public static async Task DeleteAsync(HistoryItem item, IWallpaperHost host)
    {
        var dialog = new ContentDialog
        {
            Title = Loc.Format("Delete_Title", item.Title),
            Content = Loc.Get("Delete_Body"),
            PrimaryButtonText = Loc.Get("Delete_Confirm"),
            CloseButtonText = Loc.Get("Dialog_Cancel"),
            DefaultButton = ContentDialogButton.Close,
        };
        if (await Dialogs.ShowAsync(dialog, host.DialogAnchor) != ContentDialogResult.Primary)
        {
            return;
        }
        try
        {
            await Model.Call(engine => engine.DeleteGeneration(item.Id));
            // Focus moves to the neighbour first, then the result is said (a focus change would cut it off).
            await host.DeletedAsync(item);
            host.ShowStatus(Loc.Format("History_Deleted", item.Title));
            await Model.HistoryEditedAsync();
        }
        catch (Exception error)
        {
            host.ShowProblem(Text.Problem(error, Model.Settings), announce: true);
        }
    }

    /// <summary>
    /// A wallpaper's thumbnail outside History (a mood's wallpapers, the summary's cards), as a button: a click (or
    /// Enter, Space) shows it on the desktop, and right-click, Shift+F10 or the Menu key opens History's menu. Spoken as
    /// the engine's description and the rating, as in History; its tooltip says what a click does.
    /// </summary>
    public static Button Thumbnail(HistoryItem item, IWallpaperHost host, MenuFlyout menu, double width, double height)
    {
        var image = PictureOf(item);
        var button = new Button
        {
            Width = width,
            Height = height,
            Padding = new Thickness(0),
            Content = image,
            CornerRadius = (CornerRadius)Application.Current.Resources["ControlCornerRadius"],
            HorizontalContentAlignment = HorizontalAlignment.Stretch,
            VerticalContentAlignment = VerticalAlignment.Stretch,
            ContextFlyout = menu,
            Tag = item,
            DataContext = item,
        };
        AutomationProperties.SetName(button, item.SpokenName);
        AutomationProperties.SetHelpText(button, Loc.Get("Thumbnail_Help"));
        ToolTipService.SetToolTip(button, Loc.Format("Thumbnail_Tip", item.Title));
        button.Click += async (_, _) => await ShowOnDesktopAsync(item, host);
        return button;
    }

    /// <summary>A wallpaper's thumbnail as an image that follows it as it loads (decorative: the control around it is
    /// named).</summary>
    public static Image PictureOf(HistoryItem item)
    {
        var image = new Image { Stretch = Microsoft.UI.Xaml.Media.Stretch.UniformToFill, Source = item.Thumbnail };
        AutomationProperties.SetAccessibilityView(image, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        void OnChanged(object? sender, System.ComponentModel.PropertyChangedEventArgs args)
        {
            if (args.PropertyName is nameof(HistoryItem.Thumbnail) or "" or null)
            {
                image.Source = item.Thumbnail;
            }
        }
        image.Loaded += (_, _) =>
        {
            item.PropertyChanged -= OnChanged;
            item.PropertyChanged += OnChanged;
            image.Source = item.Thumbnail;
        };
        image.Unloaded += (_, _) => item.PropertyChanged -= OnChanged;
        return image;
    }

    /// <summary>One row of Show original and echoes: its thumbnail, "Original" or "Echo", title, date and echo note.</summary>
    private static ListViewItem LineageRow(HistoryItem entry)
    {
        var grid = new Grid { ColumnSpacing = 12 };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(128) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        var frame = new Border
        {
            Height = 72,
            CornerRadius = new CornerRadius(4),
            Background = (Microsoft.UI.Xaml.Media.Brush)Application.Current.Resources["CardBackgroundFillColorDefaultBrush"],
            Child = PictureOf(entry),
        };
        grid.Children.Add(frame);
        var words = new StackPanel();
        Grid.SetColumn(words, 1);
        words.Children.Add(new TextBlock { Text = entry.LineageRole, Style = Style("CaptionTextBlockStyle"), Foreground = Brush("TextFillColorSecondaryBrush") });
        words.Children.Add(new TextBlock { Text = entry.Title, TextWrapping = TextWrapping.Wrap });
        words.Children.Add(new TextBlock { Text = entry.When, Style = Style("CaptionTextBlockStyle") });
        if (entry.EchoNote.Length > 0)
        {
            words.Children.Add(new TextBlock { Text = entry.EchoNote, Style = Style("CaptionTextBlockStyle"), TextWrapping = TextWrapping.Wrap });
        }
        grid.Children.Add(words);
        var row = new ListViewItem { Content = grid };
        AutomationProperties.SetName(row, $"{entry.LineageRole}. {entry.SpokenName}");
        return row;
    }

    public static Style Style(string key) => (Style)Application.Current.Resources[key];

    public static Microsoft.UI.Xaml.Media.Brush Brush(string key) => (Microsoft.UI.Xaml.Media.Brush)Application.Current.Resources[key];

    /// <summary>The wallpaper a menu was opened on, from its target (a thumbnail button, or anything whose data is one).</summary>
    public static HistoryItem? ItemOf(DependencyObject? target) => target switch
    {
        FrameworkElement { Tag: HistoryItem item } => item,
        ListViewItem { Content: HistoryItem item } => item,
        GridViewItem { Content: HistoryItem item } => item,
        FrameworkElement { DataContext: HistoryItem item } => item,
        _ => null,
    };

    /// <summary>Opens the menu under a "More actions" button.</summary>
    public static void ShowMenuAt(MenuFlyout menu, FrameworkElement button) =>
        menu.ShowAt(button, new FlyoutShowOptions { Placement = FlyoutPlacementMode.BottomEdgeAlignedRight });
}
