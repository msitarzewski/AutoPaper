using System.ComponentModel;
using AutoPaper.Models;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;

namespace AutoPaper.Controls;

/// <summary>
/// Now's reload/stop in a command bar, like a browser's (the macOS app's MakeOrStopButton): New wallpaper now (Ctrl+R,
/// F5) while nothing is being made, Stop (Esc) while one is. One button that changes, so keyboard focus stays on it.
/// In a mood's detail (<see cref="MoodId"/>) it's the same for the mood in use; for another mood it makes that mood
/// current first ("Use this mood and make a new wallpaper"). The window handles the keys (MainWindow), so only one of
/// these is ever on screen and each key does one thing.
/// </summary>
public sealed partial class MakeOrStopButton : AppBarButton
{
    private readonly FontIcon icon = new();

    public MakeOrStopButton()
    {
        Icon = icon;
        AutomationProperties.SetAutomationId(this, "MakeOrStop");
        Click += OnClick;
        Loaded += (_, _) =>
        {
            App.Model.PropertyChanged += OnModelChanged;
            Show();
        };
        Unloaded += (_, _) => App.Model.PropertyChanged -= OnModelChanged;
    }

    /// <summary>The mood whose detail shows the button; null in Now.</summary>
    public string? MoodId
    {
        get;
        set
        {
            field = value;
            Show();
        }
    }

    /// <summary>What the button does now.</summary>
    internal MakeOrStop State
    {
        get
        {
            var model = App.Model;
            var otherMood = MoodId is { } id && model.ActiveMood?.Id != id;
            return MakeOrStop.For(model.IsReady, model.IsGenerating, otherMood);
        }
    }

    private void OnModelChanged(object? sender, PropertyChangedEventArgs args)
    {
        if (args.PropertyName is nameof(AppModel.IsGenerating) or nameof(AppModel.IsReady) or nameof(AppModel.Moods) or nameof(AppModel.ActiveMood))
        {
            Show();
        }
    }

    private void Show()
    {
        var state = State;
        icon.Glyph = state.Glyph;
        Label = state.Label;
        AutomationProperties.SetName(this, state.Label);
        AutomationProperties.SetAcceleratorKey(this, state.Keys);
        ToolTipService.SetToolTip(this, state.Tooltip);
        // Never disabled while it has focus (focus would fall to the title bar): Stop stays enabled while the
        // wallpaper is only being put on the desktop, and says it's too late (AppModel.Cancel).
        IsEnabled = state.IsEnabled;
    }

    private async void OnClick(object sender, RoutedEventArgs e) => await InvokeAsync();

    /// <summary>What the button (and Ctrl+R, F5 or Esc for it) does now.</summary>
    internal Task InvokeAsync()
    {
        var model = App.Model;
        var state = State;
        if (!state.IsEnabled)
        {
            return Task.CompletedTask;
        }
        if (state.IsStop)
        {
            model.Cancel();
            return Task.CompletedTask;
        }
        return MoodId is { } id ? model.NewWallpaperFromMoodAsync(id) : model.NewWallpaperAsync();
    }
}
