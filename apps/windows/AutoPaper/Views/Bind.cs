using Microsoft.UI.Xaml;

namespace AutoPaper.Views;

/// <summary>Functions for x:Bind (visibility from state).</summary>
internal static class Bind
{
    public static Visibility Visible(bool value) => value ? Visibility.Visible : Visibility.Collapsed;

    public static Visibility Collapsed(bool value) => value ? Visibility.Collapsed : Visibility.Visible;

    public static bool Not(bool value) => !value;

    public static bool IsSet(string? value) => !string.IsNullOrEmpty(value);

    public static Visibility HasText(string? value) => string.IsNullOrEmpty(value) ? Visibility.Collapsed : Visibility.Visible;
}
