using System.ComponentModel;
using AutoPaper.Core;
using AutoPaper.Controls;
using AutoPaper.Models;
using AutoPaper.Services;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Windows.ApplicationModel.DataTransfer;
using Windows.Storage;
using Windows.Storage.Pickers;

namespace AutoPaper.Views;

/// <summary>Local Console: dates → runs and outcomes, with the selected run's exact requests, responses and retries.
/// Unlike History's image grid, it includes runs that never created an image. Active runs refresh while the view is
/// open; finished traces stay still so reading, text selection and expanded events are not disturbed.</summary>
public sealed partial class ConsolePage : Page, IDefaultFocus
{
    private const uint PageSize = 100;
    private readonly DispatcherQueueTimer poll;
    private readonly Dictionary<TreeViewNode, string> runIds = [];
    private readonly Dictionary<string, RunRecord> runs = new(StringComparer.Ordinal);
    private bool refreshing;
    private bool rebuilding;
    private bool clearing;
    private bool loadedOlder;
    private string? selectedId;
    private string? selectedReport;
    private RunRecord? selectedRun;
    private int selectionVersion;
    private string? listSignature;
    private int listVersion;
    private string? statisticsSignature;
    private bool hadStatistics;

    public ConsolePage()
    {
        InitializeComponent();
        poll = DispatcherQueue.CreateTimer();
        poll.Interval = TimeSpan.FromSeconds(3);
        poll.Tick += async (_, _) => await RefreshAsync();
        Loaded += async (_, _) =>
        {
            RefreshButton.IsEnabled = Model.IsReady;
            ClearButton.IsEnabled = Model.IsReady && runs.Count > 0 && !Model.IsGenerating;
            Model.PropertyChanged += OnModelChanged;
            Model.HistoryChanged += OnHistoryChanged;
            await RefreshAsync();
            poll.Start();
        };
        Unloaded += (_, _) =>
        {
            poll.Stop();
            Model.PropertyChanged -= OnModelChanged;
            Model.HistoryChanged -= OnHistoryChanged;
            selectionVersion++;
        };
    }

    private static AppModel Model => App.Model;
    public UIElement? DefaultFocusElement => runs.Count > 0 ? RunTree : RefreshButton;

    private void OnModelChanged(object? sender, PropertyChangedEventArgs args)
    {
        if (args.PropertyName is nameof(AppModel.IsReady) or nameof(AppModel.IsGenerating))
        {
            ClearButton.IsEnabled = Model.IsReady && runs.Count > 0 && !Model.IsGenerating;
            _ = RefreshAsync();
        }
    }

    private async void OnHistoryChanged(object? sender, EventArgs args) => await RefreshAsync();

    private async Task RefreshAsync(bool announce = false)
    {
        if (refreshing || clearing || !Model.IsReady || !IsLoaded)
        {
            return;
        }
        refreshing = true;
        RefreshButton.IsEnabled = false;
        try
        {
            var version = listVersion;
            var fetchOlder = loadedOlder;
            var (latest, statistics) = await Model.Call(engine =>
            {
                var first = engine.Runs(PageSize, 0).ToList();
                if (fetchOlder)
                {
                    first.AddRange(engine.Runs(PageSize, PageSize));
                }
                // A run may start between page reads and shift the offset; keep one node for each identity.
                return (first.DistinctBy(run => run.Id).ToList(), engine.ConsoleStatistics());
            });
            if (!IsLoaded || version != listVersion)
            {
                return;
            }
            runs.Clear();
            foreach (var run in latest)
            {
                runs[run.Id] = run;
            }
            ShowStatistics(statistics);
            // Change nodes only when their metadata changes; expanding a date and keyboard focus survive polling.
            var signature = string.Join("\n", latest.Select(run => $"{run.Id}|{run.StartedAt}|{run.Status}|{run.FinishedAt}|{run.MoodName}"));
            if (signature != listSignature)
            {
                listSignature = signature;
                RebuildTree(latest);
            }
            OlderButton.Visibility = !loadedOlder && latest.Count == PageSize ? Visibility.Visible : Visibility.Collapsed;
            EmptyText.Visibility = latest.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
            RunsPane.Visibility = latest.Count == 0 ? Visibility.Collapsed : Visibility.Visible;
            DetailsPane.Visibility = latest.Count == 0 ? Visibility.Collapsed : Visibility.Visible;
            if (latest.Count == 0) UpdateEmptyHeight();
            else Split.Height = 440;
            ClearButton.IsEnabled = latest.Count > 0 && !Model.IsGenerating;
            if (selectedId is null && latest.Count > 0)
            {
                SelectNode(latest[0].Id);
                await SelectAsync(latest[0].Id);
            }
            else if (selectedId is { } id)
            {
                if (!runs.TryGetValue(id, out var newest))
                {
                    ClearSelection();
                    if (latest.Count > 0)
                    {
                        SelectNode(latest[0].Id);
                        await SelectAsync(latest[0].Id);
                    }
                }
                else if (selectedRun is null || selectedRun.Status == RunStatus.Running || newest.Status != selectedRun.Status)
                {
                    await SelectAsync(id);
                }
            }
            if (announce)
            {
                ShowStatus(Loc.Get("Console_Refreshed"));
                await Model.RefreshStatusAsync();
            }
        }
        catch (Exception error)
        {
            ShowStatus(Text.Error(error, Model.Settings), error: true, announce: announce);
        }
        finally
        {
            refreshing = false;
            RefreshButton.IsEnabled = Model.IsReady;
        }
    }

