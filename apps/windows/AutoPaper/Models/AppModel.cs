using System.Globalization;
using AutoPaper.Core;
using AutoPaper.Services;
using CommunityToolkit.Mvvm.ComponentModel;
using Microsoft.UI.Dispatching;
using Microsoft.Windows.System.Power;
using Windows.ApplicationModel;
using Windows.Storage;

namespace AutoPaper.Models;

/// <summary>
/// The app's state and its conversation with the engine. Lives on the UI thread; every engine call runs on the
/// thread pool (the engine's sync methods block, and its async ones poll on the calling thread), and every
/// result comes back here before it touches a property.
/// Engine lifecycle per docs/app-spec.md: open off the UI thread, display hint, a DispatcherQueueTimer on
/// next_start() (the due time less how long a wallpaper takes here, so it's ready on time) plus resume and unlock,
/// render per display → set → lock screen → mark_shown. Progress comes as stages (ProgressObserver, per call) and in
/// detail (ProgressDetailObserver, engine-wide: a fraction and seconds left while painting).
/// </summary>
internal sealed partial class AppModel : ObservableObject
{
    /// <summary>The timer looks again at least this often (the clock can change, and sleep stops timers).</summary>
    private static readonly TimeSpan LongestWait = TimeSpan.FromMinutes(15);

    /// <summary>A due time that arrives while a wallpaper is being made or shown is looked at again this soon.</summary>
    private static readonly TimeSpan BusyRetry = TimeSpan.FromSeconds(30);

    private readonly DispatcherQueue ui;
    private readonly Desktop desktop = new();
    private readonly OwnWallpaper ownWallpaper = new(ApplicationData.Current.LocalFolder.Path);
    private readonly OwnLockScreen ownLockScreen = new(ApplicationData.Current.LocalFolder.Path);
    private readonly List<Func<Task>> afterOpen = [];
    private readonly Observer observer;
    private readonly DetailObserver detailObserver;
    private Engine? engine;
    private DispatcherQueueTimer? timer;
    private bool checking;
    /// <summary>Makes and shows the person started that are still running (the stage stays up until all are done).</summary>
    private int making;
    /// <summary>The person pressed Cancel during this make: whatever ends it is reported as cancelled.</summary>
    private bool cancelRequested;
    /// <summary>rate() asked for a disliked wallpaper's replacement while another one was being made or shown: its
    /// id, so the replacement starts once that's done (if the disliked one is still showing).</summary>
    private string? replaceWhenFree;
    /// <summary>The wallpaper is being put on the desktop: a Cancel now can't stop it (it's said as it is).</summary>
    private bool settingDesktop;
    /// <summary>What's under way can be stopped: the engine is making one (New wallpaper now, an echo, the schedule),
    /// not just putting one on the desktop (Show on desktop, a display change).</summary>
    private bool stoppable;
    /// <summary>Displays changed while something was being made or shown: render the one showing again after.</summary>
    private bool displaysChangedWhileBusy;

    public AppModel(DispatcherQueue ui)
    {
        this.ui = ui;
        observer = new Observer(this);
        detailObserver = new DetailObserver(this);
    }

    public CredentialStore Secrets { get; } = new();

    public Engine Engine => engine ?? throw new InvalidOperationException("The engine isn't open yet.");

    /// <summary>Raised with a sentence for screen readers (the window raises it as a UI Automation notification).</summary>
    public event EventHandler<Announcement>? Announced;

    /// <summary>History changed: a new wallpaper, a rating, a deletion, cleared history.</summary>
    public event EventHandler? HistoryChanged;

    /// <summary>Moods changed: one added, renamed, deleted, moved, switched to, or its keywords or Surprise edited.</summary>
    public event EventHandler? MoodsChanged;

    /// <summary>A key was saved or removed in Credential Manager (its account): views that list models or test a
    /// provider refresh themselves (docs/app-spec.md 6a: a saved key reloads the model list).</summary>
    public event EventHandler<string>? KeyChanged;

    /// <summary>The main window is open and active (decides whether a new wallpaper also gets a notification).</summary>
    public Func<bool> WindowIsActive { get; set; } = () => false;

    // ── State ───────────────────────────────────────────────────────────────────────────────────

    public bool IsReady { get; private set => SetProperty(ref field, value); }

    /// <summary>The engine couldn't open (the data folder or database): nothing else works.</summary>
    public string? StartupProblem { get; private set => SetProperty(ref field, value); }

    public Settings Settings { get; private set => SetProperty(ref field, value); } = null!;

    public Generation? Current
    {
        get;
        private set
        {
            if (SetProperty(ref field, value))
            {
                OnPropertyChanged(nameof(HasCurrent));
                OnPropertyChanged(nameof(IsLiked));
                OnPropertyChanged(nameof(IsDisliked));
            }
        }
    }

    public bool HasCurrent => Current is not null;
    public bool IsLiked => Current?.Rating == Rating.Liked;
    public bool IsDisliked => Current?.Rating == Rating.Disliked;

    /// <summary>The wallpaper in words: describe(id) and the rating (the Now view's image name).</summary>
    public string CurrentDescription { get; private set => SetProperty(ref field, value); } = "";

    public bool IsGenerating { get; private set => SetProperty(ref field, value); }

    /// <summary>The stage in words while making or applying one ("Painting…").</summary>
    public string StageText { get; private set => SetProperty(ref field, value); } = "";

    /// <summary>"Next new wallpaper at 9:00" / "New wallpapers are paused" / "New wallpapers only when you ask".</summary>
    public string NextText { get; private set => SetProperty(ref field, value); } = "";

    /// <summary>"$1.20 of $5.00 this month (estimated)".</summary>
    public string BudgetText { get; private set => SetProperty(ref field, value); } = "";

    /// <summary>The current month's budget gate, refreshed at launch, after settings changes and every run. The
    /// window keeps its notice visible across pages while the estimated next run would exceed the limit.</summary>
    public BudgetStatus? BudgetStatus
    {
        get;
        private set
        {
            if (SetProperty(ref field, value))
            {
                OnPropertyChanged(nameof(BudgetBlocked));
                OnPropertyChanged(nameof(BudgetNotice));
            }
        }
    }

