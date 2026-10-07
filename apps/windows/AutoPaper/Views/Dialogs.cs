using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace AutoPaper.Views;

/// <summary>
/// Shows a ContentDialog made in code the way XAML ones look: in the window's XamlRoot, with the Windows 11 dialog
/// style, and in the window's current theme (a dialog made after a live light/dark switch otherwise takes the theme
/// the app started with, and its buttons' text can vanish against the wrong background).
/// </summary>
internal static class Dialogs
{
    public static async Task<ContentDialogResult> ShowAsync(ContentDialog dialog, UIElement anchor)
    {
        dialog.XamlRoot = anchor.XamlRoot;
        if (anchor.XamlRoot?.Content is FrameworkElement root)
        {
            dialog.RequestedTheme = root.ActualTheme;
        }
        if (Application.Current.Resources.TryGetValue("DefaultContentDialogStyle", out var style) && style is Style windowsStyle)
        {
            dialog.Style = windowsStyle;
        }
        return await dialog.ShowAsync();
    }
}
