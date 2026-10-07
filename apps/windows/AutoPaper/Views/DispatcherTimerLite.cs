using Microsoft.UI.Dispatching;

namespace AutoPaper.Views;

/// <summary>A debounce: runs the action once, <c>delay</c> after the last <see cref="Restart"/>, on the UI thread.</summary>
internal sealed class DispatcherTimerLite
{
    private readonly DispatcherQueueTimer timer;

    public DispatcherTimerLite(DispatcherQueue queue, TimeSpan delay, Func<Task> action)
    {
        timer = queue.CreateTimer();
        timer.Interval = delay;
        timer.IsRepeating = false;
        timer.Tick += async (_, _) => await action();
    }

    public void Restart()
    {
        timer.Stop();
        timer.Start();
    }

    /// <summary>Runs now if a run is pending (leaving the page shouldn't lose a change).</summary>
    public bool Pending => timer.IsRunning;

    public void Stop() => timer.Stop();
}