    private void ShowStatistics(ConsoleStatistics statistics)
    {
        var signature = System.Text.Json.JsonSerializer.Serialize(statistics);
        if (statisticsSignature == signature) return;
        statisticsSignature = signature;
        StatTiles.Children.Clear();
        string[] captions = [Loc.Get("Console_Runs"), Loc.Get("Console_SuccessRate"), Loc.Get("Console_AverageRun")];
        string[] glyphs = ["\uE9D9", "\uE73E", "\uE916"];
        string[] values = [statistics.Total.ToString("N0", System.Globalization.CultureInfo.CurrentCulture),
            Text.RunRate(statistics.SuccessRate), Text.RunSeconds(statistics.AverageRunSecs)];
        for (var index = 0; index < captions.Length; index++)
            StatTiles.Children.Add(StatTile.Create(captions[index], glyphs[index], values[index], $"{captions[index]}: {values[index]}"));
        StatTile.Arrange(StatTiles, StatTiles.ActualWidth, 160);
        OutcomeChart.ShowOutcomes(statistics.Days);
        if (statistics.Total == 0) Overview.IsExpanded = false;
        else if (!hadStatistics) Overview.IsExpanded = true;
        hadStatistics = statistics.Total > 0;

        ModelBars.Children.Clear();
        if (statistics.Models.Length == 0)
        {
            var area = new Grid { MinHeight = 190 };
            area.Children.Add(new TextBlock { Text = Loc.Get("Console_NoTimings"), TextWrapping = TextWrapping.Wrap,
                HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center, TextAlignment = TextAlignment.Center,
                Foreground = (Brush)Application.Current.Resources["TextFillColorSecondaryBrush"] });
            ModelBars.Children.Add(area);
            return;
        }
        var maximum = Math.Max(0.000001, statistics.Models.Max(model => model.AverageSecs));
        foreach (var model in statistics.Models.OrderByDescending(model => model.AverageSecs))
        {
            var name = Loc.Format("Console_ModelJob", model.Job == ProviderJob.Concepts ? Loc.Get("Console_JobWriting") : Loc.Get("Console_JobPainting"),
                Text.RunProvider(model.Provider, model.Model));
            var row = new StackPanel { Spacing = 5 };
            row.Children.Add(new TextBlock { Text = name, TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true });
            row.Children.Add(new TextBlock { Text = Loc.Format("Console_CallTime", Text.RunSeconds(model.AverageSecs), model.Calls),
                Style = (Style)Application.Current.Resources["CaptionTextBlockStyle"], IsTextSelectionEnabled = true });
            var bar = new ProgressBar { Minimum = 0, Maximum = maximum, Value = model.AverageSecs, Height = 6 };
            AutomationProperties.SetAccessibilityView(bar, AccessibilityView.Raw);
            row.Children.Add(bar);
            ModelBars.Children.Add(row);
        }
    }

    private void OnStatTilesSizeChanged(object sender, SizeChangedEventArgs args)
    {
        StatTile.Arrange(StatTiles, args.NewSize.Width, 160);
        UpdateEmptyHeight();
    }

