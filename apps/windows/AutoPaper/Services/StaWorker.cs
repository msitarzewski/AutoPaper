using System.Collections.Concurrent;

namespace AutoPaper.Services;

/// <summary>
/// One single-threaded-apartment thread for shell COM calls that can take a moment (IDesktopWallpaper hands each
/// image to Explorer), so they never block the UI thread.
/// </summary>
internal sealed class StaWorker : IDisposable
{
    private readonly BlockingCollection<Action> queue = new();
    private readonly Thread thread;

    public StaWorker(string name)
    {
        thread = new Thread(() =>
        {
            foreach (var work in queue.GetConsumingEnumerable())
            {
                work();
            }
        })
        {
            IsBackground = true,
            Name = name,
        };
        thread.SetApartmentState(ApartmentState.STA);
        thread.Start();
    }

    public Task<T> Run<T>(Func<T> work)
    {
        var result = new TaskCompletionSource<T>(TaskCreationOptions.RunContinuationsAsynchronously);
        queue.Add(() =>
        {
            try
            {
                result.SetResult(work());
            }
            catch (Exception error)
            {
                result.SetException(error);
            }
        });
        return result.Task;
    }

    public Task Run(Action work) => Run(() =>
    {
        work();
        return true;
    });

    public void Dispose() => queue.CompleteAdding();
}