    public bool BudgetBlocked => BudgetStatus?.Blocked == true;
    public string BudgetNotice => BudgetBlocked ? BudgetStatus!.Message : "";

    /// <summary>The last thing that went wrong (cleared by the next success): a sentence and, when the person can fix
    /// it, a link to where (docs/app-spec.md 6a).</summary>
    public Problem? Problem
    {
        get;
        private set
        {
            if (SetProperty(ref field, value))
            {
                // The words first, then HasProblem: the Now view opens its InfoBar on HasProblem, and the bar says its
                // message as it opens.
                OnPropertyChanged(nameof(ProblemText));
                OnPropertyChanged(nameof(ProblemLinkText));
                OnPropertyChanged(nameof(HasProblemLink));
                OnPropertyChanged(nameof(HasProblem));
            }
        }
    }

    public bool HasProblem => Problem is not null;

    /// <summary>The problem's sentence; empty when the link says it all ("Add your OpenAI key").</summary>
    public string ProblemText => Problem?.Sentence ?? "";

    /// <summary>The problem's link ("Check ComfyUI settings"), or empty.</summary>
    public string ProblemLinkText => Problem?.Link ?? "";

    public bool HasProblemLink => Problem?.Link is not null;

    /// <summary>Why the wallpaper showing is a past one, or that the lock screen couldn't be set.</summary>
    public string? NoticeText
    {
        get;
        private set
        {
            if (SetProperty(ref field, value))
            {
                OnPropertyChanged(nameof(HasNotice));
            }
        }
    }

    public bool HasNotice => !string.IsNullOrEmpty(NoticeText);

    public bool KeywordsNarrow { get; private set => SetProperty(ref field, value); }

    /// <summary>How far the painting is (0–1), or null when nothing backs a number (the ring is indeterminate).</summary>
    public double? ProgressFraction
    {
        get;
        private set
        {
            if (SetProperty(ref field, value))
            {
                OnPropertyChanged(nameof(ProgressKnown));
                OnPropertyChanged(nameof(ProgressPercent));
            }
        }
    }

    public bool ProgressKnown => ProgressFraction is not null;

    /// <summary>The ring's value (0–100).</summary>
    public double ProgressPercent => (ProgressFraction ?? 0) * 100;

    /// <summary>Seconds until the picture arrives (the engine's estimate), or null.</summary>
    public uint? SecondsLeft { get; private set; }

    /// <summary>"About 6 minutes left" / "Almost done", or empty.</summary>
    public string TimeLeftText { get; private set => SetProperty(ref field, value); } = "";

    /// <summary>Every mood, in the person's order (the notification-area menu's Mood submenu, History's filter).</summary>
    public IReadOnlyList<Mood> Moods { get; private set; } = [];

    /// <summary>The mood in use.</summary>
    public Mood? ActiveMood => Moods.FirstOrDefault(mood => mood.Active);

    /// <summary>The person's own wallpaper is on the desktop (put back by Restore my wallpaper or on quit), not
    /// AutoPaper's: a new wallpaper or Show on desktop puts AutoPaper's back, and so does the next launch after a quit.
    /// Pausing keeps AutoPaper's wallpaper showing (user, 2026-10-06).</summary>
    public bool DesktopShowsOwn
    {
        get => Preferences.DesktopShowsOwn;
        private set
        {
            if (Preferences.DesktopShowsOwn != value)
            {
                Preferences.DesktopShowsOwn = value;
                OnPropertyChanged();
                OnPropertyChanged(nameof(CanRestoreWallpaper));
            }
        }
    }

    /// <summary>Restore my wallpaper can do something: AutoPaper kept the person's own desktop and its wallpaper is
    /// showing, or AutoPaper changed the lock screen since it kept the person's.</summary>
    public bool CanRestoreWallpaper => (!DesktopShowsOwn && ownWallpaper.HasCopy) || ownLockScreen.Read() is { Set: true };

    /// <summary>What putting the person's own lock screen back couldn't do (Windows Spotlight or a slideshow, which only
    /// the person can turn on again; a picture Windows didn't let AutoPaper read), said in Settings › General with the
    /// link to Windows' lock screen settings until AutoPaper sets the lock screen again.</summary>
    public LockScreenNote LockScreenNote
    {
        get => Preferences.LockScreenNote;
        private set
        {
            if (Preferences.LockScreenNote != value)
            {
                Preferences.LockScreenNote = value;
                OnPropertyChanged();
            }
        }
    }

    /// <summary>Model names by id from the providers' own lists (list_models' display names), read this session:
    /// the provenance line ("Painted by Nano Banana 2") prefers them.</summary>
    public IReadOnlyDictionary<string, string> ModelNames => modelNames;

    private readonly Dictionary<string, string> modelNames = new(StringComparer.Ordinal);

    /// <summary>A provider's list of models was read (Settings › Providers): their names are used from now on.</summary>
    public void RememberModelNames(IEnumerable<ModelInfo> models)
    {
        var changed = false;
        foreach (var model in models)
        {
            if (!string.IsNullOrWhiteSpace(model.DisplayName) && model.DisplayName != model.Id
                && modelNames.GetValueOrDefault(model.Id) != model.DisplayName)
            {
                modelNames[model.Id] = model.DisplayName;
                changed = true;
            }
        }
        if (changed)
        {
            OnPropertyChanged(nameof(ModelNames));
        }
    }

    /// <summary>The person's own background was Windows Spotlight and is back as its last picture, with Spotlight
    /// still off (Windows switches it off when an app sets a picture, and lets only the person switch it on): Settings
    /// › General says so, with the link to turn it back on.</summary>
    public bool SpotlightLeftOff => DesktopShowsOwn && ownWallpaper.Read() is { Kind: BackgroundKind.Spotlight } && !Desktop.SpotlightIsOn();

    /// <summary>History was cleared while a wallpaper was on the desktop: it keeps its picture until the next one,
    /// so the Now view says that rather than "No wallpaper yet".</summary>
    public bool DesktopKeepsPicture
    {
        get => Preferences.DesktopKeepsPicture;
        private set
        {
            if (Preferences.DesktopKeepsPicture != value)
            {
                Preferences.DesktopKeepsPicture = value;
                OnPropertyChanged();
            }
        }
    }