    private void OnEmptyLayoutSizeChanged(object sender, SizeChangedEventArgs args) => UpdateEmptyHeight();

    private void UpdateEmptyHeight()
    {
        if (runs.Count == 0)
            Split.Height = Math.Max(64, ContentScroll.ActualHeight - StatTiles.ActualHeight - Overview.ActualHeight - 44);
    }

    private void OnChartsSizeChanged(object sender, SizeChangedEventArgs args)
    {
        var narrow = args.NewSize.Width < 640;
        ModelsColumn.Width = narrow ? new GridLength(0) : new GridLength(1, GridUnitType.Star);
        Grid.SetColumn(ModelChart, narrow ? 0 : 1);
        Grid.SetRow(ModelChart, narrow ? 1 : 0);
    }

    private void RebuildTree(IReadOnlyList<RunRecord> records)
    {
        var expanded = RunTree.RootNodes.Where(node => node.IsExpanded).Select(node => node.Content?.ToString()).ToHashSet();
        var wasEmpty = RunTree.RootNodes.Count == 0;
        rebuilding = true;
        RunTree.RootNodes.Clear();
        runIds.Clear();
        foreach (var day in records.GroupBy(run => DateTimeOffset.FromUnixTimeSeconds(run.StartedAt).ToLocalTime().Date))
        {
            var label = Text.Date(new DateTimeOffset(day.Key));
            var date = new TreeViewNode { Content = label, IsExpanded = wasEmpty || expanded.Contains(label) };
            foreach (var run in day)
            {
                var time = Text.Time(DateTimeOffset.FromUnixTimeSeconds(run.StartedAt).ToLocalTime());
                var node = new TreeViewNode { Content = Loc.Format("Console_Row", OutcomeGlyph(run.Status), time, Text.RunOutcome(run.Status), run.MoodName) };
                date.Children.Add(node);
                runIds[node] = run.Id;
            }
            RunTree.RootNodes.Add(date);
        }
        if (selectedId is { } id)
        {
            SelectNode(id);
        }
        rebuilding = false;
    }

    private void SelectNode(string id)
    {
        var node = runIds.FirstOrDefault(pair => pair.Value == id).Key;
        if (node is not null)
        {
            var wasRebuilding = rebuilding;
            rebuilding = true;
            node.Parent.IsExpanded = true;
            RunTree.SelectedNode = node;
            rebuilding = wasRebuilding;
        }
    }

    private async void OnSelectionChanged(TreeView sender, TreeViewSelectionChangedEventArgs args)
    {
        if (!rebuilding && sender.SelectedNode is { } node && runIds.TryGetValue(node, out var id))
        {
            await SelectAsync(id);
        }
    }

    private async Task SelectAsync(string id)
    {
        var version = ++selectionVersion;
        var changed = selectedId != id;
        selectedId = id;
        if (changed)
        {
            selectedReport = null;
            selectedRun = null;
            CopyButton.IsEnabled = ExportButton.IsEnabled = false;
        }
        try
        {
            var (run, report) = await Model.Call(engine => (engine.Run(id), engine.RunReport(id)));
            if (version != selectionVersion || !IsLoaded)
            {
                return;
            }
            ShowRun(run);
            selectedReport = report;
            CopyButton.IsEnabled = ExportButton.IsEnabled = true;
            if (changed)
            {
                DetailsPane.ChangeView(null, 0, null, disableAnimation: true);
            }
        }
        catch (Exception error)
        {
            if (version == selectionVersion && IsLoaded)
            {
                ShowStatus(Text.Error(error, Model.Settings), error: true);
            }
        }
    }

