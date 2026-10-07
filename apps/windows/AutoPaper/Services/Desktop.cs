using System.Runtime.InteropServices;
using Microsoft.Win32;
using Windows.Storage;
using Windows.System.UserProfile;
using Windows.Win32;
using Windows.Win32.Foundation;
using Windows.Win32.UI.Shell;
using Windows.Win32.UI.Shell.Common;

namespace AutoPaper.Services;

/// <summary>A connected monitor: the shell's device path (stable per monitor) and its size in physical pixels.</summary>
internal sealed record DisplayInfo(string Id, uint Width, uint Height, bool IsPrimary);

/// <summary>What the desktop shows: each monitor's picture path (empty for none), the position (DESKTOP_WALLPAPER_POSITION),
/// the background colour (COLORREF), and the kind of background (with the slideshow, when it's one).</summary>
internal sealed record DesktopPictures(
    IReadOnlyDictionary<string, string> Pictures,
    int Position,
    uint Color,
    BackgroundKind Kind = BackgroundKind.Picture,
    SlideshowSetting? Slideshow = null);

internal enum LockScreenOutcome
{
    Set,

    /// <summary>A Group Policy or MDM setting fixes the lock screen picture: the organisation manages it.</summary>
    Managed,

    /// <summary>No policy, but Windows didn't take the picture (both APIs refused, or the file couldn't be opened):
    /// said neutrally, since nothing says it's the organisation.</summary>
    Failed,
}

/// <summary>
/// The desktop wallpaper per monitor (IDesktopWallpaper, the shell's own API) and the lock screen
/// (UserProfilePersonalizationSettings, then LockScreen.SetImageFileAsync).
/// Image paths are real paths under the package's LocalState (ApplicationData.Current.LocalFolder), which
/// Explorer can read: a packaged app's writes under %LOCALAPPDATA% would be redirected where it can't
/// (docs/research/windows.md, "Wallpaper and lock screen").
/// </summary>
internal sealed class Desktop : IDisposable
{
    private readonly StaWorker worker = new("AutoPaper desktop");
    private IDesktopWallpaper? wallpaper;

    private IDesktopWallpaper Wallpaper => wallpaper ??= (IDesktopWallpaper)new DesktopWallpaper();

    /// <summary>The monitors that are on, in the shell's order. Empty only if the shell can't say.</summary>
    public Task<IReadOnlyList<DisplayInfo>> DisplaysAsync() => worker.Run<IReadOnlyList<DisplayInfo>>(ReadDisplays);

    private unsafe IReadOnlyList<DisplayInfo> ReadDisplays()
    {
        var found = new List<DisplayInfo>();
        Wallpaper.GetMonitorDevicePathCount(out var count);
        for (uint index = 0; index < count; index++)
        {
            PWSTR path;
            Wallpaper.GetMonitorDevicePathAt(index, &path);
            string id;
            try
            {
                id = path.ToString();
            }
            finally
            {
                Marshal.FreeCoTaskMem((nint)path.Value);
            }
            RECT rect;
            try
            {
                fixed (char* monitor = id)
                {
                    Wallpaper.GetMonitorRECT(monitor, &rect);
                }
            }
            catch (COMException)
            {
                continue; // A monitor that's known but not attached.
            }
            var width = rect.right - rect.left;
            var height = rect.bottom - rect.top;
            if (width <= 0 || height <= 0)
            {
                continue;
            }
            found.Add(new DisplayInfo(id, (uint)width, (uint)height, rect.left == 0 && rect.top == 0));
        }
        if (found.Count > 0 && !found.Any(display => display.IsPrimary))
        {
            found[0] = found[0] with { IsPrimary = true };
        }
        return found;
    }

    /// <summary>Sets each monitor's wallpaper (images rendered at its exact size, so Fill neither crops nor scales).
    /// A null monitor id sets every monitor.</summary>
    public Task SetWallpapersAsync(IReadOnlyList<(string? MonitorId, string Path)> images) => worker.Run(() => SetWallpapers(images));

    private unsafe void SetWallpapers(IReadOnlyList<(string? MonitorId, string Path)> images)
    {
        Wallpaper.SetPosition(DESKTOP_WALLPAPER_POSITION.DWPOS_FILL);
        foreach (var (monitorId, path) in images)
        {
            fixed (char* monitor = monitorId)
            fixed (char* image = path)
            {
                Wallpaper.SetWallpaper(monitor, image);
            }
        }
    }