    /// <summary>The first-run page is up: nothing is made on the schedule until it's done or skipped.</summary>
    public bool ScheduleHeld { get; set; }

    public MemoryStatus? Memory { get; private set => SetProperty(ref field, value); }

    // ── Opening ─────────────────────────────────────────────────────────────────────────────────

    /// <summary>Opens the engine (off the UI thread: it loads the 133 MB embedding model), then starts the schedule.</summary>
    public async Task OpenAsync()
    {
        var dataDir = ApplicationData.Current.LocalFolder.Path;
        var modelDir = Path.Combine(Package.Current.InstalledLocation.Path, "Models", "bge-small-en-v1.5");
        var locale = CultureInfo.CurrentUICulture.Name is { Length: > 0 } name ? name : "en-US";
        var os = Environment.OSVersion.Version;
        var version = Package.Current.Id.Version;
        var client = $"Windows/{os.Major}.{os.Minor}.{os.Build} AutoPaper/{version.Major}.{version.Minor}.{version.Build}";
        try
        {
            engine = await Task.Run(() => Engine.Open(new EngineConfig(dataDir, modelDir, locale, client), Secrets));
            engine.SetProgressDetailObserver(detailObserver);
            Settings = await Call(e => e.Settings());
            Memory = await Call(e => e.MemoryStatus());
            Moods = await Call(e => e.Moods());
        }
        catch (Exception error)
        {
            // The details (a path, a Windows message, the core's English) go to the log, not the window.
            Log.Error("Opening the engine", error);
            StartupProblem = error is AutoPaperException.Storage ? Loc.Get("Error_OpenStorage") : Loc.Get("Error_Open");
            return;
        }
        if (Memory is { Reduced: true, Problem: { } why })
        {
            Log.Error("Memory is reduced", new InvalidOperationException(why));
        }
        await UpdateDisplayHintAsync();
        await LoadCurrentAsync();
        IsReady = true;
        await RefreshStatusAsync();
        StartSchedule();
        foreach (var work in afterOpen.ToArray())
        {
            await work();
        }
        afterOpen.Clear();
        // AutoPaper put the person's own wallpaper back when it quit: running again, its own shows again (paused or not:
        // pausing keeps the current wallpaper). One they chose with Restore my wallpaper stays until the next new one.
        if (DesktopShowsOwn && Preferences.UncoveredByQuit)
        {
            await ShowAutoPapersAgainAsync();
        }
        await CheckScheduleAsync();
    }

    /// <summary>Runs <paramref name="work"/> now, or once the engine is open (a notification button can start the app).</summary>
    public async Task WhenReadyAsync(Func<Task> work)
    {
        if (IsReady)
        {
            await work();
        }
        else if (StartupProblem is null)
        {
            afterOpen.Add(work);
        }
    }

    /// <summary>An engine call on the thread pool.</summary>
    public Task<T> Call<T>(Func<Engine, T> work)
    {
        var opened = Engine;
        return Task.Run(() => work(opened));
    }

    public Task Call(Action<Engine> work)
    {
        var opened = Engine;
        return Task.Run(() => work(opened));
    }

    /// <summary>An async engine call, polled from the thread pool (its first poll does database work).</summary>
    public Task<T> CallAsync<T>(Func<Engine, Task<T>> work)
    {
        var opened = Engine;
        return Task.Run(() => work(opened));
    }

    public Task CallAsync(Func<Engine, Task> work)
    {
        var opened = Engine;
        return Task.Run(() => work(opened));
    }

    // ── Schedule ────────────────────────────────────────────────────────────────────────────────

    private void StartSchedule()
    {
        timer = ui.CreateTimer();
        timer.IsRepeating = false;
        timer.Tick += async (_, _) => await CheckScheduleAsync();
        // Wake from sleep: the timer didn't run while the PC slept.
        PowerManager.SystemSuspendStatusChanged += (_, _) =>
        {
            if (PowerManager.SystemSuspendStatus is SystemSuspendStatus.AutoResume or SystemSuspendStatus.ManualResume)
            {
                ui.TryEnqueue(async () => await CheckScheduleAsync());
            }
        };
        _ = ArmAsync();
    }

    /// <summary>Points the timer at next_start() (or at most 15 minutes away, to look again): the due time less how
    /// long a wallpaper takes on this PC, so a scheduled one is ready when it's due (run_if_due counts it due from
    /// then; the status line still says next_due).</summary>
    private async Task ArmAsync()
    {
        if (timer is null)
        {
            return;
        }
        long? due;
        try
        {
            due = await Call(e => e.NextStart());
        }
        catch (Exception)
        {
            due = null;
        }
        timer.Stop();
        if (due is null)
        {
            return; // Paused or only when asked: settings changes re-arm.
        }
        var wait = DateTimeOffset.FromUnixTimeSeconds(due.Value) - DateTimeOffset.UtcNow;
        timer.Interval = wait < TimeSpan.FromSeconds(1) ? TimeSpan.FromSeconds(1) : wait > LongestWait ? LongestWait : wait;
        timer.Start();
    }

    private void RetrySoon()
    {
        if (timer is null)
        {
            return;
        }
        timer.Stop();
        timer.Interval = BusyRetry;
        timer.Start();
    }

