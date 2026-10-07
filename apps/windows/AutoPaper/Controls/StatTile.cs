using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;

namespace AutoPaper.Controls;

/// <summary>
/// A number with what it counts, on a card (the Moods summary's totals, a mood's header; the macOS app's StatTile): a
/// small icon and caption above, the number below. A screen reader hears it as one line ("12 liked"): the number is
/// named with the whole phrase and the caption is left out.
/// </summary>
internal static class StatTile
{
    public static FrameworkElement Create(string caption, string glyph, string value, string spoken, bool compact = false)
    {
        var captionRow = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
        captionRow.Children.Add(new FontIcon { Glyph = glyph, FontSize = compact ? 12 : 14, Foreground = Brush("TextFillColorSecondaryBrush") });
        captionRow.Children.Add(new TextBlock
        {
            Text = caption,
            Style = Style("CaptionTextBlockStyle"),
            Foreground = Brush("TextFillColorSecondaryBrush"),
            TextTrimming = TextTrimming.CharacterEllipsis,
        });
        AutomationProperties.SetAccessibilityView(captionRow, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        foreach (var child in captionRow.Children)
        {
            AutomationProperties.SetAccessibilityView(child, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        }
        var number = new TextBlock
        {
            Text = value,
            Style = Style(compact ? "SubtitleTextBlockStyle" : "TitleTextBlockStyle"),
            TextTrimming = TextTrimming.CharacterEllipsis,
            TextWrapping = TextWrapping.NoWrap,
        };
        AutomationProperties.SetName(number, spoken);
        var stack = new StackPanel { Spacing = compact ? 2 : 4 };
        stack.Children.Add(captionRow);
        stack.Children.Add(number);
        return new Border
        {
            Padding = compact ? new Thickness(12, 8, 12, 8) : new Thickness(16, 12, 16, 12),
            Background = Brush("CardBackgroundFillColorDefaultBrush"),
            BorderBrush = Brush("CardStrokeColorDefaultBrush"),
            BorderThickness = new Thickness(1),
            CornerRadius = (CornerRadius)Application.Current.Resources["OverlayCornerRadius"],
            Child = stack,
        };
    }

    /// <summary>Lays tiles out in a row, or two by two when the row is narrower than <paramref name="minimumEach"/> a
    /// tile (larger text, a narrow pane).</summary>
    public static void Arrange(Grid grid, double width, double minimumEach)
    {
        var tiles = grid.Children.OfType<FrameworkElement>().ToList();
        if (tiles.Count == 0)
        {
            return;
        }
        var perRow = width >= minimumEach * tiles.Count + grid.ColumnSpacing * (tiles.Count - 1) ? tiles.Count : Math.Min(2, tiles.Count);
        if (grid.ColumnDefinitions.Count == perRow && grid.RowDefinitions.Count == (tiles.Count + perRow - 1) / perRow)
        {
            return;
        }
        grid.ColumnDefinitions.Clear();
        grid.RowDefinitions.Clear();
        for (var column = 0; column < perRow; column++)
        {
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        }
        for (var row = 0; row < (tiles.Count + perRow - 1) / perRow; row++)
        {
            grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        }
        for (var index = 0; index < tiles.Count; index++)
        {
            Grid.SetColumn(tiles[index], index % perRow);
            Grid.SetRow(tiles[index], index / perRow);
        }
    }

    private static Style Style(string key) => (Style)Application.Current.Resources[key];

    private static Brush Brush(string key) => (Brush)Application.Current.Resources[key];
}
