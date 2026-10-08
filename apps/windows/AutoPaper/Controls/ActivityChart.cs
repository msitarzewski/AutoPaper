using AutoPaper.Core;
using Microsoft.UI;
using AutoPaper.Models;
using AutoPaper.Services;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Documents;
using Microsoft.UI.Xaml.Media;
using Windows.UI;

namespace AutoPaper.Controls;

/// <summary>
/// "Wallpapers, last 30 days" (the Moods summary; the macOS app's MoodActivityChart): a bar per day, stacked by mood,
/// the scale on the trailing side, a legend (two moods or more), the total, and the same numbers as a table ("Show as
/// table"). Hovering a day shows its numbers. Identity never rests on colour alone: the legend names each colour, the
/// tooltips and the table say every number, and a screen reader reads the chart as one picture with a summary and the
/// table as text. Colours: the dataviz palette (validated against Windows' light and dark surfaces), a fixed slot per
/// mood (by when it was made); high contrast uses the system's colours.
/// </summary>
public sealed partial class ActivityChart : StackPanel
{
    private const double PlotHeight = 190;

    /// <summary>The palette's light and dark steps, in slot order; "Other moods" is a neutral grey.</summary>
    private static readonly uint[] Light = [0x2A78D6, 0xEB6834, 0x1BAF7A, 0xEDA100, 0xE87BA4, 0x008300, 0x4A3AA7, 0xE34948];
    private static readonly uint[] Dark = [0x3987E5, 0xD95926, 0x199E70, 0xC98500, 0xD55181, 0x008300, 0x9085E9, 0xE66767];
    private const uint Other = 0x8A8984;

    private IReadOnlyList<Mood> moods = [];
    private IReadOnlyList<DayCount> counts = [];
    private IReadOnlyList<ConsoleDay>? consoleCounts;
    private bool IsConsole => consoleCounts is not null;
    private string Title => Loc.Get(IsConsole ? "Console_DailyRuns" : "Activity_Title");

    private sealed record Part(long DayStart, string? Key, int Count, int Base, bool IsTop);

    public ActivityChart()
    {
        Spacing = 10;
        ActualThemeChanged += (_, _) => Build();
    }

    /// <summary>Shows the moods' activity (moods in the person's order; counts from activity(day_bounds)).</summary>
    internal void Show(IReadOnlyList<Mood> moods, IReadOnlyList<DayCount> counts)
    {
        this.moods = moods;
        this.counts = counts;
        consoleCounts = null;
        Build();
    }

    /// <summary>The same native chart and accessible table, with real retained run counts stacked by outcome.
    /// Console days use UTC midnight; they must not be shifted into the previous local calendar date.</summary>
    internal void ShowOutcomes(IReadOnlyList<ConsoleDay> counts)
    {
        consoleCounts = counts;
        Build();
    }

    /// <summary>A mood's colour (its slot), for the summary's cards.</summary>
    internal Color ColorOf(string? moodId, IReadOnlyDictionary<string, int> slots) =>
        moodId is not null && slots.TryGetValue(moodId, out var slot) ? Rgb((ActualTheme == ElementTheme.Dark ? Dark : Light)[slot]) : Rgb(Other);

    internal static Color Rgb(uint rgb) => Color.FromArgb(255, (byte)(rgb >> 16), (byte)(rgb >> 8), (byte)rgb);

    private string NameOf(string? key) => IsConsole
        ? Text.RunOutcome(Enum.Parse<RunStatus>(key!))
        : moods.FirstOrDefault(mood => mood.Id == key)?.Name ?? Loc.Get("Activity_Other");

    private Color SeriesColor(string? key, IReadOnlyDictionary<string, int> slots)
    {
        if (!IsConsole) return ColorOf(key, slots);
        var slot = Enum.Parse<RunStatus>(key!) switch
        {
            RunStatus.Running => 0,
            RunStatus.Succeeded => 2,
            RunStatus.Failed => 7,
            RunStatus.Blocked => 3,
            RunStatus.Interrupted => 1,
            _ => -1,
        };
        return slot < 0 ? Rgb(Other) : Rgb((ActualTheme == ElementTheme.Dark ? Dark : Light)[slot]);
    }

