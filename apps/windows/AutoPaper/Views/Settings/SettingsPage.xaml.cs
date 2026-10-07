using AutoPaper.Services;
using AutoPaper.Views;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media.Animation;
using Microsoft.UI.Xaml.Navigation;

namespace AutoPaper.Views.Panes;

/// <summary>The Settings home: one card per pane (General, Providers, Keys, Memory, Budget, About).</summary>
public sealed partial class SettingsPage : Page, IDefaultFocus
{
    /// <summary>The pane the person came back from (its card gets focus again), or null.</summary>
    private string? cameBackFrom;

    public SettingsPage()
    {
        InitializeComponent();
    }

    /// <summary>Back from a pane: that pane's card, as Windows Settings does; otherwise the first card.</summary>
    public UIElement? DefaultFocusElement => cameBackFrom is { } pane
        ? Column.Children.OfType<FrameworkElement>().FirstOrDefault(card => card.Tag as string == pane)
        : null;

    /// <summary>The page for a pane's name (Preferences.SettingsPane, notification buttons).</summary>
    public static Type? PageFor(string pane) => pane switch
    {
        "General" => typeof(GeneralPage),
        "Providers" => typeof(ProvidersPage),
        "Keys" => typeof(KeysPage),
        "Memory" => typeof(MemoryPage),
        "Budget" => typeof(BudgetPage),
        "About" => typeof(AboutPage),
        _ => null,
    };

    protected override void OnNavigatedTo(NavigationEventArgs e)
    {
        base.OnNavigatedTo(e);
        // Back on the home page: Settings reopen here next time.
        if (e.NavigationMode == NavigationMode.Back)
        {
            cameBackFrom = Preferences.SettingsPane;
            Preferences.SettingsPane = "";
        }
    }

    private void OnOpen(object? sender, EventArgs e)
    {
        if (sender is FrameworkElement { Tag: string pane } && PageFor(pane) is { } page)
        {
            try
            {
                Frame.Navigate(page, null, new SlideNavigationTransitionInfo { Effect = SlideNavigationTransitionEffect.FromRight });
            }
            catch (Exception error)
            {
                Log.Error($"Opening Settings > {pane}", error);
            }
        }
    }
}
