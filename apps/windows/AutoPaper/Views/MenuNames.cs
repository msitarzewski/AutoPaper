using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;

namespace AutoPaper.Views;

/// <summary>Gives a MenuFlyout a spoken name (its presenter is the UI Automation menu element).</summary>
internal static class MenuNames
{
    public static void Name(MenuFlyout menu, string name)
    {
        var style = new Style(typeof(MenuFlyoutPresenter));
        style.Setters.Add(new Setter(AutomationProperties.NameProperty, name));
        menu.MenuFlyoutPresenterStyle = style;
    }
}
