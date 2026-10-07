using System.Security.Cryptography;
using System.Text;
using System.Text.Json;

namespace AutoPaper.Services;

/// <summary>The kind of background the person had: pictures (or a colour), a slideshow, or Windows Spotlight.</summary>
internal enum BackgroundKind
{
    Picture,
    Slideshow,
    Spotlight,
}

/// <summary>A slideshow (IDesktopWallpaper): its folders or pictures (file system paths), its options
/// (DESKTOP_SLIDESHOW_OPTIONS: shuffle) and how often it changes, in milliseconds.</summary>
internal sealed record SlideshowSetting(IReadOnlyList<string> Items, int Options, uint TickMs);

/// <summary>
/// The person's own wallpaper (docs/app-spec.md 3a, user 2026-10-06): Windows has no supported window layer between
/// the desktop picture and the icons, so AutoPaper sets the real wallpaper, but first keeps a copy of the picture
/// each monitor showed that wasn't AutoPaper's (with its position and the background colour), and puts it back when
/// AutoPaper quits or the person chooses Restore my wallpaper (pausing keeps AutoPaper's showing). The copies live in AutoPaper's data folder
/// (own-wallpaper\), so a picture that's later deleted, or a Windows Spotlight one that rotates away, still comes back.
/// The kind of background is kept too: a slideshow is put back as the slideshow it was; Windows Spotlight, which
/// Windows switches off when an app sets a picture and doesn't let an app switch on, comes back as its last picture,
/// and AutoPaper says so with a link to turn it back on (AppModel).
/// No WinUI types: AutoPaper.Tests links it.
/// </summary>
internal sealed class OwnWallpaper(string dataFolder)
{
    /// <summary>What's kept: each monitor's copy (empty: it showed no picture, only the colour), the file it was
    /// copied from, the position and the colour.</summary>
    internal sealed record Kept(
        Dictionary<string, string> Pictures,
        int Position,
        uint Color,
        Dictionary<string, string>? Originals = null,
        BackgroundKind Kind = BackgroundKind.Picture,
        SlideshowSetting? Slideshow = null);

    private string Folder => Path.Combine(dataFolder, "own-wallpaper");

    private string IndexFile => Path.Combine(Folder, "own.json");

    /// <summary>A picture AutoPaper put on the desktop (its renders, and the copies it restores, are in its data
    /// folder); anything else is the person's own.</summary>
    public static bool IsAutoPapers(string path, string dataFolder) =>
        path.StartsWith(dataFolder.TrimEnd('\\', '/') + Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase);

    public Kept? Read()
    {
        try
        {
            return File.Exists(IndexFile) ? JsonSerializer.Deserialize<Kept>(File.ReadAllText(IndexFile)) : null;
        }
        catch (Exception error) when (error is IOException or JsonException or UnauthorizedAccessException)
        {
            return null;
        }
    }

    public bool HasCopy => Read() is { Pictures.Count: > 0 };

    /// <summary>
    /// Before AutoPaper replaces the desktop: each monitor showing a picture that isn't AutoPaper's (or none) is the
    /// person's own, and its picture is copied and kept, with the position and colour then in use, and the kind of
    /// background (<paramref name="kind"/>, with the <paramref name="slideshow"/> when it's one). Monitors already
    /// showing AutoPaper's keep what was kept for them. Returns whether anything new was kept. A file that can't be
    /// read is skipped (what was kept before stays).
    /// </summary>
    public bool Keep(IReadOnlyDictionary<string, string> showing, int position, uint color,
        BackgroundKind kind = BackgroundKind.Picture, SlideshowSetting? slideshow = null)
    {
        var kept = Read();
        // Windows Spotlight's last picture, put back by AutoPaper, is still showing as a plain picture (Spotlight is
        // off and the person hasn't chosen anything else): it's still their Spotlight background.
        if (kind == BackgroundKind.Picture && kept is { Kind: BackgroundKind.Spotlight } && ShowsWhatWasKept(showing, kept))
        {
            kind = BackgroundKind.Spotlight;
        }
        var pictures = kept is null ? new Dictionary<string, string>() : new Dictionary<string, string>(kept.Pictures);
        var originals = kept?.Originals is { } keptOriginals ? new Dictionary<string, string>(keptOriginals) : new Dictionary<string, string>();
        var changed = false;
        foreach (var (monitor, path) in showing)
        {
            if (path.Length > 0 && IsAutoPapers(path, dataFolder))
            {
                continue;
            }
            var copy = "";
            if (path.Length > 0)
            {
                if (!File.Exists(path))
                {
                    continue;
                }
                Directory.CreateDirectory(Folder);
                copy = Path.Combine(Folder, FileName(monitor) + Path.GetExtension(path));
                try
                {
                    File.Copy(path, copy, overwrite: true);
                }
                catch (Exception error) when (error is IOException or UnauthorizedAccessException)
                {
                    continue;
                }
                if (pictures.TryGetValue(monitor, out var before) && before.Length > 0 && !string.Equals(before, copy, StringComparison.OrdinalIgnoreCase))
                {
                    TryDelete(before);
                }
            }
            pictures[monitor] = copy;
            originals[monitor] = path;
            changed = true;
        }
        if (!changed)
        {
            return false;
        }
        Directory.CreateDirectory(Folder);
        // The position, colour and kind are the person's when one of their pictures was showing.
        File.WriteAllText(IndexFile, JsonSerializer.Serialize(
            new Kept(pictures, position, color, originals, kind, kind == BackgroundKind.Slideshow ? slideshow : null)));
        return true;
    }

    /// <summary>Every monitor that isn't showing AutoPaper's shows the picture kept for it (the original or the copy).</summary>
    private bool ShowsWhatWasKept(IReadOnlyDictionary<string, string> showing, Kept kept) =>
        showing.All(entry =>
            (entry.Value.Length > 0 && IsAutoPapers(entry.Value, dataFolder))
            || (kept.Pictures.TryGetValue(entry.Key, out var copy) && string.Equals(copy, entry.Value, StringComparison.OrdinalIgnoreCase))
            || (kept.Originals?.GetValueOrDefault(entry.Key) is { } original && string.Equals(original, entry.Value, StringComparison.OrdinalIgnoreCase)));

    /// <summary>What to put back, per kept monitor: the person's own file where it still is (so their desktop doesn't
    /// depend on AutoPaper's folder), else the copy, else nothing (the colour shows).</summary>
    public Kept? ToRestore() => Read() is { } kept
        ? kept with
        {
            Pictures = kept.Pictures.ToDictionary(entry => entry.Key, entry =>
                kept.Originals?.GetValueOrDefault(entry.Key) is { Length: > 0 } original && File.Exists(original) ? original
                : entry.Value.Length > 0 && File.Exists(entry.Value) ? entry.Value
                : ""),
        }
        : null;

    private static string FileName(string monitor) =>
        "monitor-" + Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(monitor)))[..16].ToLowerInvariant();

    private static void TryDelete(string path)
    {
        try
        {
            File.Delete(path);
        }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException)
        {
            // Left behind; replaced next time.
        }
    }
}
