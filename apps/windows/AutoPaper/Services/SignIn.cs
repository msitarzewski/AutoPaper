using Windows.ApplicationModel;

namespace AutoPaper.Services;

/// <summary>Open at sign-in: the package's StartupTask (Package.appxmanifest, TaskId "AutoPaperStartup").</summary>
internal static class SignIn
{
    private const string TaskId = "AutoPaperStartup";

    public static async Task<StartupTaskState> StateAsync() => (await StartupTask.GetAsync(TaskId)).State;

    /// <summary>Turns it on or off and returns the resulting state (the person or a policy may have the last word:
    /// DisabledByUser can only be undone in Windows Settings > Apps > Startup).</summary>
    public static async Task<StartupTaskState> SetAsync(bool enabled)
    {
        var task = await StartupTask.GetAsync(TaskId);
        if (enabled)
        {
            return task.State is StartupTaskState.Enabled or StartupTaskState.EnabledByPolicy
                ? task.State
                : await task.RequestEnableAsync();
        }
        if (task.State == StartupTaskState.Enabled)
        {
            task.Disable();
        }
        return task.State;
    }
}
