using System.Xml.Linq;
using AutoPaper.Services;

namespace AutoPaper.Tests;

/// <summary>The app's English strings, read from Resources.resw (copied next to the tests) the way MRT serves them:
/// "Key", or "Uid/Property" for an x:Uid entry stored as "Uid.Property". Loc reads through it in tests.</summary>
internal static class ReswStrings
{
    private static readonly Lazy<Dictionary<string, string>> Values = new(() =>
        XDocument.Load(Path.Combine(AppContext.BaseDirectory, "Resources.resw"))
            .Root!.Elements("data")
            .ToDictionary(data => (string)data.Attribute("name")!, data => (string?)data.Element("value") ?? ""));

    public static IReadOnlyDictionary<string, string> All => Values.Value;

    public static string Get(string key)
    {
        if (Values.Value.TryGetValue(key, out var value))
        {
            return value;
        }
        var slash = key.IndexOf('/');
        return slash > 0 && Values.Value.TryGetValue(key[..slash] + "." + key[(slash + 1)..], out var uid) ? uid : "";
    }

    /// <summary>Points Loc at the .resw (every test class that words things calls this first).</summary>
    public static void Use() => Loc.Source = Get;
}