    private void ShowRun(RunRecord run)
    {
        SelectionTitle.Text = Loc.Format("Console_Selection", Text.RunOutcome(run.Status), run.MoodName);
        var at = DateTimeOffset.FromUnixTimeSeconds(run.StartedAt).ToLocalTime();
        var lines = new List<string>
        {
            Loc.Format("Console_Started", Text.Date(at), at.ToString("T", System.Globalization.CultureInfo.CurrentCulture)),
            Loc.Format("Console_Trigger", Text.RunTrigger(run.Trigger)),
            Loc.Format("Console_Writing", Text.RunProvider(run.TextProvider, run.TextModel)),
            Loc.Format("Console_Painting", Text.RunProvider(run.ImageProvider, run.ImageModel)),
            Loc.Format("Console_Surprise", Math.Round(run.Surprise * 100).ToString("0")),
            Loc.Format("Console_Cost", Text.RunMoney(run.CostMicrousd)),
        };
        if (run.FinishedAt is { } finish)
        {
            lines.Add(Loc.Format("Console_Duration", Math.Max(0, finish - run.StartedAt)));
        }
        lines.Add(Loc.Format("Console_Keywords", run.Keywords.Length == 0 ? Loc.Get("Console_None") :
            string.Join("; ", run.Keywords.Select(word => $"{MoodText.WeightWord(word.Weight)}: {word.Text}"))));
        lines.Add(Loc.Format("Console_Id", run.Id));
        if (run.GenerationId is { } generationId)
        {
            lines.Add(Loc.Format("Console_GenerationId", generationId));
        }
        Summary.Text = string.Join("\n", lines);
        OutcomeDetail.Text = run.Detail;
        OutcomeBar.Severity = run.Status switch
        {
            RunStatus.Succeeded => InfoBarSeverity.Success,
            RunStatus.Failed => InfoBarSeverity.Error,
            RunStatus.Blocked or RunStatus.Interrupted => InfoBarSeverity.Warning,
            _ => InfoBarSeverity.Informational,
        };
        OutcomeBar.Title = Text.RunOutcome(run.Status);
        OutcomeBar.Visibility = string.IsNullOrEmpty(run.Detail) ? Visibility.Collapsed : Visibility.Visible;
        OutcomeBar.IsOpen = OutcomeBar.Visibility == Visibility.Visible;
        // Append new events to a running trace instead of replacing open expanders and selected text.
        var oldEvents = selectedRun?.Events;
        var append = selectedRun?.Id == run.Id && oldEvents is not null && run.Events.Length >= oldEvents.Length
            && oldEvents.SequenceEqual(run.Events.Take(oldEvents.Length));
        if (!append)
        {
            Trace.Children.Clear();
        }
        foreach (var entry in run.Events.Skip(append ? oldEvents!.Length : 0))
        {
            var when = DateTimeOffset.FromUnixTimeSeconds(entry.At).ToLocalTime().ToString("T", System.Globalization.CultureInfo.CurrentCulture);
            var who = entry.Provider is { } provider ? Text.RunProvider(provider, entry.Model) : "";
            var heading = string.Join(" · ", new[] { when, entry.Stage, entry.Kind, who }.Where(value => value.Length > 0));
            if (string.IsNullOrEmpty(entry.Detail))
            {
                Trace.Children.Add(new TextBlock { Text = heading, TextWrapping = TextWrapping.Wrap });
                continue;
            }
            var body = new TextBlock
            {
                Text = entry.Detail,
                IsTextSelectionEnabled = true,
                TextWrapping = TextWrapping.Wrap,
                FontFamily = new FontFamily("Cascadia Mono, Consolas"),
                FontSize = 13,
            };
            Trace.Children.Add(new Expander
            {
                Header = new TextBlock { Text = heading, TextWrapping = TextWrapping.Wrap },
                Content = body,
                HorizontalAlignment = HorizontalAlignment.Stretch,
                HorizontalContentAlignment = HorizontalAlignment.Stretch,
                IsExpanded = true,
            });
        }
        NoEvents.Visibility = run.Events.Length == 0 ? Visibility.Visible : Visibility.Collapsed;
        selectedRun = run;
    }

    private static string OutcomeGlyph(RunStatus status) => status switch
    {
        RunStatus.Running => "◷",
        RunStatus.Succeeded => "✓",
        RunStatus.Failed => "✕",
        RunStatus.Blocked => "!",
        RunStatus.Cancelled => "–",
        RunStatus.Interrupted => "!",
        _ => "",
    };

    private void ClearSelection()
    {
        selectionVersion++;
        selectedId = null;
        selectedRun = null;
        selectedReport = null;
        SelectionTitle.Text = Loc.Get("ConsoleChooseRun/Text");
        Summary.Text = "";
        Trace.Children.Clear();
        OutcomeBar.IsOpen = false;
        OutcomeBar.Visibility = NoEvents.Visibility = Visibility.Collapsed;
        CopyButton.IsEnabled = ExportButton.IsEnabled = false;
    }