    /// <summary>The due time, wake, resume or unlock: run_if_due, and show what it returns.</summary>
    public async Task CheckScheduleAsync()
    {
        if (!IsReady || ScheduleHeld)
        {
            return;
        }
        if (checking || IsGenerating)
        {
            // Something else is being made or shown (History's Show on desktop or a display change re-render don't
            // re-arm the timer when they finish): look again shortly, so the schedule never stops.
            RetrySoon();
            return;
        }
        checking = true;
        stoppable = true;
        try
        {
            var shown = await Task.Run(() => Engine.RunIfDue(observer));
            if (shown is not null)
            {
                // A new one can still be kept off the desktop by Cancel; a saved one brought back is just shown.
                await PutOnDesktopAsync(shown.Generation, markShown: true, setLockScreen: true, cancellable: shown.Revisit is null);
                Problem = null;
                if (shown.Revisit is { } reason)
                {
                    await ShowRevisitAsync(reason);
                }
                else
                {
                    NoticeText = null;
                    NewWallpaperMade(shown.Generation, scheduled: true);
                }
            }
        }
        catch (MadeButNotShownException)
        {
            CancelledAfterMaking();
        }
        catch (AutoPaperException.Cancelled)
        {
            Announce(Loc.Get("Status_Cancelled"));
        }
        catch (Exception) when (cancelRequested)
        {
            Announce(Loc.Get("Status_Cancelled"));
        }
        catch (Exception error)
        {
            ShowProblem(error, scheduled: true);
            if (error is AutoPaperException.BudgetReached)
            {
                await NotifyBudgetOnceAsync();
            }
            else if (error is AutoPaperException.MissingKey or AutoPaperException.InvalidKey && !Preferences.KeyProblemNotified)
            {
                Preferences.KeyProblemNotified = true;
                Notifications.KeyProblem(Text.Problem(error, Settings, scheduled: true));
            }
        }
        finally
        {
            checking = false;
            stoppable = false;
            if (making == 0)
            {
                cancelRequested = false;
            }
            FinishStage();
            await RefreshStatusAsync();
            await ArmAsync();
            await AfterBusyAsync();
        }
    }

    private async Task NotifyBudgetOnceAsync()
    {
        var budget = await Call(e => e.BudgetStatus());
        BudgetStatus = budget;
        if (budget.Blocked && Preferences.BudgetNotifiedMonth != budget.Month)
        {
            Preferences.BudgetNotifiedMonth = budget.Month;
            Notifications.BudgetSpent(budget.Message);
        }
    }

    // ── Making and showing ──────────────────────────────────────────────────────────────────────

    /// <summary>New Wallpaper Now (and a disliked one's replacement): generate, then put it on every display.</summary>
    public Task<string?> NewWallpaperAsync(Trigger trigger = Trigger.Manual) =>
        MakeAsync(() => Engine.GenerateOrRevisit(trigger, observer));

    /// <summary>A mood's New wallpaper now (its detail's reload, Ctrl+R there; the macOS app's newWallpaper(from:)):
    /// a mood that isn't the one in use is made current first, then a new wallpaper is made from it. If the switch
    /// fails, that's said and nothing is made.</summary>
    public async Task<string?> NewWallpaperFromMoodAsync(string moodId)
    {
        if (!IsReady || IsGenerating)
        {
            return null;
        }
        if (ActiveMood?.Id != moodId)
        {
            try
            {
                await UseMoodAsync(moodId);
            }
            catch (Exception error)
            {
                ShowProblem(error, scheduled: false);
                return Problem?.Spoken;
            }
        }
        return await NewWallpaperAsync();
    }

    /// <summary>History's Make an Echo.</summary>
    public Task<string?> MakeEchoAsync(string id) => MakeAsync(() => Engine.MakeEchoOrRevisit(id, observer));

    /// <summary>Makes one and shows it. Returns the sentence said about how it ended (the new wallpaper, cancelled, or
    /// the problem), for a page to show too; null when it didn't start (another one is under way).</summary>
    private async Task<string?> MakeAsync(Func<Task<Shown>> make)
    {
        if (!IsReady || IsGenerating)
        {
            return null;
        }
        string? outcome = null;
        making++;
        cancelRequested = false;
        stoppable = true;
        IsGenerating = true;
        ShowProgress(null, null);
        StageText = Text.Stage(ProgressStage.CheckingServices);
        Problem = null;
        NoticeText = null;
        Announce(StageText);
        try
        {
            var shown = await Task.Run(make);
            await PutOnDesktopAsync(shown.Generation, markShown: true, setLockScreen: true, cancellable: shown.Revisit is null);
            outcome = shown.Revisit is { } reason
                ? await ShowRevisitAsync(reason)
                : NewWallpaperMade(shown.Generation, scheduled: false);
        }
        catch (MadeButNotShownException)
        {
            outcome = CancelledAfterMaking();
        }
        catch (AutoPaperException.Cancelled)
        {
            outcome = Loc.Get("Status_Cancelled");
            Announce(outcome);
        }
        catch (Exception) when (cancelRequested)
        {
            // A request that failed in the same moment as the cancel: the person asked to stop, so that's what
            // they hear.
            outcome = Loc.Get("Status_Cancelled");
            Announce(outcome);
        }
        catch (Exception error)
        {
            ShowProblem(error, scheduled: false);
            outcome = Problem?.Spoken;
            if (error is AutoPaperException.BudgetReached)
            {
                await NotifyBudgetOnceAsync();
            }
        }
        finally
        {
            making--;
            cancelRequested = false;
            stoppable = false;
            FinishStage();
            await RefreshStatusAsync();
            await ArmAsync();
            await AfterBusyAsync();
        }
        return outcome;
    }

    /// <summary>History's Show on Desktop: a past wallpaper brought back (doesn't restart the schedule).</summary>
    public async Task ShowOnDesktopAsync(Generation generation)
    {
        if (!IsReady || IsGenerating)
        {
            return;
        }
        making++;
        IsGenerating = true;
        try
        {
            await PutOnDesktopAsync(generation, markShown: true, setLockScreen: true);
            Problem = null;
            NoticeText = null;
            Announce(Loc.Format("Status_ShowingNow", generation.Concept.Title));
        }
        catch (Exception error)
        {
            ShowProblem(error, scheduled: false);
        }
        finally
        {
            making--;
            FinishStage();
            await AfterBusyAsync();
        }
    }

    /// <summary>Cancel: the engine's call returns Cancelled promptly, even mid-request. Once the wallpaper is being
    /// put on the desktop it can't be taken back: Cancel then says so instead of "Cancelling…" (the new wallpaper is
    /// announced a moment later).</summary>
    public void Cancel()
    {
        if (!IsGenerating || engine is null || cancelRequested)
        {
            return;
        }
        if (settingDesktop || !stoppable)
        {
            // Being put on the desktop (a new one, or one brought back from History): nothing left to stop.
            Announce(Loc.Get("Status_TooLateToCancel"));
            return;
        }
        engine.Cancel();
        cancelRequested = true;
        ShowProgress(null, null);
        StageText = Loc.Get("Stage_Cancelling");
        Announce(StageText);
    }

