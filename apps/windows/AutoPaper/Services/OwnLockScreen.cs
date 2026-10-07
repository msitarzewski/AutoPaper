using System.Security.Cryptography;
using System.Text.Json;

namespace AutoPaper.Services;

/// <summary>What the lock screen showed before AutoPaper: a picture, Windows Spotlight, or a slideshow.</summary>
internal enum LockScreenKind
{
    Picture,
    Spotlight,
    Slideshow,
}

/// <summary>
/// The person's own lock screen (user, 2026-10-06: "Yes" to restoring it, as the desktop's). Before AutoPaper first
/// changes the lock screen, the picture Windows reports for it is copied into AutoPaper's data folder
/// (own-lockscreen\), with the kind of lock screen it was; Quit and Restore my wallpaper put it back. What Windows
/// can't let an app put back is said, never hidden: Windows Spotlight and a slideshow can only be turned on again by
/// the person (Settings › Personalization › Lock screen), and a picture Windows didn't let AutoPaper read can't be
/// put back. Whether the lock screen still shows AutoPaper's picture is told by a fingerprint of what Windows reported
/// right after AutoPaper set it, so a picture the person chose since is never overwritten.
/// No WinUI or WinRT types: AutoPaper.Tests links it (the reading and setting are Desktop's).
/// </summary>
internal sealed class OwnLockScreen(string dataFolder)
{
    /// <summary>What's kept.</summary>
    /// <param name="Copy">The copy of the person's picture, or null when Windows didn't let AutoPaper read it.</param>
    /// <param name="Kind">What the lock screen was.</param>
    /// <param name="AutoPapers">The fingerprint of what Windows reported right after AutoPaper last set the lock
    /// screen (null when it reported nothing).</param>
    /// <param name="Set">AutoPaper has set the lock screen since the person's was kept (or last put back).</param>
    internal sealed record Kept(string? Copy, LockScreenKind Kind, string? AutoPapers, bool Set);

    /// <summary>What putting the person's own lock screen back can do.</summary>
    internal enum Outcome
    {
        /// <summary>Their picture can go back (<see cref="Kept.Copy"/>).</summary>
        Picture,

        /// <summary>It was Windows Spotlight: its last picture goes back (when read), and Spotlight stays off until the
        /// person turns it on.</summary>
        Spotlight,

        /// <summary>It was a slideshow: as Spotlight (its last picture, when read).</summary>
        Slideshow,

        /// <summary>A picture Windows didn't let AutoPaper read: nothing can go back.</summary>
        Unreadable,
    }

    private string Folder => Path.Combine(dataFolder, "own-lockscreen");

    private string IndexFile => Path.Combine(Folder, "own.json");

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

    /// <summary>A fingerprint of a picture's bytes (SHA-256, hex), or null for none.</summary>
    public static string? Fingerprint(byte[]? picture) =>
        picture is { Length: > 0 } ? Convert.ToHexString(SHA256.HashData(picture)) : null;

    /// <summary>The lock screen shows AutoPaper's picture: AutoPaper set it since it last kept or put back the
    /// person's, and (when Windows reports a picture) it's still the one AutoPaper set.</summary>
    public bool ShowsAutoPapers(byte[]? showing, LockScreenKind kind)
    {
        if (Read() is not { Set: true } kept)
        {
            return false;
        }
        if (kind != LockScreenKind.Picture)
        {
            return false; // The person turned on Spotlight or a slideshow since.
        }
        return Fingerprint(showing) is not { } now || kept.AutoPapers is not { } autoPapers || now == autoPapers;
    }

    /// <summary>
    /// Before AutoPaper sets the lock screen: when what's showing is the person's (not AutoPaper's), it's kept (a copy
    /// of the picture Windows reports, if any, and the kind). Returns whether it was kept.
    /// </summary>
    public bool Keep(byte[]? showing, LockScreenKind kind)
    {
        if (ShowsAutoPapers(showing, kind))
        {
            return false;
        }
        var before = Read();
        string? copy = null;
        if (showing is { Length: > 0 })
        {
            Directory.CreateDirectory(Folder);
            copy = Path.Combine(Folder, "lockscreen" + Extension(showing));
            File.WriteAllBytes(copy, showing);
        }
        if (before?.Copy is { } old && !string.Equals(old, copy, StringComparison.OrdinalIgnoreCase))
        {
            TryDelete(old);
        }
        Write(new Kept(copy, kind, null, false));
        return true;
    }