    /// <summary>What each monitor shows now: the shell's picture path per monitor (empty for a solid colour), the
    /// picture position (fill, fit…), the background colour, and whether it's Windows Spotlight or a slideshow.</summary>
    public Task<DesktopPictures> ReadAsync() => worker.Run(ReadPictures);

    private unsafe DesktopPictures ReadPictures()
    {
        var pictures = new Dictionary<string, string>();
        Wallpaper.GetMonitorDevicePathCount(out var count);
        for (uint index = 0; index < count; index++)
        {
            PWSTR monitorPath;
            Wallpaper.GetMonitorDevicePathAt(index, &monitorPath);
            string monitor;
            try
            {
                monitor = monitorPath.ToString();
            }
            finally
            {
                Marshal.FreeCoTaskMem((nint)monitorPath.Value);
            }
            try
            {
                PWSTR picture;
                fixed (char* id = monitor)
                {
                    Wallpaper.GetWallpaper(id, &picture);
                }
                try
                {
                    pictures[monitor] = picture.ToString() ?? "";
                }
                finally
                {
                    Marshal.FreeCoTaskMem((nint)picture.Value);
                }
            }
            catch (COMException)
            {
                // A monitor that's known but not attached.
            }
        }
        DESKTOP_WALLPAPER_POSITION position;
        Wallpaper.GetPosition(&position);
        COLORREF color;
        Wallpaper.GetBackgroundColor(&color);
        if (SpotlightIsOn())
        {
            return new DesktopPictures(pictures, (int)position, color.Value, BackgroundKind.Spotlight);
        }
        return ReadSlideshow() is { } slideshow
            ? new DesktopPictures(pictures, (int)position, color.Value, BackgroundKind.Slideshow, slideshow)
            : new DesktopPictures(pictures, (int)position, color.Value);
    }

    /// <summary>The slideshow the desktop runs (Settings › Personalization › Background › Slideshow), or null: its
    /// folders or pictures, shuffle, and how often it changes.</summary>
    private unsafe SlideshowSetting? ReadSlideshow()
    {
        try
        {
            DESKTOP_SLIDESHOW_STATE state;
            Wallpaper.GetStatus(&state);
            if ((state & DESKTOP_SLIDESHOW_STATE.DSS_SLIDESHOW) == 0)
            {
                return null;
            }
            Wallpaper.GetSlideshow(out var items);
            var paths = new List<string>();
            try
            {
                items.GetCount(out var count);
                for (uint index = 0; index < count; index++)
                {
                    items.GetItemAt(index, out var item);
                    PWSTR name;
                    item.GetDisplayName(SIGDN.SIGDN_FILESYSPATH, &name);
                    try
                    {
                        if (name.ToString() is { Length: > 0 } path)
                        {
                            paths.Add(path);
                        }
                    }
                    finally
                    {
                        Marshal.FreeCoTaskMem((nint)name.Value);
                        Marshal.ReleaseComObject(item);
                    }
                }
            }
            finally
            {
                Marshal.ReleaseComObject(items);
            }
            if (paths.Count == 0)
            {
                return null;
            }
            Wallpaper.GetSlideshowOptions(out var options, out var tick);
            return new SlideshowSetting(paths, (int)options, tick);
        }
        catch (Exception error)
        {
            // No slideshow the shell can describe: the pictures are kept as pictures.
            Log.Error("Reading the desktop slideshow", error);
            return null;
        }
    }

    /// <summary>Windows Spotlight is the desktop background. Windows has no API for this: Settings keeps it in
    /// HKCU\…\DesktopSpotlight\Settings (EnabledState 1 while it's on; Windows sets 0 when an app sets a picture).
    /// Only read, never written: turning it on is the person's, in Settings.</summary>
    public static bool SpotlightIsOn()
    {
        try
        {
            using var key = Registry.CurrentUser.OpenSubKey(@"Software\Microsoft\Windows\CurrentVersion\DesktopSpotlight\Settings");
            return key?.GetValue("EnabledState") is int state && state == 1;
        }
        catch (Exception)
        {
            return false;
        }
    }