    /// <summary>Renders for each display, sets each one's wallpaper, then the lock screen, then tells the engine.</summary>
    /// <param name="cancellable">A wallpaper just made or chosen by the schedule: Cancel, pressed after the engine
    /// finished but before the desktop changed, leaves the desktop as it was (<see cref="MadeButNotShownException"/>).</param>
    private async Task PutOnDesktopAsync(Generation generation, bool markShown, bool setLockScreen, bool cancellable = false)
    {
        IsGenerating = true;
        StageText = Text.Stage(ProgressStage.Rendering);
        var displays = await desktop.DisplaysAsync();
        IReadOnlyList<DisplayInfo> targets = displays.Count > 0 ? displays : [new DisplayInfo("", 3840, 2160, true)];
        var id = generation.Id;
        var renders = await Call(e => targets
            .Select(display => (Display: display, Path: e.RenderForDisplay(id, new DisplayTarget(display.Id, display.Width, display.Height))))
            .ToList());
        if (cancellable && cancelRequested)
        {
            throw new MadeButNotShownException();
        }
        string? lockNote = null;
        settingDesktop = true;
        try
        {
            await KeepOwnWallpaperAsync();
            await desktop.SetWallpapersAsync(renders
                .Select(render => (displays.Count > 0 ? render.Display.Id : (string?)null, render.Path))
                .ToList());
            if (setLockScreen && Settings.SetLockScreen)
            {
                var primary = renders.FirstOrDefault(render => render.Display.IsPrimary);
                lockNote = await SetLockScreenAsync(primary.Path ?? renders[0].Path);
            }
            DesktopShowsOwn = false;
            Preferences.UncoveredByQuit = false;
            if (markShown)
            {
                await Call(e => e.MarkShown(id));
                DesktopKeepsPicture = false;
            }
        }
        finally
        {
            settingDesktop = false;
        }
        await LoadCurrentAsync();
        if (lockNote is not null)
        {
            NoticeText = lockNote;
        }
    }

    /// <summary>The lock screen: the person's own is kept first (when it's theirs that's showing), then AutoPaper's is
    /// set and what Windows reports for it remembered. Returns what to say when it couldn't be set, else null.</summary>
    private async Task<string?> SetLockScreenAsync(string path)
    {
        try
        {
            var before = await Desktop.ReadLockScreenAsync();
            if (await Task.Run(() => ownLockScreen.Keep(before.Picture, before.Kind)))
            {
                Log.Note($"Kept the person's lock screen: {before.Kind} ({Desktop.LockScreenKindValues()}), {before.Picture?.Length ?? 0} bytes");
                OnPropertyChanged(nameof(CanRestoreWallpaper));
            }
        }
        catch (Exception error)
        {
            Log.Error("Keeping the person's own lock screen", error);
        }
        var outcome = await Desktop.SetLockScreenAsync(path);
        if (outcome == LockScreenOutcome.Set)
        {
            try
            {
                var after = await Desktop.ReadLockScreenAsync();
                await Task.Run(() => ownLockScreen.SetByAutoPaper(after.Picture));
            }
            catch (Exception error)
            {
                Log.Error("Reading the lock screen AutoPaper set", error);
            }
            LockScreenNote = LockScreenNote.None;
            OnPropertyChanged(nameof(CanRestoreWallpaper));
        }
        return outcome switch
        {
            LockScreenOutcome.Managed => Loc.Get("LockScreen_Managed"),
            LockScreenOutcome.Failed => Loc.Get("LockScreen_Failed"),
            _ => null,
        };
    }

    /// <summary>Cancel came after the engine had finished: the desktop stays as it was, and the wallpaper (already
    /// made, and paid for) waits in History.</summary>
    private string CancelledAfterMaking()
    {
        HistoryChanged?.Invoke(this, EventArgs.Empty);
        var said = Loc.Get("Status_CancelledKept");
        Announce(said);
        return said;
    }

    /// <summary>A saved wallpaper returned by the core, with its reason; never announced as a new image.</summary>
    private async Task<string> ShowRevisitAsync(RevisitReason reason)
    {
        // Reopen Now's InfoBar even when a consecutive run has the same reason.
        NoticeText = null;
        var message = Text.Revisit(reason, Settings);
        NoticeText = message;
        Announce(message, AnnouncementKind.NowNotice);
        HistoryChanged?.Invoke(this, EventArgs.Empty);
        if (reason == RevisitReason.OverBudget)
        {
            await NotifyBudgetOnceAsync();
        }
        return message;
    }

    /// <summary>History, the announcement and (when the window isn't in front) a notification. Returns what was said.</summary>
    private string NewWallpaperMade(Generation made, bool scheduled)
    {
        Preferences.KeyProblemNotified = false;
        HistoryChanged?.Invoke(this, EventArgs.Empty);
        var said = Loc.Format("Status_NewWallpaper", made.Concept.Title);
        Announce(said);
        if (Preferences.NotifyNewWallpapers && (scheduled || !WindowIsActive()))
        {
            Notifications.NewWallpaper(made);
        }
        return said;
    }

    private void ShowProblem(Exception error, bool scheduled)
    {
        if (error is not AutoPaperException)
        {
            Log.Error(scheduled ? "Scheduled wallpaper" : "Making or showing a wallpaper", error);
        }
        var problem = Text.Problem(error, Settings, scheduled);
        // Closed first, so the Now page's InfoBar opens again (and says so) even when the sentence is the same.
        Problem = null;
        Problem = problem;
        // The InfoBar announces its sentence when it opens; a problem that's only a link ("Add your OpenAI key") has
        // no sentence for it to say, so the window says the link's words.
        Announce(problem.Spoken, problem.Sentence.Length > 0 ? AnnouncementKind.NowProblem : AnnouncementKind.Error);
    }

    /// <summary>Clears the stage once nothing is being made or shown any more (a scheduled run and one the person
    /// asked for can overlap: the engine runs them one after the other).</summary>
    private void FinishStage()
    {
        if (making > 0 || checking)
        {
            return;
        }
        IsGenerating = false;
        StageText = "";
        ShowProgress(null, null);
    }