    /// <summary>After AutoPaper set the lock screen: remembers what Windows reports now, so a picture the person picks
    /// later is told apart from AutoPaper's.</summary>
    public void SetByAutoPaper(byte[]? showing)
    {
        var kept = Read() ?? new Kept(null, LockScreenKind.Picture, null, false);
        Write(kept with { AutoPapers = Fingerprint(showing) ?? kept.AutoPapers, Set = true });
    }

    /// <summary>What Quit or Restore my wallpaper should do with the lock screen now, given what's showing: null when
    /// there's nothing to do (nothing kept, AutoPaper hasn't changed it, or the person has chosen another since, which
    /// stays). A person's change also ends the keeping, so it's never overwritten later.</summary>
    public (Kept Kept, Outcome Outcome)? ToRestore(byte[]? showing, LockScreenKind kind)
    {
        if (Read() is not { Set: true } kept)
        {
            return null;
        }
        if (!ShowsAutoPapers(showing, kind))
        {
            Write(kept with { Set = false });
            return null;
        }
        var outcome = kept.Kind switch
        {
            LockScreenKind.Spotlight => Outcome.Spotlight,
            LockScreenKind.Slideshow => Outcome.Slideshow,
            _ when kept.Copy is { } copy && File.Exists(copy) => Outcome.Picture,
            _ => Outcome.Unreadable,
        };
        return (kept, outcome);
    }

    /// <summary>The person's lock screen went back (or couldn't, and that was said): AutoPaper's next one keeps
    /// what's showing then.</summary>
    public void Restored()
    {
        if (Read() is { } kept)
        {
            Write(kept with { Set = false });
        }
    }

    /// <summary>A copy of the kept picture under a new file name (Windows needs a new name for each lock screen
    /// picture), or null.</summary>
    public string? FreshCopy()
    {
        if (Read()?.Copy is not { } copy || !File.Exists(copy))
        {
            return null;
        }
        var fresh = Path.Combine(Folder, $"restore-{DateTimeOffset.UtcNow.ToUnixTimeMilliseconds()}{Path.GetExtension(copy)}");
        foreach (var old in Directory.EnumerateFiles(Folder, "restore-*"))
        {
            TryDelete(old);
        }
        File.Copy(copy, fresh, overwrite: true);
        return fresh;
    }

    private void Write(Kept kept)
    {
        Directory.CreateDirectory(Folder);
        File.WriteAllText(IndexFile, JsonSerializer.Serialize(kept));
    }

    /// <summary>The picture's file type from its first bytes (JPEG, PNG, BMP), so Windows reads the copy as what it is.</summary>
    private static string Extension(byte[] picture) => picture switch
    {
        [0x89, 0x50, 0x4E, 0x47, ..] => ".png",
        [0x42, 0x4D, ..] => ".bmp",
        _ => ".jpg",
    };

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

/// <summary>What putting the person's own lock screen back couldn't do, said in Settings › General (with the link to
/// Windows' lock screen settings) until AutoPaper sets the lock screen again.</summary>
internal enum LockScreenNote
{
    /// <summary>Nothing to say: their picture is back, or AutoPaper hasn't put one back.</summary>
    None,

    /// <summary>It was Windows Spotlight, which only the person can turn on again.</summary>
    Spotlight,

    /// <summary>It was a slideshow, which only the person can turn on again.</summary>
    Slideshow,

    /// <summary>Windows didn't let AutoPaper read their picture, so it couldn't be put back.</summary>
    Unreadable,

    /// <summary>Windows didn't take the picture back.</summary>
    Failed,
}