    /// <summary>Puts the person's own background back (AutoPaper's restore): their slideshow, as it was, when they had
    /// one and its folders are still there; otherwise each monitor's file (empty: no picture, the background colour
    /// shows), with the position and colour they had.</summary>
    public Task RestoreAsync(OwnWallpaper.Kept kept) => worker.Run(() =>
    {
        if (kept is { Kind: BackgroundKind.Slideshow, Slideshow: { } slideshow } && RestoreSlideshow(slideshow, kept.Position, kept.Color))
        {
            return;
        }
        Restore(kept.Pictures, kept.Position, kept.Color);
    });

    /// <summary>The slideshow back: its items (those still there), shuffle and interval. False when none of its
    /// items exists any more or the shell refuses (the pictures go back instead).</summary>
    private unsafe bool RestoreSlideshow(SlideshowSetting slideshow, int position, uint color)
    {
        var lists = new List<nint>();
        try
        {
            foreach (var path in slideshow.Items.Where(path => Directory.Exists(path) || File.Exists(path)))
            {
                ITEMIDLIST* list;
                fixed (char* name = path)
                {
                    if (PInvoke.SHParseDisplayName(name, null, &list, 0, null).Succeeded)
                    {
                        lists.Add((nint)list);
                    }
                }
            }
            if (lists.Count == 0)
            {
                return false;
            }
            var pointers = new ITEMIDLIST*[lists.Count];
            for (var index = 0; index < lists.Count; index++)
            {
                pointers[index] = (ITEMIDLIST*)lists[index];
            }
            IShellItemArray items;
            fixed (ITEMIDLIST** first = pointers)
            {
                PInvoke.SHCreateShellItemArrayFromIDLists((uint)pointers.Length, first, out items).ThrowOnFailure();
            }
            try
            {
                Wallpaper.SetBackgroundColor(new COLORREF(color));
                Wallpaper.SetPosition((DESKTOP_WALLPAPER_POSITION)position);
                Wallpaper.SetSlideshow(items);
                Wallpaper.SetSlideshowOptions((DESKTOP_SLIDESHOW_OPTIONS)slideshow.Options, slideshow.TickMs);
            }
            finally
            {
                Marshal.ReleaseComObject(items);
            }
            return true;
        }
        catch (Exception error)
        {
            // The shell refused the slideshow (or its folder can't be read): the pictures go back instead.
            Log.Error("Restoring the desktop slideshow", error);
            return false;
        }
        finally
        {
            foreach (var list in lists)
            {
                PInvoke.ILFree((ITEMIDLIST*)list);
            }
        }
    }

    private unsafe void Restore(IReadOnlyDictionary<string, string> pictures, int position, uint color)
    {
        Wallpaper.SetBackgroundColor(new COLORREF(color));
        Wallpaper.SetPosition((DESKTOP_WALLPAPER_POSITION)position);
        foreach (var (monitorId, path) in pictures)
        {
            try
            {
                fixed (char* monitor = monitorId)
                fixed (char* image = path)
                {
                    Wallpaper.SetWallpaper(monitor, image);
                }
            }
            catch (COMException)
            {
                // That monitor isn't attached now.
            }
        }
    }

    /// <summary>Sets the lock screen picture. Each new picture needs a new file name (the renders have one per
    /// wallpaper and size).</summary>
    public static async Task<LockScreenOutcome> SetLockScreenAsync(string path)
    {
        if (LockScreenIsManaged())
        {
            return LockScreenOutcome.Managed;
        }
        StorageFile file;
        try
        {
            file = await StorageFile.GetFileFromPathAsync(path);
        }
        catch (Exception)
        {
            return LockScreenOutcome.Failed;
        }
        try
        {
            if (UserProfilePersonalizationSettings.IsSupported()
                && await UserProfilePersonalizationSettings.Current.TrySetLockScreenImageAsync(file))
            {
                return LockScreenOutcome.Set;
            }
        }
        catch (Exception)
        {
            // Fall through to the older API.
        }
        try
        {
            await LockScreen.SetImageFileAsync(file);
            return LockScreenOutcome.Set;
        }
        catch (Exception)
        {
            return LockScreenOutcome.Failed;
        }
    }

