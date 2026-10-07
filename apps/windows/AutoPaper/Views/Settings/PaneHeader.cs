using System.Runtime.CompilerServices;
using AutoPaper.Core;
using AutoPaper.Services;
using AutoPaper.Views;
using CommunityToolkit.WinUI.Controls;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media.Animation;

namespace AutoPaper.Views.Panes;

/// <summary>
/// A pane's title as Windows 11 Settings shows it: a breadcrumb "Settings › General" at title size, whose first
/// crumb goes back to the Settings home. Also remembers the pane, so Settings reopen on it. And a setting that
/// couldn't be saved, said on its own card (<see cref="ShowProblem"/>).
/// </summary>
internal static class PaneHeader
{
    /// <summary>Each card showing a problem, with the description it had before.</summary>
    private static readonly ConditionalWeakTable<SettingsCard, StrongBox<object?>> Replaced = new();

    public static void Attach(BreadcrumbBar crumbs, string pane, Frame frame)
    {
        crumbs.ItemsSource = new[] { Loc.Get("Settings_Title"), Loc.Get($"Pane_{pane}") };
        crumbs.ItemClicked += (_, args) =>
        {
            if (args.Index != 0)
            {
                return;
            }
            if (frame.CanGoBack && frame.BackStack[^1].SourcePageType == typeof(SettingsPage))
            {
                frame.GoBack(new SlideNavigationTransitionInfo { Effect = SlideNavigationTransitionEffect.FromLeft });
            }
            else
            {
                frame.Navigate(typeof(SettingsPage), null, new SlideNavigationTransitionInfo { Effect = SlideNavigationTransitionEffect.FromLeft });
            }
        };
        Preferences.SettingsPane = pane;
    }

    /// <summary>A ContentDialog's result, the XamlRoot set from <paramref name="anchor"/>.</summary>
    public static async Task<ContentDialogResult> ConfirmAsync(UIElement anchor, string title, string body, string primary, string? secondary = null)
    {
        var dialog = new ContentDialog
        {
            Title = title,
            Content = new TextBlock { Text = body, TextWrapping = TextWrapping.Wrap },
            PrimaryButtonText = primary,
            SecondaryButtonText = secondary ?? "",
            CloseButtonText = Loc.Get("Dialog_Cancel"),
            DefaultButton = ContentDialogButton.Close,
        };
        return await Dialogs.ShowAsync(dialog, anchor);
    }

    /// <summary>
    /// A change that couldn't be saved, said where it was made: the card's description becomes the reason, in the
    /// error colour (in view, next to the control the person just used, rather than at the end of the pane), and the
    /// window says it. The page puts the control back to the saved value first. A failure that isn't one of the
    /// engine's own errors is logged. <see cref="ClearProblem"/> puts the description back.
    /// </summary>
    public static void ShowProblem(SettingsCard card, Exception error, string message)
    {
        if (error is not AutoPaperException)
        {
            Log.Error($"Saving a setting ({AutomationName(card)})", error);
        }
        if (!IsProblem(card.Description))
        {
            Replaced.AddOrUpdate(card, new StrongBox<object?>(card.Description));
        }
        card.Description = ProblemText(message);
        App.Model.AnnounceError(message);
    }

    /// <summary>The card's own description again, after a change on it saved (unless the page has given it another
    /// one meanwhile).</summary>
    public static void ClearProblem(SettingsCard card)
    {
        if (Replaced.TryGetValue(card, out var before))
        {
            Replaced.Remove(card);
            if (IsProblem(card.Description))
            {
                card.Description = before.Value!;
            }
        }
    }

    /// <summary>A problem as a card's description: in the error colour, wrapped.</summary>
    public static TextBlock ProblemText(string message) =>
        new() { Text = message, Style = (Style)Application.Current.Resources["SettingProblemTextBlockStyle"] };

    private static bool IsProblem(object? description) =>
        description is TextBlock text && ReferenceEquals(text.Style, Application.Current.Resources["SettingProblemTextBlockStyle"]);

    private static string AutomationName(SettingsCard card) =>
        Microsoft.UI.Xaml.Automation.AutomationProperties.GetName(card) is { Length: > 0 } name ? name : card.Header as string ?? "";
}
