using System.Globalization;

namespace AutoPaper.Services;

/// <summary>
/// The app's strings (Strings\en-US\Resources.resw), for text built in code. XAML uses x:Uid. The app reads them
/// through MRT's ResourceLoader (set in App's constructor, before any page exists); AutoPaper.Tests reads the same
/// .resw file, so the wording in Text and MoodText can be tested without a package. No WinUI or MRT types here, so
/// the test project can link this file.
/// </summary>
internal static class Loc
{
    /// <summary>Looks a string up by its key; empty when there's none.</summary>
    public static Func<string, string> Source { get; set; } = _ => "";

    public static string Get(string key)
    {
        var value = Source(key);
        return string.IsNullOrEmpty(value) ? key : value;
    }

    public static string Format(string key, params object?[] values) =>
        string.Format(CultureInfo.CurrentCulture, Get(key), values);
}