    /// <summary>The ring and the time left.</summary>
    private void ShowProgress(float? fraction, uint? secondsLeft)
    {
        ProgressFraction = fraction is { } known ? Math.Clamp(known, 0, 1) : null;
        if (SecondsLeft != secondsLeft)
        {
            SecondsLeft = secondsLeft;
            OnPropertyChanged(nameof(SecondsLeft));
        }
        TimeLeftText = Text.TimeLeft(secondsLeft);
    }

    /// <summary>A stage's numbers (ProgressDetailObserver): while painting, how far it is and about how long is left;
    /// every stage reports with none, which resets the ring.</summary>
    private void OnDetail(ProgressDetail detail)
    {
        if (cancelRequested || detail.Stage == ProgressStage.Done || (!IsGenerating && making == 0 && !checking))
        {
            return;
        }
        ShowProgress(detail.Fraction, detail.SecondsLeft);
    }

    private void OnStage(ProgressStage stage)
    {
        if (stage == ProgressStage.Done || cancelRequested)
        {
            return;
        }
        // A scheduled run shows its stages too (the window and the tray menu). Each stage is said once.
        IsGenerating = true;
        var text = Text.Stage(stage);
        if (text != StageText)
        {
            StageText = text;
            Announce(text);
        }
    }

    // ── Ratings ─────────────────────────────────────────────────────────────────────────────────

    /// <summary>Rates a wallpaper. With <paramref name="toggle"/>, choosing its current rating again clears it
    /// (the window's and the tray's Like/Dislike); a notification button just sets it. A dislike of the one
    /// showing, with "Replace wallpapers I dislike" on, makes a new one (rate() says when).</summary>
    public async Task RateAsync(string id, Rating requested, bool toggle = true)
    {
        if (!IsReady)
        {
            return;
        }
        try
        {
            var before = await Call(e => e.Generation(id));
            var rating = toggle && before.Rating == requested ? Rating.Unrated : requested;
            var replace = await Call(e => e.Rate(id, rating));
            await LoadCurrentAsync();
            HistoryChanged?.Invoke(this, EventArgs.Empty);
            Announce(rating switch
            {
                Rating.Liked => Loc.Get("Status_Liked"),
                Rating.Disliked => Loc.Get("Status_Disliked"),
                _ => Loc.Get("Status_RatingCleared"),
            });
            if (replace)
            {
                if (IsGenerating)
                {
                    // Another wallpaper is being made or shown: replace this one when that's done, unless it has
                    // replaced it already.
                    replaceWhenFree = id;
                }
                else
                {
                    await NewWallpaperAsync(Trigger.DislikeReplace);
                }
            }
        }
        catch (Exception error)
        {
            ShowProblem(error, scheduled: false);
        }
    }

    /// <summary>Something that was being made or shown has finished: a display change that came meanwhile renders
    /// the one showing again, and a disliked one's replacement that waited is made.</summary>
    private async Task AfterBusyAsync()
    {
        if (displaysChangedWhileBusy && !IsGenerating && !checking)
        {
            displaysChangedWhileBusy = false;
            await DisplaysChangedAsync();
        }
        await ReplaceWhenFreeAsync();
    }

    /// <summary>A disliked wallpaper's replacement that waited for a make or show to finish: made now, when nothing
    /// else is running and the disliked wallpaper is still the one showing (a new one may have replaced it, or the
    /// person may have changed the rating).</summary>
    private async Task ReplaceWhenFreeAsync()
    {
        if (replaceWhenFree is not { } id || IsGenerating || checking)
        {
            return;
        }
        replaceWhenFree = null;
        if (Current is { } showing && showing.Id == id && showing.Rating == Rating.Disliked && Settings.ReplaceDisliked)
        {
            await NewWallpaperAsync(Trigger.DislikeReplace);
        }
    }

    // ── Settings and status ─────────────────────────────────────────────────────────────────────

    /// <summary>Saves a change to the engine's settings (it validates and clamps), then re-reads them. Throws
    /// the engine's error for the page to show. The change is applied to the engine's settings as they are at that
    /// moment, in the same call: Surprise belongs to the active mood, so a copy read before a mood switch (here, in
    /// the notification-area menu) would give the new mood the old one's Surprise.
    /// <paramref name="change"/> therefore runs on the thread pool: it must only use values the caller read before
    /// (a control's property read there throws RPC_E_WRONG_THREAD).</summary>
    public async Task UpdateSettingsAsync(Func<Settings, Settings> change)
    {
        var (wasWriter, wasPainter) = (Settings.TextProvider, Settings.ImageProvider);
        try
        {
            Settings = await Call(e =>
            {
                e.UpdateSettings(change(e.Settings()));
                return e.Settings();
            });
        }
        catch (Exception)
        {
            Settings = await Call(e => e.Settings());
            throw;
        }
        await RefreshStatusAsync();
        await ArmAsync();
        if ((Settings.TextProvider != wasWriter || Settings.ImageProvider != wasPainter) && Problem is { Fix: Fix.Keys or Fix.Providers })
        {
            // A problem with the providers that were in use: the person changed them, and the next wallpaper says
            // whether anything is still wrong.
            Problem = null;
        }
        // Pausing only stops new wallpapers: the one showing stays (user, 2026-10-06). Only Quit and Restore my
        // wallpaper put the person's own back.
    }

    // ── The person's own wallpaper ──────────────────────────────────────────────────────────────

    /// <summary>Before AutoPaper sets the desktop: the pictures that aren't AutoPaper's are copied and kept.</summary>
    private async Task KeepOwnWallpaperAsync()
    {
        try
        {
            var showing = await desktop.ReadAsync();
            if (await Task.Run(() => ownWallpaper.Keep(showing.Pictures, showing.Position, showing.Color, showing.Kind, showing.Slideshow)))
            {
                OnPropertyChanged(nameof(CanRestoreWallpaper));
            }
        }
        catch (Exception error)
        {
            Log.Error("Keeping the person's own wallpaper", error);
        }
    }

