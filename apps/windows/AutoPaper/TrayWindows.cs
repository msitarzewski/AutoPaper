using System.Runtime.InteropServices;
using System.Text;

namespace AutoPaper;

/// <summary>Finds the hidden window WinUIEx's TrayIcon creates on the UI thread (the other WinUI desktop window).</summary>
internal static partial class TrayWindows
{
    private const string WinUIWindowClass = "WinUIDesktopWin32WindowClass";

    public static IntPtr Find(IntPtr mainWindow)
    {
        var found = IntPtr.Zero;
        EnumThreadWindows(GetCurrentThreadId(), (hwnd, _) =>
        {
            if (hwnd != mainWindow && ClassOf(hwnd) == WinUIWindowClass)
            {
                found = hwnd;
                return false;
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }

    private static string ClassOf(IntPtr hwnd)
    {
        var name = new StringBuilder(256);
        return GetClassName(hwnd, name, name.Capacity) > 0 ? name.ToString() : "";
    }

    private delegate bool EnumThreadWndProc(IntPtr hwnd, IntPtr lParam);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool EnumThreadWindows(uint threadId, EnumThreadWndProc callback, IntPtr lParam);

    [DllImport("kernel32.dll")]
    private static extern uint GetCurrentThreadId();

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetClassName(IntPtr hwnd, StringBuilder className, int maxCount);
}