    /// <summary>
    /// What the lock screen shows now, as far as Windows tells an app: the picture (LockScreen.GetImageStream, else the
    /// file LockScreen.OriginalImageFile names), or null when Windows doesn't hand it over; and the kind of lock screen
    /// (Windows Spotlight, a slideshow, or a picture), from the settings Windows keeps for its Settings page (there's no
    /// API for the kind; only read, never written).
    /// </summary>
    public static Task<(byte[]? Picture, LockScreenKind Kind)> ReadLockScreenAsync() => Task.Run(() =>
    {
        byte[]? picture = null;
        try
        {
            using var stream = LockScreen.GetImageStream();
            if (stream is { Size: > 0 and < 64 * 1024 * 1024 })
            {
                using var reader = stream.AsStreamForRead();
                using var copy = new MemoryStream();
                reader.CopyTo(copy);
                picture = copy.ToArray();
            }
        }
        catch (Exception error)
        {
            Log.Error("Reading the lock screen picture", error);
        }
        if (picture is null)
        {
            try
            {
                if (LockScreen.OriginalImageFile is { IsFile: true } original && File.Exists(original.LocalPath))
                {
                    picture = File.ReadAllBytes(original.LocalPath);
                }
            }
            catch (Exception error)
            {
                Log.Error("Reading the lock screen's original file", error);
            }
        }
        return (picture is { Length: > 0 } ? picture : null, LockScreenKindNow());
    });

    /// <summary>The raw values the kind is read from, for the support log ("rotating=1 slideshow=0").</summary>
    public static string LockScreenKindValues()
    {
        try
        {
            using var lockScreen = Registry.CurrentUser.OpenSubKey(@"Software\Microsoft\Windows\CurrentVersion\Lock Screen");
            using var content = Registry.CurrentUser.OpenSubKey(@"Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager");
            return $"rotating={content?.GetValue("RotatingLockScreenEnabled") ?? "none"} slideshow={lockScreen?.GetValue("SlideshowEnabled") ?? "none"}";
        }
        catch (Exception error)
        {
            return error.GetType().Name;
        }
    }

    /// <summary>The lock screen's kind: Windows Spotlight (HKCU\…\ContentDeliveryManager RotatingLockScreenEnabled),
    /// a slideshow (HKCU\…\Lock Screen SlideshowEnabled), else a picture.</summary>
    public static LockScreenKind LockScreenKindNow()
    {
        try
        {
            using (var lockScreen = Registry.CurrentUser.OpenSubKey(@"Software\Microsoft\Windows\CurrentVersion\Lock Screen"))
            {
                if (lockScreen?.GetValue("SlideshowEnabled") is int slideshow && slideshow == 1)
                {
                    return LockScreenKind.Slideshow;
                }
            }
            using var content = Registry.CurrentUser.OpenSubKey(@"Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager");
            return content?.GetValue("RotatingLockScreenEnabled") is int rotating && rotating == 1 ? LockScreenKind.Spotlight : LockScreenKind.Picture;
        }
        catch (Exception)
        {
            return LockScreenKind.Picture;
        }
    }

    /// <summary>A Group Policy or MDM setting that fixes the lock screen picture (Personalization CSP / GPO
    /// "Prevent changing lock screen and logon image", "Force a specific default lock screen image").</summary>
    public static bool LockScreenIsManaged()
    {
        try
        {
            using var policy = Registry.LocalMachine.OpenSubKey(@"SOFTWARE\Policies\Microsoft\Windows\Personalization");
            if (policy?.GetValue("NoChangingLockScreen") is int noChanging && noChanging != 0)
            {
                return true;
            }
            if (policy?.GetValue("LockScreenImage") is string forced && forced.Length > 0)
            {
                return true;
            }
            using var mdm = Registry.LocalMachine.OpenSubKey(@"SOFTWARE\Microsoft\PolicyManager\current\device\Personalization");
            return mdm?.GetValue("LockScreenImageUrl") is string url && url.Length > 0;
        }
        catch (Exception)
        {
            return false;
        }
    }

    public void Dispose()
    {
        if (wallpaper is not null)
        {
            var held = wallpaper;
            wallpaper = null;
            _ = worker.Run(() => Marshal.ReleaseComObject(held));
        }
        worker.Dispose();
    }
}