    /// <summary>Restore my wallpaper (and quit): the person's own background back on their monitors (their pictures, or
    /// their slideshow), and their own lock screen. AutoPaper's comes back with a new wallpaper or Show on desktop, and,
    /// after a quit, with the next launch. Returns whether anything went back.
    /// Windows Spotlight (desktop or lock screen) and a lock screen slideshow can't be switched back on by an app: the
    /// last picture comes back where AutoPaper could read it, and that's said once, with the link to turn it on
    /// (Settings › General shows the link; when the window isn't in front, such as from the notification area or on
    /// quit, a notification has it as its button).</summary>
    public async Task<bool> RestoreOwnWallpaperAsync(bool announce = true, bool quitting = false)
    {
        var desktopBack = false;
        var spotlightOff = false;
        var failed = false;
        if (!DesktopShowsOwn && ownWallpaper.ToRestore() is { } kept)
        {
            try
            {
                await desktop.RestoreAsync(kept);
                DesktopShowsOwn = true;
                Preferences.UncoveredByQuit = quitting;
                desktopBack = true;
                spotlightOff = kept.Kind == BackgroundKind.Spotlight && !Desktop.SpotlightIsOn();
                OnPropertyChanged(nameof(SpotlightLeftOff));
            }
            catch (Exception error)
            {
                Log.Error("Restoring the person's own wallpaper", error);
                failed = true;
            }
        }
        var lockScreen = await RestoreOwnLockScreenAsync();
        OnPropertyChanged(nameof(CanRestoreWallpaper));
        if (announce)
        {
            if (failed)
            {
                AnnounceError(Loc.Get("OwnWallpaper_RestoreFailed"));
            }
            else if (desktopBack || lockScreen is not null)
            {
                var said = desktopBack ? Loc.Get(spotlightOff ? "OwnWallpaper_RestoredSpotlight" : "OwnWallpaper_Restored") : "";
                if (lockScreen is { } note)
                {
                    said = (said + " " + Text.LockScreenRestored(note)).Trim();
                }
                Announce(said);
            }
        }
        var lockScreenLeftOff = lockScreen is LockScreenNote.Spotlight or LockScreenNote.Slideshow;
        if ((spotlightOff || lockScreenLeftOff) && (quitting || !WindowIsActive()))
        {
            Notifications.SpotlightOff(desktop: spotlightOff, lockScreen: lockScreen);
        }
        return desktopBack || lockScreen is not null;
    }

    /// <summary>The person's own lock screen back, when AutoPaper changed it and it still shows AutoPaper's picture (one
    /// the person chose since stays). Returns what's to be said about it (<see cref="LockScreenNote.None"/>: their
    /// picture is back), or null when there was nothing to do.</summary>
    private async Task<LockScreenNote?> RestoreOwnLockScreenAsync()
    {
        try
        {
            var showing = await Desktop.ReadLockScreenAsync();
            if (await Task.Run(() => ownLockScreen.ToRestore(showing.Picture, showing.Kind)) is not { } plan)
            {
                return null;
            }
            // Spotlight and a slideshow can't be turned on by an app: their last picture goes back where it was read.
            var note = plan.Outcome switch
            {
                OwnLockScreen.Outcome.Spotlight => LockScreenNote.Spotlight,
                OwnLockScreen.Outcome.Slideshow => LockScreenNote.Slideshow,
                OwnLockScreen.Outcome.Unreadable => LockScreenNote.Unreadable,
                _ => LockScreenNote.None,
            };
            if (await Task.Run(ownLockScreen.FreshCopy) is { } copy && await Desktop.SetLockScreenAsync(copy) != LockScreenOutcome.Set)
            {
                note = LockScreenNote.Failed;
            }
            await Task.Run(ownLockScreen.Restored);
            Log.Note($"Put back the person's lock screen: {plan.Outcome}, said {note}");
            LockScreenNote = note;
            return note;
        }
        catch (Exception error)
        {
            Log.Error("Restoring the person's own lock screen", error);
            LockScreenNote = LockScreenNote.Failed;
            return LockScreenNote.Failed;
        }
    }

    /// <summary>AutoPaper's current wallpaper on the desktop again (the launch after a quit put the person's back):
    /// rendered for today's displays, no new mark_shown (it's the same one).</summary>
    private async Task ShowAutoPapersAgainAsync()
    {
        if (Current is not { ImagePath: not null } showing || IsGenerating)
        {
            return;
        }
        making++;
        try
        {
            IsGenerating = true;
            // The lock screen too (when it's set): quitting put the person's own back there as well.
            await PutOnDesktopAsync(showing, markShown: false, setLockScreen: true);
        }
        catch (Exception error)
        {
            Log.Error("Showing AutoPaper's wallpaper again", error);
        }
        finally
        {
            making--;
            FinishStage();
        }
    }

    /// <summary>Re-reads the settings (after a mood's Surprise changed, or a mood switch).</summary>
    public async Task ReloadSettingsAsync()
    {
        Settings = await Call(e => e.Settings());
        await RefreshStatusAsync();
        await ArmAsync();
    }

    // ── Moods ───────────────────────────────────────────────────────────────────────────────────

    /// <summary>Re-reads the moods (after the Moods view changed one) and tells the views; the active mood's Surprise
    /// is Settings' too, so they're read again.</summary>
    public async Task MoodsEditedAsync()
    {
        if (!IsReady)
        {
            return;
        }
        try
        {
            (Moods, Settings) = await Call(e => (e.Moods(), e.Settings()));
        }
        catch (Exception error)
        {
            Log.Error("Reading moods", error);
        }
        OnPropertyChanged(nameof(Moods));
        OnPropertyChanged(nameof(ActiveMood));
        MoodsChanged?.Invoke(this, EventArgs.Empty);
        await RefreshStatusAsync();
        await ArmAsync();
    }

    /// <summary>Makes a mood the one in use (the Moods view's Use, the notification-area menu). Nothing is made by
    /// itself: the next wallpaper uses its keywords and Surprise. Throws the engine's error.</summary>
    public async Task UseMoodAsync(string id)
    {
        await Call(e => e.SetActiveMood(id));
        await MoodsEditedAsync();
        if (ActiveMood is { } now)
        {
            Announce(Loc.Format("Mood_Using", now.Name));
        }
    }

    // ── Keys ────────────────────────────────────────────────────────────────────────────────────

