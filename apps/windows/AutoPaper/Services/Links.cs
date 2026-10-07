namespace AutoPaper.Services;

/// <summary>AutoPaper's pages on its website (GitHub Pages): About's Website · Privacy · Help, Help in the
/// notification-area menu, and F1.</summary>
internal static class Links
{
    public static readonly Uri Website = new("https://msitarzewski.github.io/AutoPaper/");
    public static readonly Uri Privacy = new("https://msitarzewski.github.io/AutoPaper/privacy.html");
    public static readonly Uri Help = new("https://msitarzewski.github.io/AutoPaper/help.html");

    /// <summary>Opens a page in the person's browser.</summary>
    public static async Task OpenAsync(Uri page)
    {
        try
        {
            await Windows.System.Launcher.LaunchUriAsync(page);
        }
        catch (Exception error)
        {
            Log.Error("Opening a web page", error);
        }
    }
}