    private async void OnRefresh(object sender, RoutedEventArgs args) => await RefreshAsync(announce: true);

    private async void OnOlder(object sender, RoutedEventArgs args)
    {
        loadedOlder = true;
        await RefreshAsync();
    }

    private void OnCopy(object sender, RoutedEventArgs args)
    {
        if (selectedReport is not { } report)
        {
            return;
        }
        try
        {
            var package = new DataPackage();
            package.SetText(report);
            Clipboard.SetContent(package);
            Clipboard.Flush();
            ShowStatus(Loc.Get("Console_Copied"));
        }
        catch (Exception error)
        {
            ShowStatus(Text.Error(error, Model.Settings), error: true);
        }
    }

    private async void OnExport(object sender, RoutedEventArgs args)
    {
        if (selectedReport is not { } report || selectedRun is not { } run)
        {
            return;
        }
        try
        {
            var picker = new FileSavePicker
            {
                SuggestedStartLocation = PickerLocationId.DocumentsLibrary,
                SuggestedFileName = "AutoPaper-run-" + run.Id,
            };
            picker.FileTypeChoices.Add(Loc.Get("Console_Json"), [".json"]);
            WinRT.Interop.InitializeWithWindow.Initialize(picker, WinRT.Interop.WindowNative.GetWindowHandle(App.Window));
            if (await picker.PickSaveFileAsync() is { } file)
            {
                await FileIO.WriteTextAsync(file, report);
                ShowStatus(Loc.Get("Console_Exported"));
            }
        }
        catch (Exception error)
        {
            ShowStatus(Text.Error(error, Model.Settings), error: true);
        }
    }

    private async void OnClear(object sender, RoutedEventArgs args)
    {
        if (!Model.IsReady || Model.IsGenerating)
        {
            return;
        }
        var dialog = new ContentDialog
        {
            Title = Loc.Get("Console_ClearTitle"),
            Content = Loc.Get("Console_ClearBody"),
            PrimaryButtonText = Loc.Get("Console_ClearConfirm"),
            CloseButtonText = Loc.Get("Dialog_Cancel"),
            DefaultButton = ContentDialogButton.Close,
        };
        if (await Dialogs.ShowAsync(dialog, this) != ContentDialogResult.Primary)
        {
            return;
        }
        try
        {
            clearing = true;
            listVersion++;
            await Model.Call(engine => engine.ClearRuns());
            ClearSelection();
            RunTree.RootNodes.Clear();
            runIds.Clear();
            runs.Clear();
            listSignature = null;
            loadedOlder = false;
            ShowStatus(Loc.Get("Console_Cleared"));
        }
        catch (Exception error)
        {
            ShowStatus(Text.Error(error, Model.Settings), error: true);
        }
        finally
        {
            clearing = false;
        }
        await RefreshAsync();
    }

    private void ShowStatus(string message, bool error = false, bool announce = true)
    {
        StatusBar.Message = message;
        StatusBar.Severity = error ? InfoBarSeverity.Error : InfoBarSeverity.Informational;
        StatusBar.Visibility = Visibility.Visible;
        StatusBar.IsOpen = true;
        if (announce)
        {
            if (error) Model.AnnounceError(message);
            else Model.Announce(message);
        }
    }

    private void OnStatusClosed(InfoBar sender, InfoBarClosedEventArgs args) => sender.Visibility = Visibility.Collapsed;

    private void OnSplitSizeChanged(object sender, SizeChangedEventArgs args)
    {
        var narrow = args.NewSize.Width < 640;
        RunsColumn.Width = narrow ? new GridLength(1, GridUnitType.Star) : new GridLength(300);
        DetailsColumn.Width = narrow ? new GridLength(0) : new GridLength(1, GridUnitType.Star);
        RunsRow.Height = narrow && runs.Count > 0 ? new GridLength(220) : new GridLength(1, GridUnitType.Star);
        DetailsRow.Height = narrow && runs.Count > 0 ? new GridLength(1, GridUnitType.Star) : new GridLength(0);
        Grid.SetColumn(DetailsPane, narrow ? 0 : 1);
        Grid.SetRow(DetailsPane, narrow ? 1 : 0);
        Split.ColumnSpacing = narrow ? 0 : 20;
    }
}
