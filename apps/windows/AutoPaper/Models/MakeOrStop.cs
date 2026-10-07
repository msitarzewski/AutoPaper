using AutoPaper.Services;

namespace AutoPaper.Models;

/// <summary>
/// Now's reload/stop, like a browser's (user, 2026-10-06; the macOS app's MakeOrStop): New wallpaper now (Ctrl+R, F5)
/// while nothing is being made, Stop (Esc) while one is. One button that changes, so keyboard focus stays on it. A
/// mood's detail has it too: the same for the mood in use; for any other, "Use this mood and make a new wallpaper",
/// which makes that mood current first. No WinUI types: AutoPaper.Tests links it.
/// </summary>
internal readonly record struct MakeOrStop(bool IsStop, bool IsEnabled, bool OtherMood)
{
    /// <param name="ready">The engine is open.</param>
    /// <param name="working">A wallpaper is being made or put on the desktop.</param>
    /// <param name="otherMood">The button is in the detail of a mood that isn't the one in use.</param>
    public static MakeOrStop For(bool ready, bool working, bool otherMood = false) =>
        working ? new(true, ready, false) : new(false, ready, otherMood);

    /// <summary>The button's words (its label and spoken name).</summary>
    public string Label => Loc.Get(IsStop ? "MakeOrStop_Stop" : OtherMood ? "MakeOrStop_UseAndMake" : "MakeOrStop_Make");

    /// <summary>The tooltip, with the shortcut.</summary>
    public string Tooltip => IsStop ? Loc.Get("MakeOrStop_StopTip") : Loc.Format("MakeOrStop_MakeTip", Label);

    /// <summary>The shortcut as UI Automation's AcceleratorKey says it.</summary>
    public string Keys => IsStop ? Loc.Get("MakeOrStop_StopKeys") : Loc.Get("MakeOrStop_MakeKeys");

    /// <summary>Segoe Fluent Icons: Refresh, or Cancel while working.</summary>
    public string Glyph => IsStop ? "\uE711" : "\uE72C";
}