    /// <summary>A key was saved (or removed) in Settings › Keys or the first-run page: views that need it refresh, and
    /// a problem about it on the Now view goes (the next wallpaper will tell if it works).</summary>
    public void KeySaved(string account, bool hasKey)
    {
        if (hasKey && Problem is { Fix: Fix.Keys } problem && problem.KeyAccount == account)
        {
            Problem = null;
            Preferences.KeyProblemNotified = false;
        }
        KeyChanged?.Invoke(this, account);
    }

    public Task SetPausedAsync(bool paused) => UpdateSettingsAsync(settings => settings with { Paused = paused });

    public async Task RefreshStatusAsync()
    {
        if (!IsReady)
        {
            return;
        }
        try
        {
            var (due, spend, narrow, budget) = await Call(e => (e.NextDue(), e.SpendSummary(), e.KeywordsAreNarrow(), e.BudgetStatus()));
            NextText = Text.Next(due, Settings);
            BudgetText = Text.Budget(spend);
            KeywordsNarrow = narrow;
            BudgetStatus = budget;
            // Budget blocks have a persistent notice with the actual estimates, rather than a second transient error.
            if (Problem is { Fix: Fix.Budget })
            {
                Problem = null;
            }
        }
        catch (Exception)
        {
            // Status lines are informational; the next refresh tries again.
        }
    }

    public async Task LoadCurrentAsync()
    {
        var (current, description) = await Call(e =>
        {
            var showing = e.Current();
            return (showing, showing is null ? "" : e.Describe(showing.Id));
        });
        Current = current;
        CurrentDescription = current is null ? "" : Text.Spoken(description, current.Rating);
    }

    /// <summary>History was cleared: the desktop keeps its picture (its display renders stay) until the next one.</summary>
    public async Task HistoryClearedAsync(bool hadCurrent)
    {
        if (hadCurrent)
        {
            DesktopKeepsPicture = true;
        }
        await HistoryEditedAsync();
    }

    /// <summary>After History deletes or clears: the current one may be gone.</summary>
    public async Task HistoryEditedAsync()
    {
        await LoadCurrentAsync();
        await RefreshStatusAsync();
        HistoryChanged?.Invoke(this, EventArgs.Empty);
    }

    // ── Displays ────────────────────────────────────────────────────────────────────────────────

    /// <summary>Displays changed (WM_DISPLAYCHANGE): a new size hint, and the wallpaper rendered again for each.</summary>
    public async Task DisplaysChangedAsync()
    {
        if (!IsReady)
        {
            return;
        }
        await UpdateDisplayHintAsync();
        if (IsGenerating || checking)
        {
            // Rendered again once that's done (a new wallpaper is rendered for the new sizes anyway).
            displaysChangedWhileBusy = true;
            return;
        }
        if (DesktopShowsOwn)
        {
            return; // The person's own is showing: it's theirs to fit.
        }
        if (Current is { ImagePath: not null } showing)
        {
            making++;
            try
            {
                IsGenerating = true;
                await PutOnDesktopAsync(showing, markShown: false, setLockScreen: false);
            }
            catch (Exception error)
            {
                ShowProblem(error, scheduled: true);
            }
            finally
            {
                making--;
                FinishStage();
                await ReplaceWhenFreeAsync();
            }
        }
    }

    /// <summary>set_display_hint with the largest display's pixel size.</summary>
    private async Task UpdateDisplayHintAsync()
    {
        try
        {
            var displays = await desktop.DisplaysAsync();
            if (displays.Count > 0)
            {
                var largest = displays.MaxBy(display => (ulong)display.Width * display.Height)!;
                await Call(e => e.SetDisplayHint(largest.Width, largest.Height));
            }
        }
        catch (Exception)
        {
            // The engine keeps aiming at 3840×2160.
        }
    }

    /// <summary>A polite announcement: a stage, a result, a status (it waits for what's being said).</summary>
    public void Announce(string? text) => Announce(text, AnnouncementKind.Status);

    /// <summary>An error shown on a page (it interrupts).</summary>
    public void AnnounceError(string? text) => Announce(text, AnnouncementKind.Error);

    private void Announce(string? text, AnnouncementKind kind)
    {
        if (!string.IsNullOrEmpty(text))
        {
            Announced?.Invoke(this, new Announcement(text, kind));
        }
    }

    /// <summary>Quit AutoPaper: the person's own wallpaper goes back first (a few seconds at most).</summary>
    public async Task BeforeQuitAsync()
    {
        if (!IsReady)
        {
            return;
        }
        if (IsGenerating)
        {
            Cancel();
        }
        var restore = RestoreOwnWallpaperAsync(announce: false, quitting: true);
        await Task.WhenAny(restore, Task.Delay(TimeSpan.FromSeconds(5)));
    }

    public void Close()
    {
        timer?.Stop();
        desktop.Dispose();
        engine?.SetProgressDetailObserver(null);
        engine?.Dispose();
        engine = null;
    }

    /// <summary>Progress from the engine, on whichever thread polls the generation: marshalled to the UI thread.</summary>
    private sealed class Observer(AppModel model) : ProgressObserver
    {
        public void OnProgress(ProgressStage stage) => model.ui.TryEnqueue(() => model.OnStage(stage));
    }

    /// <summary>Progress in detail, for every generation (New wallpaper now, the schedule, echoes): marshalled too.</summary>
    private sealed class DetailObserver(AppModel model) : ProgressDetailObserver
    {
        public void OnProgressDetail(ProgressDetail detail) => model.ui.TryEnqueue(() => model.OnDetail(detail));
    }
}

/// <summary>Cancel was pressed after the engine made the wallpaper, before it was put on the desktop.</summary>
internal sealed class MadeButNotShownException : Exception;

/// <summary>A sentence for screen readers, and how it's spoken.</summary>
internal sealed record Announcement(string Text, AnnouncementKind Kind);

internal enum AnnouncementKind
{
    /// <summary>A stage, a result or a status: polite (spoken after what's being said; a newer one replaces a
    /// waiting one).</summary>
    Status,

    /// <summary>An error on a page: important (interrupts).</summary>
    Error,

    /// <summary>The Now view's problem: its InfoBar announces itself when it opens, so the window only speaks it
    /// when Now isn't showing.</summary>
    NowProblem,

    /// <summary>The Now view's notice (a wallpaper brought back): as <see cref="NowProblem"/>, but polite.</summary>
    NowNotice,
}
