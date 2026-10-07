using AutoPaper.Services;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Navigation;
using Windows.ApplicationModel;

namespace AutoPaper.Views.Panes;

/// <summary>Settings › About: icon, name, version (build), what it is, the licence.</summary>
public sealed partial class AboutPage : Page
{
    public AboutPage()
    {
        InitializeComponent();
        var version = Package.Current.Id.Version;
        VersionLine.Text = Loc.Format("About_Version", $"{version.Major}.{version.Minor}.{version.Build}", version.Revision);
    }

    protected override void OnNavigatedTo(NavigationEventArgs e)
    {
        base.OnNavigatedTo(e);
        PaneHeader.Attach(Crumbs, "About", Frame);
    }
}
