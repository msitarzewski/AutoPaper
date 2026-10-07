using System.Diagnostics;
using System.Runtime.InteropServices;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.Windows.AppLifecycle;

namespace AutoPaper;

/// <summary>
/// Entry point. AutoPaper runs once per user: a second launch, a notification button pressed while it runs, or
/// a sign-in start all reach the instance that's already running (Windows App SDK single-instancing,
/// https://learn.microsoft.com/windows/apps/windows-app-sdk/applifecycle/applifecycle-single-instance).
/// </summary>
public static class Program
{
    private const string InstanceKey = "AutoPaper";

    [STAThread]
    private static int Main(string[] args)
    {
        WinRT.ComWrappersSupport.InitializeComWrappers();
        if (RedirectedToRunningInstance())
        {
            return 0;
        }
        Application.Start(callback =>
        {
            var context = new DispatcherQueueSynchronizationContext(DispatcherQueue.GetForCurrentThread());
            SynchronizationContext.SetSynchronizationContext(context);
            _ = new App();
        });
        return 0;
    }

    private static bool RedirectedToRunningInstance()
    {
        var activation = AppInstance.GetCurrent().GetActivatedEventArgs();
        var main = AppInstance.FindOrRegisterForKey(InstanceKey);
        if (main.IsCurrent)
        {
            main.Activated += (_, redirected) => App.Current?.OnRedirectedActivation(redirected);
            return false;
        }

        // Let the running instance bring its window to the front (this process has the foreground right).
        _ = AllowSetForegroundWindow(main.ProcessId);
        using var done = new ManualResetEvent(false);
        Task.Run(() =>
        {
            try
            {
                main.RedirectActivationToAsync(activation).AsTask().Wait();
            }
            catch (Exception error)
            {
                Debug.WriteLine($"Redirecting the activation failed: {error}");
            }
            done.Set();
        });
        // Wait while still pumping COM messages, as the single-instancing documentation does.
        _ = CoWaitForMultipleObjects(0, uint.MaxValue, 1, [done.SafeWaitHandle.DangerousGetHandle()], out _);
        return true;
    }

    [DllImport("ole32.dll")]
    private static extern int CoWaitForMultipleObjects(uint dwFlags, uint dwMilliseconds, uint nHandles, IntPtr[] pHandles, out uint dwIndex);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool AllowSetForegroundWindow(uint dwProcessId);
}