    private List<Part> Parts(IReadOnlyDictionary<string, int> slots)
    {
        if (consoleCounts is null)
            return MoodActivity.Segments(counts, moods.Select(mood => mood.Id).ToList(), slots)
                .Select(part => new Part(part.DayStart, part.MoodId, part.Count, part.Base, part.IsTop)).ToList();
        var parts = new List<Part>();
        foreach (var day in consoleCounts.GroupBy(count => count.DayStart).OrderBy(day => day.Key))
        {
            var stack = day.Where(count => count.Count > 0).OrderBy(count => count.Status).ToList();
            var below = 0;
            for (var index = 0; index < stack.Count; index++)
            {
                var part = stack[index];
                var count = checked((int)part.Count);
                parts.Add(new Part(day.Key, part.Status.ToString(), count, below, index == stack.Count - 1));
                below += count;
            }
        }
        return parts;
    }

    private void Build()
    {
        Children.Clear();
        var title = new TextBlock { Text = Title, Style = TextStyle("SubtitleTextBlockStyle") };
        AutomationProperties.SetHeadingLevel(title, AutomationHeadingLevel.Level2);
        Children.Add(title);

        var bounds = IsConsole ? MoodActivity.DayBounds(zone: TimeZoneInfo.Utc) : MoodActivity.DayBounds();
        // Retention is a rolling 30 days, which can span 31 UTC calendar dates. Include its first partial day.
        if (consoleCounts is { Count: > 0 } && consoleCounts.Min(day => day.DayStart) is var earliest && earliest < bounds[0])
            bounds = MoodActivity.DayBounds(days: checked((int)((bounds[^1] - earliest) / 86_400)), zone: TimeZoneInfo.Utc);
        var slots = MoodActivity.ColorSlots(moods);
        var segments = Parts(slots);
        var total = segments.Sum(segment => segment.Count);
        if (total == 0)
        {
            var empty = new TextBlock { Text = IsConsole ? Loc.Get("Console_NoChartData") : Loc.Format("Activity_None", MoodActivity.Days),
                Style = TextStyle("BodyTextBlockStyle"), Foreground = Brush("TextFillColorSecondaryBrush") };
            if (IsConsole)
            {
                empty.HorizontalAlignment = HorizontalAlignment.Center;
                empty.VerticalAlignment = VerticalAlignment.Center;
                var area = new Grid { MinHeight = PlotHeight };
                area.Children.Add(empty);
                Children.Add(area);
            }
            else Children.Add(empty);
            return;
        }
        var days = segments.GroupBy(segment => segment.DayStart).ToDictionary(group => group.Key, group => group.ToList());
        var ticks = MoodActivity.Ticks(days.Values.Max(day => day.Sum(segment => segment.Count)));
        var top = Math.Max(1, ticks[^1]);
        var contrast = new Windows.UI.ViewManagement.AccessibilitySettings().HighContrast;

        var plot = new ChartGrid(Summary(total));
        plot.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        plot.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        plot.RowDefinitions.Add(new RowDefinition { Height = new GridLength(PlotHeight) });
        plot.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });

        // Recessive grid lines and the scale on the trailing side.
        var lines = new Grid();
        var scale = new Grid { Margin = new Thickness(8, 0, 0, 0), MinWidth = 24 };
        Grid.SetColumn(scale, 1);
        foreach (var tick in ticks)
        {
            var y = PlotHeight - tick * PlotHeight / top;
            lines.Children.Add(new Border
            {
                Height = 1,
                VerticalAlignment = VerticalAlignment.Top,
                Margin = new Thickness(0, Math.Min(y, PlotHeight - 1), 0, 0),
                Background = Brush("DividerStrokeColorDefaultBrush"),
            });
            scale.Children.Add(new TextBlock
            {
                Text = tick.ToString(System.Globalization.CultureInfo.CurrentCulture),
                Style = TextStyle("CaptionTextBlockStyle"),
                Foreground = Brush("TextFillColorSecondaryBrush"),
                VerticalAlignment = VerticalAlignment.Top,
                Margin = new Thickness(0, Math.Clamp(y - 8, 0, PlotHeight - 16), 0, 0),
            });
        }
        plot.Children.Add(lines);
        plot.Children.Add(scale);

        // A column per day; each day's bar is its moods' segments, bottom up, 2 px of surface between them and the top
        // end rounded. The whole column shows the day's numbers on hover.
        var bars = new Grid { ColumnSpacing = 2 };
        var labels = new Grid { ColumnSpacing = 2, Margin = new Thickness(0, 6, 0, 0) };
        Grid.SetRow(labels, 1);
        for (var index = 0; index < bounds.Length - 1; index++)
        {
            bars.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            labels.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            if (!days.TryGetValue(bounds[index], out var stack))
            {
                continue;
            }
            var column = new Grid { Background = new SolidColorBrush(Colors.Transparent) };
            Grid.SetColumn(column, index);
            var bar = new StackPanel { VerticalAlignment = VerticalAlignment.Bottom, HorizontalAlignment = HorizontalAlignment.Center, Width = 0 };
            column.SizeChanged += (_, args) => bar.Width = Math.Max(3, args.NewSize.Width * 0.62);
            foreach (var segment in Enumerable.Reverse(stack))
            {
                var height = segment.Count * PlotHeight / top;
                var color = contrast ? null : new SolidColorBrush(SeriesColor(segment.Key, slots));
                bar.Children.Add(new Border
                {
                    Height = Math.Max(1, height - (segment.Base > 0 ? 2 : 0)),
                    Margin = new Thickness(0, 0, 0, segment.Base > 0 ? 2 : 0),
                    CornerRadius = segment.IsTop ? new CornerRadius(4, 4, 0, 0) : new CornerRadius(0),
                    Background = color ?? SystemBrush("SystemColorHighlightColor"),
                    BorderBrush = contrast ? SystemBrush("SystemColorWindowTextColor") : null,
                    BorderThickness = contrast ? new Thickness(1) : new Thickness(0),
                });
            }
            column.Children.Add(bar);
            ToolTipService.SetToolTip(column, DayTip(bounds[index], stack));
            bars.Children.Add(column);
        }
        plot.Children.Add(bars);
        foreach (var label in MoodActivity.WeekLabels(bounds))
        {
            var index = Array.IndexOf(bounds, label);
            var text = new TextBlock
            {
                Text = IsConsole ? DateTimeOffset.FromUnixTimeSeconds(label).UtcDateTime.ToString("MMM d", System.Globalization.CultureInfo.CurrentCulture)
                    : Text.Clean(new Windows.Globalization.DateTimeFormatting.DateTimeFormatter("month.abbreviated day").Format(DateTimeOffset.FromUnixTimeSeconds(label).ToLocalTime())),
                Style = TextStyle("CaptionTextBlockStyle"),
                Foreground = Brush("TextFillColorSecondaryBrush"),
                TextAlignment = TextAlignment.Center,
                HorizontalAlignment = HorizontalAlignment.Center,
            };
            Grid.SetColumn(text, Math.Max(0, index - 2));
            Grid.SetColumnSpan(text, 5);
            labels.Children.Add(text);
        }
        plot.Children.Add(labels);
        Children.Add(plot);

        // The legend (two series or more): a dot of each colour and its name, wrapping as text does.
        var series = segments.Select(segment => segment.Key).Distinct()
            .OrderBy(id => IsConsole ? (int)Enum.Parse<RunStatus>(id!) : id is null ? int.MaxValue : moods.ToList().FindIndex(mood => mood.Id == id))
            .ToList();
        if (IsConsole || series.Count > 1)
        {
            var legend = new TextBlock { Style = TextStyle("BodyTextBlockStyle"), TextWrapping = TextWrapping.Wrap };
            foreach (var id in series)
            {
                if (legend.Inlines.Count > 0)
                {
                    legend.Inlines.Add(new Run { Text = "    " });
                }
                legend.Inlines.Add(new Run { Text = "\u25CF ", Foreground = contrast ? Brush("TextFillColorPrimaryBrush") : new SolidColorBrush(SeriesColor(id, slots)) });
                legend.Inlines.Add(new Run { Text = IsConsole ? Loc.Format("Activity_DayPart", NameOf(id), segments.Where(part => part.Key == id).Sum(part => part.Count)) : NameOf(id) });
            }
            AutomationProperties.SetName(legend, Loc.Format("Activity_Legend", string.Join(", ", series.Select(id => IsConsole
                ? Loc.Format("Activity_DayPart", NameOf(id), segments.Where(part => part.Key == id).Sum(part => part.Count)) : NameOf(id)))));
            Children.Add(legend);
        }
        if (!IsConsole)
            Children.Add(new TextBlock { Text = MoodActivity.Footnote(total), Style = TextStyle("BodyTextBlockStyle"), Foreground = Brush("TextFillColorSecondaryBrush") });

        // The numbers as a table: each day with wallpapers, newest first.
        Children.Add(new Expander
        {
            Header = Loc.Get("Activity_ShowTable"),
            HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Left,
            Content = Table(days),
        });
    }

    private string DayTip(long dayStart, List<Part> stack) =>
        $"{DayName(dayStart)}\n" + string.Join("\n", Enumerable.Reverse(stack).Select(segment => Loc.Format("Activity_DayPart", NameOf(segment.Key), segment.Count)));

    private string DayName(long dayStart) => IsConsole
        ? DateTimeOffset.FromUnixTimeSeconds(dayStart).UtcDateTime.ToString("ddd, MMM d", System.Globalization.CultureInfo.CurrentCulture)
        : Text.Clean(new Windows.Globalization.DateTimeFormatting.DateTimeFormatter("dayofweek.abbreviated month.abbreviated day").Format(DateTimeOffset.FromUnixTimeSeconds(dayStart).ToLocalTime()));

    private string Summary(int total) => IsConsole ? Loc.Format("Console_ChartSummary", total)
        : $"{Title}. {MoodActivity.Footnote(total)} {Loc.Get("Activity_TableHint")}";

    private Grid Table(Dictionary<long, List<Part>> days)
    {
        var table = new Grid { ColumnSpacing = 24, RowSpacing = 6 };
        table.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        table.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        table.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var row = 0;
        void Add(string first, string second, string third, bool header)
        {
            table.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            var cells = new[] { first, second, third };
            for (var column = 0; column < 3; column++)
            {
                var cell = new TextBlock
                {
                    Text = cells[column],
                    Style = TextStyle(header ? "BodyStrongTextBlockStyle" : "BodyTextBlockStyle"),
                    TextWrapping = TextWrapping.Wrap,
                    HorizontalAlignment = column == 2 ? HorizontalAlignment.Right : HorizontalAlignment.Left,
                };
                Grid.SetRow(cell, row);
                Grid.SetColumn(cell, column);
                table.Children.Add(cell);
            }
            row++;
        }
        Add(Loc.Get("Activity_TableDay"), Loc.Get(IsConsole ? "Console_Outcomes" : "Activity_TableMoods"), Loc.Get(IsConsole ? "Console_Runs" : "Activity_TableWallpapers"), header: true);
        foreach (var (day, stack) in days.OrderByDescending(entry => entry.Key))
        {
            Add(DayName(day), string.Join(", ", stack.Select(part => Loc.Format("Activity_DayPart", NameOf(part.Key), part.Count))), stack.Sum(segment => segment.Count).ToString(System.Globalization.CultureInfo.CurrentCulture), header: false);
        }
        return table;
    }

    private static Style TextStyle(string key) => (Style)Application.Current.Resources[key];

    private static Microsoft.UI.Xaml.Media.Brush Brush(string key) => (Microsoft.UI.Xaml.Media.Brush)Application.Current.Resources[key];

    /// <summary>A high-contrast theme's colour as a brush.</summary>
    private static SolidColorBrush SystemBrush(string key) => new((Color)Application.Current.Resources[key]);

    /// <summary>The plot, read by a screen reader as one picture with a summary (the table has the numbers).</summary>
    private sealed partial class ChartGrid(string summary) : Grid
    {
        protected override AutomationPeer OnCreateAutomationPeer() => new Peer(this, summary);

        private sealed partial class Peer(ChartGrid owner, string summary) : FrameworkElementAutomationPeer(owner)
        {
            protected override AutomationControlType GetAutomationControlTypeCore() => AutomationControlType.Image;

            protected override string GetNameCore() => summary;

            protected override string GetClassNameCore() => nameof(ActivityChart);

            protected override IList<AutomationPeer>? GetChildrenCore() => null;

            protected override bool IsControlElementCore() => true;

            protected override bool IsContentElementCore() => true;
        }
    }
}
