//! The application: one `Engine`, the schedule, the portals and notifications, and the state every view shows.
//!
//! Lifecycle (docs/app-spec.md "Engine lifecycle"): at startup the engine opens off the main thread, gets the
//! largest display's size, and the schedule starts. A glib timer fires at `next_due()` (re-checked at least every
//! five minutes, since monotonic timers stop during suspend), and resume (logind `PrepareForSleep`) and screen
//! unlock (GNOME's ScreenSaver `ActiveChanged`) check again; `run_if_due` decides. Every wallpaper is rendered for
//! the largest display, set through the Wallpaper portal, then `mark_shown`. The app holds itself open, so it
//! keeps running in the background when its window closes (GNOME lists it under Background Apps); at login the
//! Background portal starts it with `--background`.
//!
//! Views subscribe to `Event`s and read `State`; everything here runs on the GTK main thread, and every engine
//! call goes through `runtime::blocking` / `runtime::spawn`.

use std::cell::{Cell, OnceCell, Ref, RefCell};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use autopaper_core::{
    AutoPaperError, BudgetStatus, DisplayTarget, Engine, EngineConfig, Fallback, Generation, InvalidInputReason, Mood, MoodStats,
    ProgressDetail, ProgressDetailObserver, ProgressObserver, ProgressStage, ProviderJob, ProviderKind,
    ProviderUnavailableReason, Rating, RevisitReason, SecretStore, Settings, Shown, SpendSummary, Trigger,
    secret_account_for,
};
use gtk::{gio, glib};

use crate::desktop::{self, APP_ID, WallpaperProblem};
use crate::preferences::{Fix, ProviderField};
use crate::runtime::{blocking, runtime, spawn};
use crate::secrets::KeyringSecrets;
use crate::strings;
use crate::window::MainWindow;

/// What changed; views re-read `State` (or the engine) for the parts they show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// The engine opened (or failed to: `App::open_error`).
    Ready,
    /// The wallpaper showing, or its rating.
    Current,
    /// A wallpaper started or stopped being made, or moved to its next stage.
    Progress,
    /// The painting's fraction and time left changed (`State::detail`), during a job.
    Detail,
    /// The status lines: next due, budget, the notice, narrow keywords.
    Status,
    Settings,
    /// The moods: one added, renamed, moved, deleted or put to use, or a mood's keywords or Surprise changed.
    Moods,
    /// What each mood has made (`State::mood_stats`): after a new wallpaper, a rating, a delete or a mood change.
    MoodStats,
    History,
    Taste,
    /// A key was saved to (or deleted from) the keyring.
    Keys,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoticeKind {
    Error,
    /// A missing or rejected key: saving a key clears it (the next attempt says if it's still wrong).
    KeyProblem,
    /// A liked wallpaper was brought back instead of a new one.
    Revisit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
    pub kind: NoticeKind,
    /// Where the problem is fixed: the notice is then a link there, and its text says what to do, not where
    /// (docs/app-spec.md 6a).
    pub fix: Option<Fix>,
    /// The link's own words under `text`, when `text` says what happened rather than what to do ("ComfyUI couldn't
    /// paint with Qwen-Image 2.1." + "Check ComfyUI's settings"). `None`: `text` is the link ("Add your OpenAI key").
    pub link: Option<String>,
}

impl Notice {
    fn plain(text: impl Into<String>, kind: NoticeKind) -> Self {
        Self { text: text.into(), kind, fix: None, link: None }
    }
}

#[derive(Default)]
pub struct State {
    pub settings: Settings,
    pub current: Option<Generation>,
    /// `describe(current)` plus its rating in words.
    pub current_spoken: String,
    /// The stage of the wallpaper being made or shown; `None` when idle.
    pub stage: Option<ProgressStage>,
    /// How far the painting is and about how long is left, when something real backs it (ComfyUI's steps, or how
    /// long this painter took here before); `None` shows an indeterminate spinner.
    pub detail: Option<ProgressDetail>,
    /// The job makes a new picture (not just a re-render or a picture put back): the Now view veils the old one.
    pub making: bool,
    /// Every mood in the person's order; exactly one is `active`.
    pub moods: Vec<Mood>,
    /// What each mood has made (`Engine::mood_stats`), in the moods' order.
    pub mood_stats: Vec<MoodStats>,
    pub notice: Option<Notice>,
    pub next_due: Option<i64>,
    /// When the next scheduled wallpaper starts so it's ready by `next_due` (the core's `next_start`).
    pub next_start: Option<i64>,
    pub spend: Option<SpendSummary>,
    /// The current cap, estimated next-run cost and exact reason new wallpapers are blocked.
    pub budget: Option<BudgetStatus>,
    pub narrow: bool,
    /// The Background portal said no: new wallpapers only come while the window is open.
    pub background_denied: bool,
    /// How the last job ended, for screen readers ("New wallpaper: …", "Stopped."); taken by the view that
    /// announces it. Problems aren't here: they're the notice, which is announced when it changes.
    pub outcome: Option<String>,
}

type Listener = Rc<dyn Fn(Event) -> bool>;
type AppAction = Box<dyn Fn(&Rc<App>)>;
type AppActionWithId = Box<dyn Fn(&Rc<App>, String)>;

/// A change to the engine's stored state, run by the one writer in the order asked for (see `App::serial`).
type EngineJob = Box<dyn FnOnce(&Engine) + Send>;

/// A key typed into Preferences, waiting for the typing to pause before it goes to the keyring.
struct PendingKey {
    value: String,
    source: glib::SourceId,
    /// Where to say it couldn't be saved (the dialog may be gone by then; the Now view's notice says it instead).
    dialog: glib::WeakRef<adw::PreferencesDialog>,
}

/// How long typing in a key field pauses before the key is saved (AudioPaper's Keychain fields save as you type).
const KEY_SAVE_DELAY: std::time::Duration = std::time::Duration::from_millis(600);
/// How long Quit waits for the keyring to save keys typed just before it (an unlock prompt may be showing).
const QUIT_KEY_WAIT: std::time::Duration = std::time::Duration::from_secs(60);

pub struct App {
    pub gtk: adw::Application,
    /// The Linux app's own settings (GSettings): notifications, open at login, window size, first run.
    pub prefs: gio::Settings,
    pub secrets: Arc<KeyringSecrets>,
    engine: OnceCell<Arc<Engine>>,
    open_error: RefCell<Option<String>>,
    state: RefCell<State>,
    listeners: RefCell<Vec<Listener>>,
    window: RefCell<Option<Rc<MainWindow>>>,
    timer: RefCell<Option<glib::SourceId>>,
    hold: RefCell<Option<gio::ApplicationHoldGuard>>,
    /// A wallpaper is being made (one at a time; the engine would queue a second).
    busy: Cell<bool>,
    /// Settings, moods and Surprise are changed by one writer, in order (see `serial`).
    writer: OnceCell<async_channel::Sender<EngineJob>>,
    settings_pending: Cell<u32>,
    /// Ratings asked for before the engine opened (a notification's Like or Dislike that started AutoPaper),
    /// applied once it has.
    pending_ratings: RefCell<Vec<(String, Rating)>>,
    /// Signal subscriptions for resume and unlock, kept alive for the app's lifetime.
    wake_subscriptions: RefCell<Vec<gio::SignalSubscription>>,
    /// The id of the wallpaper the "New wallpaper" notification is about.
    notified_id: RefCell<Option<String>>,
    /// Monitors whose geometry and scale are watched, with their handlers.
    monitor_handlers: RefCell<Vec<(glib::WeakRef<gtk::gdk::Monitor>, Vec<glib::SignalHandlerId>)>>,
    /// A display change waiting to settle.
    display_change: RefCell<Option<glib::SourceId>>,
    /// Keys typed but not saved yet, by account. They belong to the app, not to Preferences: closing the dialog
    /// (or quitting) right after typing still saves, or deletes, the key.
    pending_keys: RefCell<std::collections::HashMap<String, PendingKey>>,
    /// Keys being saved to the keyring now (Quit waits for them).
    key_saves: Cell<u32>,
    /// GNOME's background settings and the Wallpaper portal's copy, watched to hear someone put up another wallpaper
    /// (`watch_background`).
    background: RefCell<Option<(gio::Settings, Option<gio::FileMonitor>)>>,
    /// AutoPaper is handing a picture to the Wallpaper portal: the portal's copy and the background keys change
    /// under it, which isn't the person choosing another wallpaper.
    setting_wallpaper: Cell<bool>,
    #[cfg(feature = "tray")]
    tray: RefCell<Option<crate::tray::Tray>>,
}

impl App {
    pub fn new() -> Result<Rc<Self>, String> {
        let gtk = adw::Application::builder()
            .application_id(APP_ID)
            .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
            .build();
        gtk.add_main_option(
            "background",
            glib::Char::from(b'b'),
            glib::OptionFlags::NONE,
            glib::OptionArg::None,
            "Start in the background without opening the window (used at login)",
            None,
        );
        let app = Rc::new(Self {
            gtk,
            prefs: load_prefs()?,
            secrets: KeyringSecrets::start(),
            engine: OnceCell::new(),
            open_error: RefCell::new(None),
            state: RefCell::new(State::default()),
            listeners: RefCell::new(Vec::new()),
            window: RefCell::new(None),
            timer: RefCell::new(None),
            hold: RefCell::new(None),
            busy: Cell::new(false),
            writer: OnceCell::new(),
            settings_pending: Cell::new(0),
            pending_ratings: RefCell::new(Vec::new()),
            wake_subscriptions: RefCell::new(Vec::new()),
            notified_id: RefCell::new(None),
            monitor_handlers: RefCell::new(Vec::new()),
            display_change: RefCell::new(None),
            pending_keys: RefCell::new(std::collections::HashMap::new()),
            key_saves: Cell::new(0),
            background: RefCell::new(None),
            setting_wallpaper: Cell::new(false),
            #[cfg(feature = "tray")]
            tray: RefCell::new(None),
        });
        app.connect_lifecycle();
        Ok(app)
    }

    pub fn run(&self) -> glib::ExitCode {
        self.gtk.run()
    }

    fn connect_lifecycle(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.gtk.connect_startup(move |_| {
            if let Some(app) = weak.upgrade() {
                app.startup();
            }
        });
        let weak = Rc::downgrade(self);
        self.gtk.connect_activate(move |_| {
            if let Some(app) = weak.upgrade() {
                app.present_window();
            }
        });
        self.gtk.connect_command_line(|gtk_app, command_line| {
            // `--background` (autostart): keep running without a window. Anything else opens the window.
            if !command_line.options_dict().contains("background") {
                gtk_app.activate();
            }
            glib::ExitCode::SUCCESS
        });
    }

    fn startup(self: &Rc<Self>) {
        // A background app: closing the window doesn't quit (Quit does).
        self.hold.replace(Some(self.gtk.hold()));
        self.install_actions();
        let app = self.clone();
        glib::spawn_future_local(async move { app.open_engine().await });
    }

    // ── The engine ──────────────────────────────────────────────────────────────────────────────

    pub fn engine(&self) -> Option<Arc<Engine>> {
        self.engine.get().cloned()
    }

    /// Why the engine couldn't open (shown in the window instead of the views).
    pub fn open_error(&self) -> Option<String> {
        self.open_error.borrow().clone()
    }

    pub fn is_ready(&self) -> bool {
        self.engine.get().is_some()
    }

    async fn open_engine(self: &Rc<Self>) {
        desktop::register_with_portals().await;
        let data_dir = desktop::data_dir();
        let model_dir = desktop::model_dir();
        tracing::info!(data = %data_dir.display(), model = %model_dir.display(), "opening the engine");
        let config = EngineConfig {
            data_dir: data_dir.to_string_lossy().into_owned(),
            model_dir: model_dir.to_string_lossy().into_owned(),
            locale: desktop::locale(),
            client: desktop::client(),
        };
        let secrets: Arc<dyn SecretStore> = self.secrets.clone();
        let engine = match blocking(move || Engine::open(config, secrets)).await {
            Ok(engine) => engine,
            Err(error) => {
                tracing::error!(%error, "the engine couldn't open");
                self.open_error.replace(Some(format!(
                    "AutoPaper couldn't open its data in {}. {}",
                    data_dir.display(),
                    strings::error_sentence(&error, Fallback::KeepCurrent)
                )));
                self.emit(Event::Ready);
                return;
            }
        };
        let _ = self.engine.set(engine.clone());
        self.watch_progress_detail(&engine);
        self.start_writer(engine);
        self.watch_displays();
        self.watch_background();
        self.send_display_hint().await;
        self.refresh_settings().await;
        self.refresh_moods().await;
        self.refresh_mood_stats().await;
        self.refresh_current().await;
        self.refresh_status().await;
        self.update_busy_actions();
        self.emit(Event::Ready);
        // A notification's Like or Dislike may have started AutoPaper: apply it now that the engine is open.
        let ratings: Vec<(String, Rating)> = self.pending_ratings.take();
        for (id, rating) in ratings {
            self.rate(id, rating);
        }
        self.watch_wake();
        self.start_tray();
        // Nothing happens behind the welcome: no wallpaper and no login item until it's finished or skipped.
        if !crate::welcome::Welcome::wanted(self) {
            self.apply_open_at_login().await;
            // At launch, as at the due time: run_if_due decides whether one is due.
            self.check_schedule();
        }
    }

    // ── State and events ────────────────────────────────────────────────────────────────────────

    pub fn state(&self) -> Ref<'_, State> {
        self.state.borrow()
    }

    /// Calls `listener` on every event until it returns false (a view that's gone).
    pub fn subscribe(&self, listener: impl Fn(Event) -> bool + 'static) {
        self.listeners.borrow_mut().push(Rc::new(listener));
    }

    pub fn emit(&self, event: Event) {
        let listeners: Vec<Listener> = self.listeners.borrow().clone();
        let mut gone = Vec::new();
        for listener in &listeners {
            if !listener(event) {
                gone.push(listener.clone());
            }
        }
        if !gone.is_empty() {
            self.listeners.borrow_mut().retain(|kept| !gone.iter().any(|g| Rc::ptr_eq(g, kept)));
        }
        #[cfg(feature = "tray")]
        if matches!(event, Event::Current | Event::Progress | Event::Settings | Event::Ready | Event::Moods)
            && let Some(tray) = self.tray.borrow().as_ref() {
                tray.refresh(self);
            }
        if matches!(event, Event::Moods | Event::Ready)
            && let Some(action) = self.gtk.lookup_action("mood")
            && let Some(active) = self.active_mood()
        {
            action.change_state(&active.id.to_variant());
        }
        if event == Event::Settings
            && let Some(action) = self.gtk.lookup_action("pause") {
                action.change_state(&self.state.borrow().settings.paused.to_variant());
            }
    }

    fn set_stage(&self, stage: Option<ProgressStage>) {
        let stage = stage.filter(|stage| *stage != ProgressStage::Done);
        if self.state.borrow().stage == stage {
            return;
        }
        {
            let mut state = self.state.borrow_mut();
            state.stage = stage;
            // Numbers belong to the stage they came with (painting); the next stage starts without them.
            if state.detail.is_some_and(|detail| Some(detail.stage) != stage) {
                state.detail = None;
            }
        }
        self.emit(Event::Progress);
    }

    /// The painting's fraction and time left, from the engine-wide detail observer. Only while a job runs (a late
    /// report after it ended is dropped), and only numbers: the stage comes from the job's own observer.
    fn set_detail(&self, detail: ProgressDetail) {
        if !self.busy.get() || detail.stage == ProgressStage::Done {
            return;
        }
        let detail = (detail.fraction.is_some() || detail.seconds_left.is_some()).then_some(detail);
        if self.state.borrow().detail == detail {
            return;
        }
        self.state.borrow_mut().detail = detail;
        self.emit(Event::Detail);
    }

    fn watch_progress_detail(self: &Rc<Self>, engine: &Arc<Engine>) {
        let (sender, receiver) = async_channel::unbounded::<ProgressDetail>();
        engine.set_progress_detail_observer(Some(Arc::new(DetailSender(sender))));
        let app = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(detail) = receiver.recv().await {
                match app.upgrade() {
                    Some(app) => app.set_detail(detail),
                    None => break,
                }
            }
        });
    }

    /// Starts or ends a job (making, showing or re-rendering a wallpaper) with its first stage, in one `Progress`
    /// event. Actions that become available are enabled before it and the ones that go away disabled after it, so
    /// a view can move keyboard focus off a control it's about to hide or disable onto one that's there (New
    /// wallpaper now → Cancel and back): a focused control that goes away would leave focus nowhere.
    fn set_busy(&self, busy: bool, stage: Option<ProgressStage>) {
        self.set_job(busy, stage, false);
    }

    /// As `set_busy`; `making` marks a job that makes a new picture (generate, an echo, a scheduled run).
    fn set_job(&self, busy: bool, stage: Option<ProgressStage>, making: bool) {
        self.busy.set(busy);
        {
            let mut state = self.state.borrow_mut();
            state.stage = stage.filter(|stage| *stage != ProgressStage::Done);
            state.detail = None;
            state.making = busy && making;
            if busy {
                state.outcome = None;
            }
        }
        self.apply_busy_actions(true);
        self.emit(Event::Progress);
        self.apply_busy_actions(false);
    }

    /// How the job that's ending went, for `set_busy(false, …)` to hand to the views.
    fn set_outcome(&self, outcome: Option<String>) {
        self.state.borrow_mut().outcome = outcome;
    }

    /// The last job's outcome, once (the Now view announces it).
    pub fn take_outcome(&self) -> Option<String> {
        self.state.borrow_mut().outcome.take()
    }

    fn set_notice(&self, notice: Option<Notice>) {
        if self.state.borrow().notice == notice {
            return;
        }
        self.state.borrow_mut().notice = notice;
        self.emit(Event::Status);
    }

    fn set_error(&self, error: &AutoPaperError) {
        if matches!(error, AutoPaperError::Cancelled) {
            self.set_notice(None);
            return;
        }
        tracing::warn!(%error, "AutoPaper couldn't finish");
        let fallback = self.state.borrow().settings.fallback;
        let kind = match error {
            AutoPaperError::MissingKey { .. } | AutoPaperError::InvalidKey { .. } => NoticeKind::KeyProblem,
            _ => NoticeKind::Error,
        };
        let fix = {
            let state = self.state.borrow();
            fix_for(&state.settings, &state.moods, error)
        };
        // With a link to the fix, the sentence says what to do ("Add your OpenAI key"), not where.
        let text = match fix {
            Some(Fix::Budget) => self.state.borrow().budget.as_ref().filter(|budget| budget.blocked)
                .map(|budget| budget.message.clone()).unwrap_or_else(|| strings::error_sentence(error, fallback)),
            Some(_) => strings::linked_sentence(error, fallback),
            None => strings::error_sentence(error, fallback),
        };
        // A painting failure says what happened; the link under it goes to the painting settings.
        let link = match error {
            AutoPaperError::BudgetReached { .. } => Some("Change budget".to_string()),
            AutoPaperError::PaintingFailed { provider, .. } if fix.is_some() => Some(strings::painting_link(*provider)),
            _ => None,
        };
        self.set_notice(Some(Notice { text, kind, fix, link }));
    }

    pub async fn refresh_settings(&self) {
        let Some(engine) = self.engine() else { return };
        match blocking(move || engine.settings()).await {
            Ok(settings) => {
                self.state.borrow_mut().settings = settings;
                self.emit(Event::Settings);
            }
            Err(error) => tracing::warn!(%error, "couldn't read the settings"),
        }
    }

    pub async fn refresh_current(&self) {
        let Some(engine) = self.engine() else { return };
        let read = blocking(move || {
            let current = engine.current()?;
            let spoken = match &current {
                Some(generation) => strings::spoken_description(
                    &engine.describe(generation.id.clone()).unwrap_or_default(),
                    generation.rating,
                ),
                None => String::new(),
            };
            Ok((current, spoken))
        })
        .await;
        match read {
            Ok((current, spoken)) => {
                {
                    let mut state = self.state.borrow_mut();
                    state.current = current;
                    state.current_spoken = spoken;
                }
                self.emit(Event::Current);
            }
            Err(error) => tracing::warn!(%error, "couldn't read the current wallpaper"),
        }
    }

    /// Next due, this month's spend and the narrow-keywords flag; then re-arms the timer.
    pub async fn refresh_status(self: &Rc<Self>) {
        let Some(engine) = self.engine() else { return };
        let read = blocking(move || {
            let next_due = engine.next_due()?;
            let next_start = engine.next_start()?;
            let spend = engine.spend_summary().ok();
            let budget = engine.budget_status()?;
            let narrow = engine.keywords_are_narrow().unwrap_or(false);
            Ok((next_due, next_start, spend, budget, narrow))
        })
        .await;
        match read {
            Ok((next_due, next_start, spend, budget, narrow)) => {
                {
                    let mut state = self.state.borrow_mut();
                    state.next_due = next_due;
                    state.next_start = next_start;
                    state.spend = spend;
                    // Refresh a previous budget failure when settings change; keep other errors alongside the global
                    // budget notice. A higher limit or a new month clears only the budget error.
                    if state.notice.as_ref().is_some_and(|notice| notice.fix == Some(Fix::Budget)) {
                        if budget.blocked {
                            if let Some(notice) = &mut state.notice {
                                notice.text = budget.message.clone();
                            }
                        } else {
                            state.notice = None;
                        }
                    }
                    state.budget = Some(budget);
                    state.narrow = narrow;
                }
                self.emit(Event::Status);
            }
            Err(error) => tracing::warn!(%error, "couldn't read the schedule"),
        }
        self.reschedule();
    }

    // ── Settings ────────────────────────────────────────────────────────────────────────────────

    fn start_writer(&self, engine: Arc<Engine>) {
        let (sender, receiver) = async_channel::unbounded::<EngineJob>();
        // One writer, in order: two quick changes can't land out of order, and a mood switch can't slip between
        // reading the settings and saving them.
        runtime().spawn(async move {
            while let Ok(job) = receiver.recv().await {
                let engine = engine.clone();
                if let Err(error) = tokio::task::spawn_blocking(move || job(&engine)).await {
                    tracing::warn!(%error, "a change to AutoPaper's data failed");
                }
            }
        });
        let _ = self.writer.set(sender);
    }

    /// Runs `op` on the engine (off the main thread) after every change queued before it. Everything that changes
    /// settings, moods or a mood's keywords goes through here, so they land in the order they were made.
    pub async fn serial<T: Send + 'static>(
        &self,
        op: impl FnOnce(&Engine) -> Result<T, AutoPaperError> + Send + 'static,
    ) -> Result<T, AutoPaperError> {
        let Some(queue) = self.writer.get().cloned() else {
            return Err(AutoPaperError::Internal { detail: "the engine isn't open".into() });
        };
        let (reply, answer) = tokio::sync::oneshot::channel();
        let job: EngineJob = Box::new(move |engine| {
            let _ = reply.send(op(engine));
        });
        if queue.send(job).await.is_err() {
            return Err(AutoPaperError::Internal { detail: "the writer stopped".into() });
        }
        answer.await.unwrap_or_else(|_| Err(AutoPaperError::Internal { detail: "no answer".into() }))
    }

    /// Changes the settings and saves them. Views update at once (the change is applied to `State` first); if
    /// the engine refuses it (a bad server address), the saved settings come back and the error is returned.
    /// Surprise isn't changed here: it belongs to a mood (`set_mood_surprise`). The core saves `Settings::surprise`
    /// to whichever mood is active, so the writer puts back the active mood's own value first, and a copy read
    /// before a mood switch can't hand the old mood's Surprise to the new one.
    pub async fn change_settings(self: &Rc<Self>, change: impl FnOnce(&mut Settings)) -> Result<(), AutoPaperError> {
        let providers = |settings: &Settings| {
            (settings.text_provider.clone(), settings.image_provider.clone(), settings.comfyui_workflow.clone())
        };
        let (settings, before) = {
            let mut state = self.state.borrow_mut();
            let before = providers(&state.settings);
            change(&mut state.settings);
            (state.settings.clone(), before)
        };
        let providers_changed = providers(&settings) != before;
        self.emit(Event::Settings);
        self.settings_pending.set(self.settings_pending.get() + 1);
        let result = self
            .serial(move |engine| {
                let mut settings = settings;
                settings.surprise = engine.settings()?.surprise;
                engine.update_settings(settings)
            })
            .await;
        self.settings_pending.set(self.settings_pending.get().saturating_sub(1));
        // A problem whose link goes to the provider settings is fixed (or at least changed) once they are: it goes,
        // as a key problem goes when a key is saved. So does a missing or refused key of a provider that's no longer
        // chosen: its link would lead to a key nothing uses. The next attempt says if a problem is still there.
        let stale = {
            let state = self.state.borrow();
            match &state.notice {
                Some(Notice { fix: Some(Fix::Provider(..)), .. }) => true,
                Some(Notice { kind: NoticeKind::KeyProblem, fix: Some(Fix::Key(account)), .. }) => {
                    !uses_key(&state.settings, account)
                }
                _ => false,
            }
        };
        if result.is_ok() && providers_changed && stale {
            if matches!(&self.state.borrow().notice, Some(Notice { kind: NoticeKind::KeyProblem, .. })) {
                self.key_problem_gone();
            }
            self.set_notice(None);
        }
        // Read back what was stored (trimmed and clamped, or the old settings after an error) once the last
        // pending change has landed, so a slider being dragged doesn't jump back.
        if self.settings_pending.get() == 0 || result.is_err() {
            self.refresh_settings().await;
        }
        self.refresh_status().await;
        result
    }

    // ── Moods ───────────────────────────────────────────────────────────────────────────────────

    /// The mood in use (`None` only before the engine opens).
    pub fn active_mood(&self) -> Option<Mood> {
        self.state.borrow().moods.iter().find(|mood| mood.active).cloned()
    }

    /// Reads the moods (with their keywords and Surprise) and tells the views when they changed.
    pub async fn refresh_moods(&self) {
        let Some(engine) = self.engine() else { return };
        match blocking(move || engine.moods()).await {
            Ok(moods) => {
                if self.state.borrow().moods == moods {
                    return;
                }
                self.state.borrow_mut().moods = moods;
                self.emit(Event::Moods);
            }
            Err(error) => tracing::warn!(%error, "couldn't read the moods"),
        }
    }

    /// Reads what each mood has made (counts, liked, echoes, last made, newest few) and tells the views when it changed.
    pub async fn refresh_mood_stats(&self) {
        let Some(engine) = self.engine() else { return };
        match blocking(move || engine.mood_stats()).await {
            Ok(stats) => {
                if self.state.borrow().mood_stats == stats {
                    return;
                }
                self.state.borrow_mut().mood_stats = stats;
                self.emit(Event::MoodStats);
            }
            Err(error) => tracing::warn!(%error, "couldn't read what the moods have made"),
        }
    }

    /// Runs a change to the moods or a mood's keywords in order with the others, then re-reads the moods (and, when
    /// the mood in use or its Surprise may have changed, the settings and the schedule).
    pub async fn change_moods<T: Send + 'static>(
        self: &Rc<Self>,
        affects_active: bool,
        op: impl FnOnce(&Engine) -> Result<T, AutoPaperError> + Send + 'static,
    ) -> Result<T, AutoPaperError> {
        let result = self.serial(op).await;
        self.refresh_moods().await;
        self.refresh_mood_stats().await;
        if affects_active && result.is_ok() {
            self.refresh_settings().await;
            self.refresh_status().await;
        }
        result
    }

    /// Puts a mood to use. Nothing is made by itself: the next wallpaper, scheduled or asked for, uses it.
    pub fn use_mood(self: &Rc<Self>, id: String) {
        let app = self.clone();
        glib::spawn_future_local(async move {
            if app.active_mood().is_some_and(|active| active.id == id) {
                return;
            }
            match app.change_moods(true, move |engine| engine.set_active_mood(id)).await {
                Ok(()) => {
                    if let Some(active) = app.active_mood() {
                        app.announce(&strings::mood_in_use(&active.name));
                    }
                }
                Err(error) => app.toast(&strings::error_sentence(&error, app.state.borrow().settings.fallback)),
            }
        });
    }

    /// Says something politely to screen readers through the window, when it's open.
    pub fn announce(&self, text: &str) {
        if let Some(window) = self.window_widget().filter(|window| window.is_mapped()) {
            window.announce(text, gtk::AccessibleAnnouncementPriority::Medium);
        }
    }

    /// A toast in the main window (a problem from the tray or the header's mood menu), or the log without one.
    pub fn toast(&self, text: &str) {
        match self.main_window() {
            Some(window) => window.toast(text),
            None => tracing::warn!(text, "nowhere to show this"),
        }
    }

    // ── Making and showing wallpapers ───────────────────────────────────────────────────────────

    pub fn is_busy(&self) -> bool {
        self.busy.get()
    }

    /// "New wallpaper now" (and a disliked one's replacement).
    pub fn new_wallpaper(self: &Rc<Self>, trigger: Trigger) {
        self.make(Job::New(trigger));
    }

    /// A mood's "Use this mood and make a new wallpaper" (its header's reload button, Ctrl+R on its page): the mood
    /// becomes current, then a new wallpaper is made from it. If the switch fails, that's said and nothing is made.
    /// The current mood's is plain New wallpaper now.
    pub fn new_wallpaper_from(self: &Rc<Self>, mood_id: String) {
        if self.busy.get() || !self.is_ready() {
            return;
        }
        if self.active_mood().is_some_and(|active| active.id == mood_id) {
            self.new_wallpaper(Trigger::Manual);
            return;
        }
        let app = self.clone();
        glib::spawn_future_local(async move {
            match app.change_moods(true, move |engine| engine.set_active_mood(mood_id)).await {
                Ok(()) => {
                    if let Some(active) = app.active_mood() {
                        app.announce(&strings::mood_in_use(&active.name));
                    }
                    app.new_wallpaper(Trigger::Manual);
                }
                Err(error) => app.toast(&strings::error_sentence(&error, app.state.borrow().settings.fallback)),
            }
        });
    }

    /// "Make an echo" on a past wallpaper.
    pub fn make_echo(self: &Rc<Self>, id: String) {
        self.make(Job::Echo(id));
    }

    pub fn cancel(&self) {
        if let Some(engine) = self.engine() {
            // `cancel` only bumps a counter, but every engine call stays off the main thread.
            runtime().spawn_blocking(move || engine.cancel());
        }
    }

    /// Marks a wallpaper as being made. `first_stage` shows at once (a manual request); a scheduled run shows
    /// the engine's stages as they come.
    fn begin(self: &Rc<Self>, first_stage: Option<ProgressStage>) -> Option<(Arc<Engine>, Arc<dyn ProgressObserver>)> {
        if self.busy.get() {
            return None;
        }
        let engine = self.engine()?;
        self.set_job(true, first_stage, true);
        let (sender, receiver) = async_channel::unbounded::<ProgressStage>();
        let app = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            // Ends when the engine drops the observer (the generation finished).
            while let Ok(stage) = receiver.recv().await {
                match app.upgrade() {
                    Some(app) if app.busy.get() => app.set_stage(Some(stage)),
                    _ => break,
                }
            }
        });
        Some((engine, Arc::new(ProgressSender(sender))))
    }

    /// Ends a job: `outcome` is what screen readers hear ("New wallpaper: …", "Stopped."), or `None` when the
    /// notice already says what happened.
    async fn finish(self: &Rc<Self>, outcome: Option<String>) {
        self.set_outcome(outcome);
        self.set_busy(false, None);
        self.refresh_status().await;
        self.refresh_mood_stats().await;
        self.emit(Event::History);
        self.emit(Event::Taste);
    }

    fn make(self: &Rc<Self>, job: Job) {
        if self.busy.get() || !self.is_ready() {
            return;
        }
        // A problem from the last attempt doesn't stay on screen while this one runs; if it's still there, this
        // attempt says so again when it ends.
        if !matches!(&self.state.borrow().notice, Some(Notice { kind: NoticeKind::Revisit, .. }) | None) {
            self.set_notice(None);
        }
        let Some((engine, observer)) = self.begin(Some(ProgressStage::CheckingServices)) else { return };
        let app = self.clone();
        glib::spawn_future_local(async move {
            let result = spawn(async move {
                match job {
                    Job::New(trigger) => engine.generate_or_revisit(trigger, Some(observer)).await,
                    Job::Echo(id) => engine.make_echo_or_revisit(id, Some(observer)).await,
                }
            })
            .await;
            let mut outcome = None;
            match result {
                Ok(shown) => outcome = app.show_result(shown).await,
                Err(error) => {
                    if matches!(error, AutoPaperError::Cancelled) {
                        outcome = Some(strings::error_sentence(&error, Fallback::KeepCurrent));
                    }
                    app.set_error(&error);
                }
            }
            app.finish(outcome).await;
        });
    }

    /// Called on the timer, at launch, and on resume/unlock.
    pub fn check_schedule(self: &Rc<Self>) {
        // `next_start` is fresh (read after every change and before a wake check); only a due slot runs the engine.
        let due = self.starts_by(unix_now());
        if !due || self.busy.get() || self.state.borrow().settings.paused || crate::welcome::Welcome::wanted(self) {
            return;
        }
        let Some((engine, observer)) = self.begin(Some(ProgressStage::CheckingServices)) else { return };
        let app = self.clone();
        glib::spawn_future_local(async move {
            let mut outcome = None;
            match spawn(async move { engine.run_if_due(Some(observer)).await }).await {
                Ok(None) => {}
                Ok(Some(shown)) => outcome = app.show_result(shown).await,
                Err(error) => {
                    if matches!(error, AutoPaperError::Cancelled) {
                        outcome = Some(strings::error_sentence(&error, Fallback::KeepCurrent));
                    }
                    app.set_error(&error);
                    match error {
                        AutoPaperError::BudgetReached { .. } => app.notify_budget_spent(),
                        AutoPaperError::MissingKey { provider } => app.notify_key_problem(provider, false),
                        AutoPaperError::InvalidKey { provider } => app.notify_key_problem(provider, true),
                        _ => {}
                    }
                }
            }
            app.finish(outcome).await;
        });
    }

    /// Manual, echo and scheduled results share the same presentation. A saved wallpaper's reason is the notice,
    /// while only a newly generated wallpaper clears key problems and gets a new-wallpaper announcement.
    async fn show_result(self: &Rc<Self>, shown: Shown) -> Option<String> {
        let revisit = shown.revisit;
        if revisit.is_none() {
            self.made_new();
        }
        let mut outcome = None;
        if self.show(&shown.generation, true).await {
            match revisit.and_then(strings::revisit_sentence) {
                Some(text) => self.set_notice(Some(Notice::plain(text, NoticeKind::Revisit))),
                None => self.set_notice(None),
            }
            if revisit.is_none() {
                self.notify_new(&shown.generation);
                outcome = Some(strings::new_wallpaper(&shown.generation.concept.title));
            }
        }
        if revisit == Some(RevisitReason::OverBudget) {
            self.notify_budget_spent();
        }
        outcome
    }

    /// A new wallpaper was made: the providers and keys work again.
    fn made_new(&self) {
        if self.prefs.boolean("key-problem-notified") {
            let _ = self.prefs.set_boolean("key-problem-notified", false);
        }
    }

    /// A key problem was dealt with (a key saved, or its provider no longer chosen): its notification goes too, and a
    /// problem that comes back is notified again.
    fn key_problem_gone(&self) {
        self.gtk.withdraw_notification("keys");
        self.made_new();
    }

    /// "Show on desktop" from History: a revisit, which doesn't restart the schedule.
    pub fn show_on_desktop(self: &Rc<Self>, generation: Generation) {
        if self.busy.get() || !self.is_ready() {
            return;
        }
        self.set_busy(true, Some(ProgressStage::Rendering));
        let app = self.clone();
        glib::spawn_future_local(async move {
            let mut outcome = None;
            if app.show(&generation, true).await {
                app.set_notice(None);
                outcome = Some(strings::on_desktop(&generation.concept.title));
            }
            app.finish(outcome).await;
        });
    }

    /// Renders `generation` for the largest display, sets it through the portal, and (with `mark`) tells the
    /// engine it's showing. False (with the reason in the status line) when it couldn't be set.
    async fn show(self: &Rc<Self>, generation: &Generation, mark: bool) -> bool {
        let Some(engine) = self.engine() else { return false };
        self.set_stage(Some(ProgressStage::Rendering));
        let display = desktop::largest_display()
            .unwrap_or(DisplayTarget { id: "default".into(), width: 3840, height: 2160 });
        let id = generation.id.clone();
        let render_engine = engine.clone();
        let path = match blocking(move || render_engine.render_for_display(id, display)).await {
            Ok(path) => path,
            Err(error) => {
                self.set_error(&error);
                return false;
            }
        };
        let lock_screen = self.state.borrow().settings.set_lock_screen;
        self.remember_own_wallpaper().await;
        self.setting_wallpaper.set(true);
        let set = desktop::set_wallpaper(Path::new(&path), lock_screen, self.window_widget()).await;
        self.setting_wallpaper.set(false);
        match set {
            Ok(()) => self.note_own_set(),
            Err(problem) => {
                let text = match problem {
                    // GNOME Settings has no switch for this permission; resetting the app's portal permissions makes
                    // the desktop ask again.
                    WallpaperProblem::Declined => {
                        "AutoPaper isn't allowed to change the wallpaper. To be asked again, run \
                         “flatpak permission-reset io.github.msitarzewski.AutoPaper” in a terminal."
                    }
                    WallpaperProblem::Unavailable(detail) => {
                        tracing::warn!(detail, "the Wallpaper portal failed");
                        "AutoPaper couldn't set the wallpaper: the desktop's wallpaper service didn't answer."
                    }
                };
                self.set_notice(Some(Notice::plain(text, NoticeKind::Error)));
                return false;
            }
        }
        if mark {
            let id = generation.id.clone();
            let mark_engine = engine.clone();
            if let Err(error) = blocking(move || mark_engine.mark_shown(id)).await {
                tracing::warn!(%error, "couldn't record the wallpaper as shown");
            }
        }
        self.refresh_current().await;
        true
    }

    // ── The person's own wallpaper (docs/app-spec.md 3a) ──────────────────────────────────────────

    /// The person's own wallpaper settings, recorded while AutoPaper's is on the desktop (empty otherwise).
    fn own_wallpaper(&self) -> std::collections::HashMap<String, String> {
        self.prefs.value("own-wallpaper").get::<std::collections::HashMap<String, String>>().unwrap_or_default()
    }

    /// Whether AutoPaper's wallpaper is the one on the desktop. On GNOME: the person's own is recorded and nobody has
    /// put another one up since (`replaced`). Elsewhere it can't be told, so AutoPaper's is taken to be.
    fn ours_on_desktop(&self) -> bool {
        !desktop::can_restore_wallpaper() || (!self.own_wallpaper().is_empty() && !self.replaced())
    }

    /// Whether someone put up another wallpaper after AutoPaper's (the person in Settings, or another app): a picture
    /// key names something that's neither the Wallpaper portal's copy (AutoPaper's) nor what was recorded as theirs
    /// (a key still naming that is AutoPaper's own change on its way: the keys are written one at a time); or both
    /// name the portal's copy, but the file isn't the one AutoPaper left (another app's picture set through the portal
    /// under the same name). False with nothing recorded.
    fn replaced(&self) -> bool {
        let record = self.own_wallpaper();
        if record.is_empty() {
            return false;
        }
        let Some(now) = desktop::picture_settings() else { return false };
        let portal = desktop::portal_uri();
        if now.iter().any(|(key, value)| *value != portal && record.get(key) != Some(value)) {
            return true;
        }
        // The portal's copy changes under AutoPaper while it sets one; and while the keys move over to it, it's too
        // early to compare.
        if self.setting_wallpaper.get() || !now.iter().all(|(_, value)| *value == portal) {
            return false;
        }
        let noted = self.prefs.string("own-wallpaper-set");
        !noted.is_empty() && desktop::portal_copy_identity().as_deref() != Some(noted.as_str())
    }

    /// The person picked another wallpaper while AutoPaper's was showing (in Settings, or through another app): theirs
    /// is on the desktop now, so the record of the one before is dropped. Putting that back on Quit or Restore
    /// would undo their choice, and a display change mustn't paint AutoPaper's over it. True when it was dropped.
    fn forget_own_wallpaper_if_replaced(&self) -> bool {
        if !desktop::can_restore_wallpaper() || !self.replaced() {
            return false;
        }
        tracing::info!("another wallpaper was chosen while AutoPaper's showed; it's the person's own now");
        self.clear_own_wallpaper();
        true
    }

    /// Nothing of the person's is waiting to go back (theirs is showing).
    fn clear_own_wallpaper(&self) {
        let _ = self.prefs.set_value("own-wallpaper", &std::collections::HashMap::<String, String>::new().to_variant());
        let _ = self.prefs.set_string("own-wallpaper-set", "");
        self.update_restore_action();
    }

    /// AutoPaper just set its picture through the portal: notes the portal's copy as AutoPaper left it.
    fn note_own_set(&self) {
        if !desktop::can_restore_wallpaper() {
            return;
        }
        let identity = desktop::portal_copy_identity().unwrap_or_default();
        let _ = self.prefs.set_string("own-wallpaper-set", &identity);
        self.update_restore_action();
    }

    /// Before AutoPaper puts its own wallpaper on the desktop, the person's is recorded: once while AutoPaper's stays
    /// (until it's put back), and again when the person has chosen another one meanwhile (that one is theirs now).
    async fn remember_own_wallpaper(&self) {
        self.forget_own_wallpaper_if_replaced();
        if !self.own_wallpaper().is_empty() {
            return;
        }
        let kept = desktop::data_dir().join("own-wallpaper").join("picture");
        let Some((saved, copy_from)) = desktop::own_wallpaper(&kept) else { return };
        if let Some(from) = copy_from {
            let target = kept.clone();
            let copied = blocking(move || {
                desktop::keep_own_picture(&from, &target).map_err(|error| AutoPaperError::Storage { detail: error.to_string() })
            })
            .await;
            if let Err(error) = copied {
                tracing::warn!(%error, "couldn't keep a copy of the wallpaper that was showing");
                return;
            }
        }
        let record: std::collections::HashMap<String, String> = saved.into_iter().collect();
        if let Err(error) = self.prefs.set_value("own-wallpaper", &record.to_variant()) {
            tracing::warn!(%error, "couldn't record the wallpaper that was showing");
        }
        self.update_restore_action();
    }

    /// Puts the person's own wallpaper back (Quit, Restore my wallpaper; Pause keeps AutoPaper's showing, docs/app-spec.md
    /// 3a). False when there's none recorded, or
    /// when the person has picked another one since AutoPaper's went up (theirs stays; nothing is written).
    pub fn restore_own_wallpaper(&self) -> bool {
        if self.forget_own_wallpaper_if_replaced() {
            return false;
        }
        let saved: Vec<(String, String)> = self.own_wallpaper().into_iter().collect();
        if saved.is_empty() || !desktop::put_back_own_wallpaper(&saved) {
            return false;
        }
        tracing::info!("put the person's own wallpaper back");
        self.clear_own_wallpaper();
        true
    }

    /// Hears someone put up another wallpaper: the person in GNOME Settings (it writes the picture keys), or another
    /// app through the Wallpaper portal (it rewrites the portal's copy). The record of the one before goes at once, so
    /// Restore my wallpaper turns off. A change made while AutoPaper wasn't running is noticed now.
    fn watch_background(self: &Rc<Self>) {
        let Some(settings) = desktop::gnome_background() else { return };
        let check = {
            let weak = Rc::downgrade(self);
            move || {
                if let Some(app) = weak.upgrade()
                    && !app.setting_wallpaper.get()
                {
                    app.forget_own_wallpaper_if_replaced();
                    app.update_restore_action();
                }
            }
        };
        let on_key = check.clone();
        settings.connect_changed(None, move |_, key| {
            if key.starts_with("picture-uri") {
                on_key();
            }
        });
        // GSettings reports changes only for keys that have been read.
        let _ = (settings.string("picture-uri"), settings.string("picture-uri-dark"));
        let monitor = gio::File::for_path(desktop::portal_copy())
            .monitor_file(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
            .inspect_err(|error| tracing::warn!(%error, "can't watch the Wallpaper portal's picture"))
            .ok();
        if let Some(monitor) = &monitor {
            monitor.connect_changed(move |_, _, _, event| {
                if event != gio::FileMonitorEvent::Changed {
                    check();
                }
            });
        }
        self.background.replace(Some((settings, monitor)));
        self.forget_own_wallpaper_if_replaced();
    }

    /// Restore my wallpaper is offered while AutoPaper's wallpaper is showing and the person's can be put back. The Now
    /// view's line about the person's own wallpaper follows it (`Event::Status`).
    fn update_restore_action(&self) {
        if let Some(action) = self.gtk.lookup_action("restore-wallpaper").and_downcast::<gio::SimpleAction>() {
            action.set_enabled(self.can_restore_own_wallpaper());
        }
        if self.is_ready() {
            self.emit(Event::Status);
        }
        #[cfg(feature = "tray")]
        if let Some(tray) = self.tray.borrow().as_ref() {
            tray.refresh(self);
        }
    }

    /// The person's own wallpaper is on the desktop instead of AutoPaper's current one (they chose Restore my wallpaper,
    /// or another picture, since): the Now view says the next new wallpaper covers it. Only where it can be told.
    pub fn own_wallpaper_showing(&self) -> bool {
        desktop::can_restore_wallpaper() && self.state.borrow().current.is_some() && !self.ours_on_desktop()
    }

    /// Whether Restore my wallpaper can do something now (the menu's and the tray's item).
    pub fn can_restore_own_wallpaper(&self) -> bool {
        desktop::can_restore_wallpaper() && !self.own_wallpaper().is_empty() && !self.replaced()
    }

    /// Displays changed (one added or removed, or a new resolution or scale): tell the engine the new size and
    /// re-render what's showing for it, once the changes settle.
    fn watch_displays(self: &Rc<Self>) {
        let Some(display) = gtk::gdk::Display::default() else { return };
        let monitors = display.monitors();
        let weak = Rc::downgrade(self);
        monitors.connect_items_changed(move |monitors, _, _, _| {
            if let Some(app) = weak.upgrade() {
                app.watch_monitor_geometry(monitors);
                app.displays_changed();
            }
        });
        self.watch_monitor_geometry(&monitors);
    }

    fn watch_monitor_geometry(self: &Rc<Self>, monitors: &gio::ListModel) {
        for (monitor, handlers) in self.monitor_handlers.take() {
            if let Some(monitor) = monitor.upgrade() {
                for handler in handlers {
                    monitor.disconnect(handler);
                }
            }
        }
        let mut watched = Vec::new();
        for monitor in (0..monitors.n_items()).filter_map(|i| monitors.item(i).and_downcast::<gtk::gdk::Monitor>()) {
            let changed = {
                let weak = Rc::downgrade(self);
                move |_: &gtk::gdk::Monitor| {
                    if let Some(app) = weak.upgrade() {
                        app.displays_changed();
                    }
                }
            };
            let handlers = vec![monitor.connect_geometry_notify(changed.clone()), monitor.connect_scale_notify(changed)];
            watched.push((monitor.downgrade(), handlers));
        }
        self.monitor_handlers.replace(watched);
    }

    fn displays_changed(self: &Rc<Self>) {
        if let Some(pending) = self.display_change.take() {
            pending.remove();
        }
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local_once(std::time::Duration::from_millis(800), move || {
            let Some(app) = weak.upgrade() else { return };
            app.display_change.replace(None);
            glib::spawn_future_local(async move {
                app.send_display_hint().await;
                let current = app.state.borrow().current.clone();
                // Only AutoPaper's own wallpaper is rendered again: the person's, put back, stays as it is.
                if let Some(current) = current.filter(|_| !app.busy.get() && app.ours_on_desktop()) {
                    tracing::info!("displays changed: rendering the wallpaper again");
                    app.set_busy(true, Some(ProgressStage::Rendering));
                    app.show(&current, false).await;
                    app.set_busy(false, None);
                }
            });
        });
        self.display_change.replace(Some(source));
    }

    async fn send_display_hint(&self) {
        let (Some(engine), Some(display)) = (self.engine(), desktop::largest_display()) else { return };
        if let Err(error) = blocking(move || engine.set_display_hint(display.width, display.height)).await {
            tracing::warn!(%error, "couldn't pass the display size");
        }
    }

    // ── Ratings, history, memory ────────────────────────────────────────────────────────────────

    /// Sets a rating. Disliking the wallpaper that's showing replaces it when the settings say so.
    pub fn rate(self: &Rc<Self>, id: String, rating: Rating) {
        let Some(engine) = self.engine() else {
            // Before the engine is open (a notification's button started AutoPaper): kept until it is.
            self.pending_ratings.borrow_mut().push((id, rating));
            return;
        };
        if self.notified_id.borrow().as_deref() == Some(id.as_str()) {
            self.gtk.withdraw_notification("new-wallpaper");
        }
        let app = self.clone();
        glib::spawn_future_local(async move {
            match blocking(move || engine.rate(id, rating)).await {
                Ok(true) => app.new_wallpaper(Trigger::DislikeReplace),
                Ok(false) => {}
                Err(error) => app.set_error(&error),
            }
            app.refresh_current().await;
            app.refresh_mood_stats().await;
            app.emit(Event::History);
            app.emit(Event::Taste);
        });
    }

    /// Like/Dislike as a toggle: choosing the rating it already has clears it.
    pub fn toggle_rating(self: &Rc<Self>, generation: &Generation, rating: Rating) {
        let target = if generation.rating == rating { Rating::Unrated } else { rating };
        self.rate(generation.id.clone(), target);
    }

    pub async fn delete(self: &Rc<Self>, id: String) -> Result<(), AutoPaperError> {
        let Some(engine) = self.engine() else { return Ok(()) };
        let result = blocking(move || engine.delete_generation(id)).await;
        self.refresh_current().await;
        self.refresh_mood_stats().await;
        self.emit(Event::History);
        result
    }

    pub async fn clear_history(self: &Rc<Self>, keep_memory: bool) -> Result<(), AutoPaperError> {
        let Some(engine) = self.engine() else { return Ok(()) };
        let result = blocking(move || engine.clear_history(keep_memory)).await;
        self.refresh_current().await;
        self.refresh_status().await;
        self.refresh_mood_stats().await;
        self.emit(Event::History);
        self.emit(Event::Taste);
        result
    }

    pub async fn reset_taste(self: &Rc<Self>) -> Result<(), AutoPaperError> {
        let Some(engine) = self.engine() else { return Ok(()) };
        let result = blocking(move || engine.reset_taste()).await;
        self.emit(Event::Taste);
        result
    }

    /// Keywords changed outside the Moods view (the welcome added some): the moods are read again.
    pub fn keywords_changed(self: &Rc<Self>) {
        let app = self.clone();
        glib::spawn_future_local(async move { app.refresh_moods().await });
    }

    /// A key field changed: saves it (empty deletes it) once typing pauses. A later change to the same account
    /// replaces this one. `dialog` is where a failure is toasted while it's still open.
    pub fn schedule_key_save(self: &Rc<Self>, account: String, value: String, dialog: &adw::PreferencesDialog) {
        if let Some(pending) = self.pending_keys.borrow_mut().remove(&account) {
            pending.source.remove();
        }
        let weak = Rc::downgrade(self);
        let due = account.clone();
        let source = glib::timeout_add_local_once(KEY_SAVE_DELAY, move || {
            let Some(app) = weak.upgrade() else { return };
            // This source has fired: take its entry without removing the source again.
            let pending = app.pending_keys.borrow_mut().remove(&due);
            if let Some(pending) = pending {
                app.spawn_key_save(due, pending.value, pending.dialog);
            }
        });
        self.pending_keys.borrow_mut().insert(account, PendingKey { value, source, dialog: dialog.downgrade() });
    }

    /// Saves every key still waiting for typing to pause (Preferences closed).
    pub fn flush_key_saves(self: &Rc<Self>) {
        let pending: Vec<(String, PendingKey)> = self.pending_keys.borrow_mut().drain().collect();
        for (account, key) in pending {
            key.source.remove();
            self.spawn_key_save(account, key.value, key.dialog);
        }
    }

    fn spawn_key_save(self: &Rc<Self>, account: String, value: String, dialog: glib::WeakRef<adw::PreferencesDialog>) {
        let app = self.clone();
        self.key_saves.set(self.key_saves.get() + 1);
        glib::spawn_future_local(async move {
            let saved = app.save_key(account, value).await;
            app.key_saves.set(app.key_saves.get().saturating_sub(1));
            if let Err(problem) = saved {
                tracing::warn!(problem, "couldn't save a key");
                let text = "Couldn't save the key to the keyring. Is it unlocked?";
                match dialog.upgrade() {
                    Some(dialog) => dialog.add_toast(crate::window::toast(text)),
                    None => app.set_notice(Some(Notice::plain(text, NoticeKind::KeyProblem))),
                }
            }
        });
    }

    /// Saves a key to the keyring (empty deletes it).
    pub async fn save_key(&self, account: String, value: String) -> Result<(), String> {
        let secrets = self.secrets.clone();
        let result = blocking(move || Ok(secrets.write(&account, &value))).await;
        let result = result.unwrap_or_else(|error| Err(error.to_string()));
        let key_problem = matches!(&self.state.borrow().notice, Some(Notice { kind: NoticeKind::KeyProblem, .. }));
        if result.is_ok() && key_problem {
            self.key_problem_gone();
            self.set_notice(None);
        }
        if result.is_ok() {
            self.emit(Event::Keys);
        }
        result
    }

    pub async fn read_key(&self, account: String) -> Result<Option<String>, String> {
        let secrets = self.secrets.clone();
        blocking(move || Ok(secrets.read(&account))).await.unwrap_or_else(|error| Err(error.to_string()))
    }

    // ── Schedule ────────────────────────────────────────────────────────────────────────────────

    /// Whether the next scheduled wallpaper should have started by `now`: from `next_start` (the due time less how
    /// long a wallpaper takes here, so it's ready on time), else `next_due`.
    fn starts_by(&self, now: i64) -> bool {
        let state = self.state.borrow();
        state.next_start.or(state.next_due).is_some_and(|start| start <= now)
    }

    /// Arms the timer for `next_start`, at most five minutes ahead so a suspended machine catches up soon. While a
    /// wallpaper is being made the timer waits at least a minute: the job's end re-reads the schedule anyway, and a
    /// due time passing meanwhile mustn't turn into a status refresh every second.
    fn reschedule(self: &Rc<Self>) {
        if let Some(source) = self.timer.take() {
            source.remove();
        }
        let (start, paused) = {
            let state = self.state.borrow();
            (state.next_start.or(state.next_due), state.settings.paused)
        };
        let Some(start) = start.filter(|_| !paused) else { return };
        let soonest = if self.busy.get() { 60 } else { 1 };
        let wait = (start - unix_now()).clamp(soonest, 300) as u32;
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_seconds_local_once(wait, move || {
            let Some(app) = weak.upgrade() else { return };
            // This source has fired; forget it so it's never removed twice.
            app.timer.replace(None);
            app.tick();
        });
        self.timer.replace(Some(source));
    }

    fn tick(self: &Rc<Self>) {
        if self.starts_by(unix_now()) && !self.busy.get() {
            self.check_schedule();
        } else {
            let app = self.clone();
            glib::spawn_future_local(async move { app.refresh_status().await });
        }
    }

    /// Resume and unlock: the timer may have slept through the due time.
    fn woke(self: &Rc<Self>, why: &str) {
        tracing::info!(why, "checking the schedule");
        let app = self.clone();
        glib::spawn_future_local(async move {
            app.refresh_status().await;
            app.check_schedule();
        });
    }

    fn watch_wake(self: &Rc<Self>) {
        // One clock watcher on every desktop: budget resets also matter when logind is available and scheduling is
        // paused or manual. It doubles as resume detection where the system bus is unavailable.
        self.watch_clock_jumps();
        let app = self.clone();
        glib::spawn_future_local(async move {
            match gio::bus_get_future(gio::BusType::System).await {
                Ok(bus) => {
                    let weak = Rc::downgrade(&app);
                    let subscription = bus.subscribe_to_signal(
                        Some("org.freedesktop.login1"),
                        Some("org.freedesktop.login1.Manager"),
                        Some("PrepareForSleep"),
                        Some("/org/freedesktop/login1"),
                        None,
                        gio::DBusSignalFlags::NONE,
                        move |signal| {
                            let going_to_sleep = signal.parameters.get::<(bool,)>().map(|(sleep,)| sleep);
                            if going_to_sleep == Some(false)
                                && let Some(app) = weak.upgrade() {
                                    app.woke("resume");
                                }
                        },
                    );
                    app.wake_subscriptions.borrow_mut().push(subscription);
                }
                // Expected inside Flatpak (no system bus there, and logind would need a permission): a resume is told
                // instead by the wall clock jumping ahead of the monotonic one, which stops while suspended.
                Err(error) => {
                    tracing::info!(%error, "no system bus: resume is noticed by the clocks and unlock");
                }
            }
            match gio::bus_get_future(gio::BusType::Session).await {
                // Inside Flatpak the shell's ScreenSaver signal never reaches the sandbox: the Inhibit portal says
                // when the screen saver goes off instead.
                Ok(bus) if desktop::is_sandboxed() => app.watch_unlock_portal(&bus).await,
                Ok(bus) => {
                    let weak = Rc::downgrade(&app);
                    let subscription = bus.subscribe_to_signal(
                        None,
                        Some("org.gnome.ScreenSaver"),
                        Some("ActiveChanged"),
                        Some("/org/gnome/ScreenSaver"),
                        None,
                        gio::DBusSignalFlags::NONE,
                        move |signal| {
                            let active = signal.parameters.get::<(bool,)>().map(|(active,)| active);
                            if active == Some(false)
                                && let Some(app) = weak.upgrade() {
                                    app.woke("unlock");
                                }
                        },
                    );
                    app.wake_subscriptions.borrow_mut().push(subscription);
                }
                Err(error) => tracing::warn!(%error, "no session bus: unlock won't be noticed"),
            }
        });
    }

    /// Resume without logind: every 30 s, whether the wall clock moved on further than the monotonic clock, which
    /// doesn't count time asleep (`slept_between`). Also refreshes the UTC monthly budget boundary, even when
    /// scheduling is manual or paused. Needs no permission.
    fn watch_clock_jumps(self: &Rc<Self>) {
        let last = Cell::new((glib::real_time(), glib::monotonic_time()));
        let weak = Rc::downgrade(self);
        glib::timeout_add_seconds_local(CLOCK_TICK_SECS, move || {
            let Some(app) = weak.upgrade() else { return glib::ControlFlow::Break };
            let now = (glib::real_time(), glib::monotonic_time());
            if slept_between(last.replace(now), now) {
                app.woke("resume");
            } else if app.state.borrow().budget.as_ref().is_some_and(|budget| {
                budget.month != autopaper_core::schedule::month_key(now.0 / 1_000_000)
            }) {
                app.woke("budget month changed");
            }
            glib::ControlFlow::Continue
        });
    }

    /// Unlock (and resume, which locks the screen) inside the sandbox: the Inhibit portal's monitor session reports
    /// `screensaver-active`, with no bus permission needed. When it turns off, the schedule is checked. A request to end
    /// the session (logout) is acknowledged at once, as the portal asks.
    async fn watch_unlock_portal(self: &Rc<Self>, bus: &gio::DBusConnection) {
        const PORTAL: &str = "org.freedesktop.portal.Desktop";
        const PATH: &str = "/org/freedesktop/portal/desktop";
        const INHIBIT: &str = "org.freedesktop.portal.Inhibit";
        let weak = Rc::downgrade(self);
        let screensaver = Rc::new(Cell::new(false));
        let responder = bus.clone();
        let subscription = bus.subscribe_to_signal(
            Some(PORTAL),
            Some(INHIBIT),
            Some("StateChanged"),
            Some(PATH),
            None,
            gio::DBusSignalFlags::NONE,
            move |signal| {
                let parameters = signal.parameters;
                if parameters.n_children() < 2 {
                    return;
                }
                let session = parameters.child_value(0);
                let state = glib::VariantDict::new(Some(&parameters.child_value(1)));
                let active = state.lookup::<bool>("screensaver-active").ok().flatten().unwrap_or(false);
                if state.lookup::<u32>("session-state").ok().flatten() == Some(2) {
                    // QueryEnd: say it's been heard (nothing to save), so logging out isn't held up.
                    responder.call(
                        Some(PORTAL),
                        PATH,
                        INHIBIT,
                        "QueryEndResponse",
                        Some(&glib::Variant::tuple_from_iter([session])),
                        None,
                        gio::DBusCallFlags::NONE,
                        -1,
                        gio::Cancellable::NONE,
                        |_| {},
                    );
                }
                if screensaver.replace(active) && !active
                    && let Some(app) = weak.upgrade()
                {
                    app.woke("unlock");
                }
            },
        );
        self.wake_subscriptions.borrow_mut().push(subscription);
        let options = glib::VariantDict::new(None);
        options.insert_value("handle_token", &"autopaper_monitor".to_variant());
        options.insert_value("session_handle_token", &"autopaper_monitor".to_variant());
        let arguments = glib::Variant::tuple_from_iter([String::new().to_variant(), options.end()]);
        let created = bus
            .call_future(Some(PORTAL), PATH, INHIBIT, "CreateMonitor", Some(&arguments), None, gio::DBusCallFlags::NONE, -1)
            .await;
        if let Err(error) = created {
            tracing::warn!(%error, "no Inhibit portal monitor: unlock won't be noticed (the timer still checks)");
        }
    }

    // ── Background, login, notifications ────────────────────────────────────────────────────────

    /// Asks the Background portal to let AutoPaper run with no window, and to start it at login or not.
    pub async fn apply_open_at_login(self: &Rc<Self>) {
        let auto_start = self.prefs.boolean("open-at-login");
        match desktop::request_background(auto_start, self.window_widget()).await {
            Ok(grant) => {
                tracing::info!(?grant, "Background portal");
                if auto_start && !grant.auto_start {
                    tracing::warn!("the Background portal didn't allow starting at login");
                }
                let denied = !grant.run_in_background;
                if self.state.borrow().background_denied != denied {
                    self.state.borrow_mut().background_denied = denied;
                    self.emit(Event::Status);
                }
            }
            Err(error) => tracing::warn!(%error, "the Background portal didn't answer"),
        }
    }

    fn window_is_active(&self) -> bool {
        self.window.borrow().as_ref().is_some_and(|window| window.widget().is_active())
    }

    /// "New wallpaper: <title>" with Like, Dislike and Show — unless the window is in front (the person is
    /// looking at it) or they turned notifications off.
    fn notify_new(&self, generation: &Generation) {
        if !self.prefs.boolean("notify-new-wallpapers") || self.window_is_active() {
            return;
        }
        let notification = gio::Notification::new(&format!("New wallpaper: {}", generation.concept.title));
        notification.set_body(Some(&generation.concept.summary));
        let id = generation.id.to_variant();
        notification.add_button_with_target_value("Like", "app.like-wallpaper", Some(&id));
        notification.add_button_with_target_value("Dislike", "app.dislike-wallpaper", Some(&id));
        notification.add_button("Show", "app.show-window");
        notification.set_default_action("app.show-window");
        if let Some(bytes) = generation.thumb_path.as_ref().and_then(|path| std::fs::read(path).ok()) {
            notification.set_icon(&gio::BytesIcon::new(&glib::Bytes::from_owned(bytes)));
        }
        self.gtk.send_notification(Some("new-wallpaper"), &notification);
        self.notified_id.replace(Some(generation.id.clone()));
    }

    /// Once a month: the budget ran out.
    fn notify_budget_spent(&self) {
        let month = self.state.borrow().spend.as_ref().map(|spend| spend.month.clone()).unwrap_or_default();
        if self.prefs.string("budget-notified-month") == month.as_str() {
            return;
        }
        let notification = gio::Notification::new("New wallpapers paused by monthly budget");
        let message = self.state.borrow().budget.as_ref().map(|budget| budget.message.clone())
            .unwrap_or_else(|| "The next wallpaper would exceed this month's budget. Your current wallpaper is kept. Raise the budget in Preferences, or wait for the monthly reset.".into());
        notification.set_body(Some(&message));
        notification.add_button_with_target_value("Change budget", "app.preferences-page", Some(&"budget".to_variant()));
        notification.set_default_action("app.show-window");
        self.gtk.send_notification(Some("budget"), &notification);
        let _ = self.prefs.set_string("budget-notified-month", &month);
    }

    /// Once, until a wallpaper is made again: a key is missing or was rejected. Its button is the link to the key's
    /// field ("Add your OpenAI key"), as on the Now view.
    fn notify_key_problem(&self, provider: ProviderKind, refused: bool) {
        if self.prefs.boolean("key-problem-notified") {
            return;
        }
        let name = strings::provider_in_sentence(provider);
        let notification = gio::Notification::new(&format!("AutoPaper can't use {name}"));
        notification.set_body(Some(&if refused {
            format!("{} didn't accept your key.", strings::capitalized(&name))
        } else {
            format!("{} needs your key to make new wallpapers.", strings::capitalized(&name))
        }));
        match key_account(&self.state.borrow().settings, provider) {
            Some(account) => {
                let target = account.to_variant();
                notification.add_button_with_target_value(&strings::key_link(provider, refused), "app.fix-key", Some(&target));
                notification.set_default_action_and_target_value("app.fix-key", Some(&target));
            }
            None => notification.set_default_action("app.show-window"),
        }
        self.gtk.send_notification(Some("keys"), &notification);
        let _ = self.prefs.set_boolean("key-problem-notified", true);
    }

    // ── Window, tray, actions ───────────────────────────────────────────────────────────────────

    pub fn window_widget(&self) -> Option<gtk::Window> {
        self.window.borrow().as_ref().map(|window| window.widget().clone().upcast())
    }

    pub fn main_window(&self) -> Option<Rc<MainWindow>> {
        self.window.borrow().clone()
    }

    /// Shows the main window, building a new one when there's none. A window that was closed is never shown
    /// again: GTK destroyed it (it has no accessible tree or app identity left), so it's let go as it closes.
    pub fn present_window(self: &Rc<Self>) {
        let existing = self.window.borrow().clone().filter(|window| {
            let widget: &gtk::Widget = window.widget().upcast_ref();
            gtk::Window::list_toplevels().iter().any(|toplevel| toplevel == widget)
        });
        let window = match existing {
            Some(window) => window,
            None => {
                let window = MainWindow::new(self);
                self.window.replace(Some(window.clone()));
                let app = Rc::downgrade(self);
                let closing = Rc::downgrade(&window);
                window.widget().connect_close_request(move |_| {
                    // AutoPaper's reference would keep the destroyed window (and its views and thumbnails) alive;
                    // drop it once GTK has finished closing, unless a new window has taken its place by then.
                    let (app, closing) = (app.clone(), closing.clone());
                    glib::idle_add_local_once(move || {
                        let Some(app) = app.upgrade() else { return };
                        let same = matches!(
                            (app.window.borrow().as_ref(), closing.upgrade()),
                            (Some(held), Some(closed)) if Rc::ptr_eq(held, &closed)
                        );
                        if same {
                            app.window.replace(None);
                        }
                    });
                    glib::Propagation::Proceed
                });
                window
            }
        };
        window.widget().present();
    }

    /// Goes to where a problem is fixed: Preferences on that page with the field focused, or a mood (its keywords) for
    /// a keyword the writing model wouldn't follow. The window opens first.
    pub fn open_fix(self: &Rc<Self>, fix: Fix) {
        self.present_window();
        let Some(window) = self.main_window() else { return };
        match &fix {
            Fix::Mood(id) => window.show_mood(id),
            Fix::Key(_) | Fix::Provider(..) | Fix::Budget => window.open_fix(&fix),
        }
    }

    /// Opens the window on Moods, the summary of every mood (the tray's "Edit moods…").
    pub fn edit_moods(self: &Rc<Self>) {
        self.present_window();
        if let Some(window) = self.main_window() {
            window.show_moods();
        }
    }

    /// Opens Preferences on `page` (or the page shown last), opening the window first.
    pub fn open_preferences(self: &Rc<Self>, page: Option<&str>) {
        if let Some(page) = page {
            let _ = self.prefs.set_string("preferences-page", page);
        }
        self.present_window();
        if let Some(window) = self.main_window() {
            // The page goes through too: Preferences may be open already, on another page.
            window.open_preferences(page);
        }
    }

    fn start_tray(self: &Rc<Self>) {
        #[cfg(feature = "tray")]
        if desktop::wants_tray() {
            let tray = crate::tray::Tray::start(self);
            self.tray.replace(Some(tray));
            if let Some(tray) = self.tray.borrow().as_ref() {
                tray.refresh(self);
            }
        }
    }

    /// Quit: the window closes at once; keys still waiting for typing to pause are saved first, off the main thread
    /// (a locked keyring asks to be unlocked, and the main loop keeps running meanwhile), and AutoPaper stops once
    /// they're saved, or after `QUIT_KEY_WAIT` if the keyring never answers.
    pub fn quit(self: &Rc<Self>) {
        #[cfg(feature = "tray")]
        if let Some(tray) = self.tray.take() {
            tray.shutdown();
        }
        // Quitting puts the person's own wallpaper back (docs/app-spec.md 3a).
        self.restore_own_wallpaper();
        // Taken before the window closes: closing Preferences would otherwise hand them to saves nobody waits for.
        let pending: Vec<(String, PendingKey)> = self.pending_keys.borrow_mut().drain().collect();
        if let Some(window) = self.main_window() {
            let widget = window.widget().clone();
            // With a dialog open (Preferences), libadwaita closes the dialog instead of the window (its size is saved
            // all the same): the window then goes too.
            widget.close();
            if widget.is_visible() {
                widget.destroy();
            }
        }
        let app = self.clone();
        glib::spawn_future_local(async move {
            let deadline = glib::monotonic_time() + QUIT_KEY_WAIT.as_micros() as i64;
            let left = || std::time::Duration::from_micros((deadline - glib::monotonic_time()).max(0) as u64);
            for (account, key) in pending {
                key.source.remove();
                let secrets = app.secrets.clone();
                let save = blocking(move || Ok(secrets.write(&account, &key.value)));
                match glib::future_with_timeout(left(), save).await {
                    Ok(Ok(Ok(()))) => {}
                    Ok(Ok(Err(problem))) => tracing::warn!(problem, "couldn't save a key while quitting"),
                    Ok(Err(error)) => tracing::warn!(%error, "couldn't save a key while quitting"),
                    Err(_) => tracing::warn!("the keyring didn't answer; a key typed just before Quit wasn't saved"),
                }
            }
            // Saves already under way (typing paused just before Quit) finish too.
            while app.key_saves.get() > 0 && !left().is_zero() {
                glib::timeout_future(std::time::Duration::from_millis(100)).await;
            }
            app.hold.replace(None);
            app.gtk.quit();
        });
    }

    fn update_busy_actions(&self) {
        self.apply_busy_actions(false);
    }

    /// Enables New wallpaper now while idle and Cancel while busy. With `enable_only`, only actions that become
    /// enabled change (see `set_busy`).
    fn apply_busy_actions(&self, enable_only: bool) {
        let busy = self.busy.get();
        let ready = self.is_ready();
        let set = |name: &str, enabled: bool| {
            if (enabled || !enable_only)
                && let Some(action) = self.gtk.lookup_action(name).and_downcast::<gio::SimpleAction>()
            {
                action.set_enabled(enabled);
            }
        };
        set("new-wallpaper", ready && !busy);
        set("cancel", busy);
    }

    fn install_actions(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let simple = |name: &str, run: AppAction| {
            let action = gio::SimpleAction::new(name, None);
            let weak = weak.clone();
            action.connect_activate(move |_, _| {
                if let Some(app) = weak.upgrade() {
                    run(&app);
                }
            });
            self.gtk.add_action(&action);
        };
        simple("show-window", Box::new(|app| app.present_window()));
        simple("preferences", Box::new(|app| app.open_preferences(None)));
        simple("new-wallpaper", Box::new(|app| app.new_wallpaper(Trigger::Manual)));
        simple("cancel", Box::new(|app| app.cancel()));
        simple("about", Box::new(crate::window::show_about));
        simple("help", Box::new(crate::window::show_help));
        simple("shortcuts", Box::new(crate::window::show_shortcuts));
        simple("quit", Box::new(|app| app.quit()));
        simple("like-current", Box::new(|app| app.rate_current(Rating::Liked)));
        simple("dislike-current", Box::new(|app| app.rate_current(Rating::Disliked)));
        simple("edit-moods", Box::new(|app| app.edit_moods()));
        simple(
            "restore-wallpaper",
            Box::new(|app| {
                if app.restore_own_wallpaper() {
                    app.announce("Your own wallpaper is back.");
                }
            }),
        );

        let with_id = |name: &str, run: AppActionWithId| {
            let action = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
            let weak = weak.clone();
            action.connect_activate(move |_, parameter| {
                let id = parameter.and_then(|value| value.get::<String>());
                if let (Some(app), Some(id)) = (weak.upgrade(), id) {
                    run(&app, id);
                }
            });
            self.gtk.add_action(&action);
        };
        // From the notification's buttons: set that rating (not a toggle: the button says what it does).
        with_id("like-wallpaper", Box::new(|app, id| app.rate(id, Rating::Liked)));
        with_id("dislike-wallpaper", Box::new(|app, id| app.rate(id, Rating::Disliked)));
        with_id("preferences-page", Box::new(|app, page| app.open_preferences(Some(&page))));
        // A key problem's link (the notification's button): Preferences → Keys with that key's field focused.
        with_id("fix-key", Box::new(|app, account| app.open_fix(Fix::Key(account))));

        // The mood in use (the header's Mood menu): its state is the active mood's id, and choosing one uses it.
        let mood = gio::SimpleAction::new_stateful("mood", Some(glib::VariantTy::STRING), &String::new().to_variant());
        let weak_mood = weak.clone();
        mood.connect_activate(move |_, parameter| {
            if let (Some(app), Some(id)) = (weak_mood.upgrade(), parameter.and_then(|value| value.get::<String>())) {
                app.use_mood(id);
            }
        });
        mood.connect_change_state(|action, value| {
            if let Some(value) = value {
                action.set_state(value);
            }
        });
        self.gtk.add_action(&mood);

        let pause = gio::SimpleAction::new_stateful("pause", None, &false.to_variant());
        let weak_pause = weak.clone();
        pause.connect_activate(move |action, _| {
            let paused = !action.state().and_then(|state| state.get::<bool>()).unwrap_or(false);
            if let Some(app) = weak_pause.upgrade() {
                app.set_paused(paused);
            }
        });
        pause.connect_change_state(|action, value| {
            if let Some(value) = value {
                action.set_state(value);
            }
        });
        self.gtk.add_action(&pause);

        // Ctrl+R and F5 (GNOME's Refresh) are the page's reload button: New wallpaper now, or on another mood's page,
        // that mood and then a new wallpaper (window.rs); Esc stops one being made. Ctrl+N is New mood.
        self.gtk.set_accels_for_action("win.make", &["<Control>r", "F5"]);
        self.gtk.set_accels_for_action("win.new-mood", &["<Control>n"]);
        self.gtk.set_accels_for_action("app.preferences", &["<Control>comma"]);
        self.gtk.set_accels_for_action("app.shortcuts", &["<Control>question"]);
        self.gtk.set_accels_for_action("app.help", &["F1"]);
        self.gtk.set_accels_for_action("app.quit", &["<Control>q"]);
        self.update_restore_action();
        self.gtk.set_accels_for_action("window.close", &["<Control>w"]);
        self.gtk.set_accels_for_action("win.view::now", &["<Alt>1"]);
        self.gtk.set_accels_for_action("win.view::moods", &["<Alt>2"]);
        self.gtk.set_accels_for_action("win.view::history", &["<Alt>3"]);
        self.gtk.set_accels_for_action("win.view::console", &["<Alt>4"]);
        self.update_busy_actions();
    }

    pub fn set_paused(self: &Rc<Self>, paused: bool) {
        let app = self.clone();
        glib::spawn_future_local(async move {
            if let Err(error) = app.change_settings(|settings| settings.paused = paused).await {
                app.set_error(&error);
            }
        });
    }

    /// Like / Dislike for the wallpaper showing (the tray), as toggles.
    pub fn rate_current(self: &Rc<Self>, rating: Rating) {
        let current = self.state.borrow().current.clone();
        if let Some(current) = current {
            self.toggle_rating(&current, rating);
        }
    }
}

enum Job {
    New(Trigger),
    Echo(String),
}

/// Engine progress, sent from the engine's thread to the main loop.
struct ProgressSender(async_channel::Sender<ProgressStage>);

impl ProgressObserver for ProgressSender {
    fn on_progress(&self, stage: ProgressStage) {
        let _ = self.0.try_send(stage);
    }
}

/// The painting's fraction and time left, sent from the engine's thread to the main loop.
struct DetailSender(async_channel::Sender<ProgressDetail>);

impl ProgressDetailObserver for DetailSender {
    fn on_progress_detail(&self, detail: ProgressDetail) {
        let _ = self.0.try_send(detail);
    }
}

/// The keyring account of `provider`'s key in these settings (an OpenAI-compatible server's depends on its address).
fn key_account(settings: &Settings, provider: ProviderKind) -> Option<String> {
    [&settings.text_provider, &settings.image_provider]
        .into_iter()
        .find(|selection| selection.kind == provider)
        .and_then(|selection| secret_account_for(selection.clone()))
        .or_else(|| provider.secret_account().map(str::to_string))
}

/// Whether the writer or the painter chosen in these settings uses the key kept under `account`.
fn uses_key(settings: &Settings, account: &str) -> bool {
    [&settings.text_provider, &settings.image_provider]
        .into_iter()
        .any(|selection| secret_account_for(selection.clone()).as_deref() == Some(account))
}

/// Where the problem behind `error` is fixed (docs/app-spec.md 6a): a setting in Preferences, or the mood whose keyword
/// the writing model wouldn't follow (while that mood is still there).
fn fix_for(settings: &Settings, moods: &[Mood], error: &AutoPaperError) -> Option<Fix> {
    let selections = [(ProviderJob::Concepts, &settings.text_provider), (ProviderJob::Images, &settings.image_provider)];
    let job_of = |kind: ProviderKind| selections.iter().find(|(_, selection)| selection.kind == kind).map(|(job, _)| *job);
    let uses_address = |kind: ProviderKind| {
        matches!(kind, ProviderKind::Ollama | ProviderKind::OpenAiCompatible | ProviderKind::ComfyUi)
    };
    match error {
        AutoPaperError::BudgetReached { .. } => Some(Fix::Budget),
        AutoPaperError::MissingKey { provider } | AutoPaperError::InvalidKey { provider } => {
            key_account(settings, *provider).map(Fix::Key)
        }
        AutoPaperError::ProviderUnavailable { provider, reason, .. }
            if *reason == ProviderUnavailableReason::NotRunning
                || (*reason == ProviderUnavailableReason::Other && provider.is_local()) =>
        {
            job_of(*provider).map(|job| Fix::Provider(job, ProviderField::Address))
        }
        AutoPaperError::Unsupported { provider, .. } => job_of(*provider).map(|job| Fix::Provider(job, ProviderField::Provider)),
        AutoPaperError::KeywordNotFollowed { mood_id, .. } => {
            moods.iter().any(|mood| mood.id == *mood_id).then(|| Fix::Mood(mood_id.clone()))
        }
        // The painting settings: the workflow when it's the person's own, else the model chosen.
        AutoPaperError::PaintingFailed { provider, .. } => job_of(*provider).map(|job| {
            let own = settings.comfyui_workflow.as_deref().is_some_and(|text| !text.trim().is_empty());
            let field = if *provider == ProviderKind::ComfyUi && own { ProviderField::Workflow } else { ProviderField::Model };
            Fix::Provider(job, field)
        }),
        AutoPaperError::InvalidInput { reason, .. } => match reason {
            InvalidInputReason::AddressMissing | InvalidInputReason::AddressInvalid | InvalidInputReason::AddressNotAllowed => {
                // The group whose address is the problem: a missing one has none; a refused one has one typed.
                let missing = *reason == InvalidInputReason::AddressMissing;
                selections
                    .iter()
                    .filter(|(_, selection)| uses_address(selection.kind))
                    .find(|(_, selection)| {
                        selection.base_url.as_deref().map(str::trim).unwrap_or("").is_empty() == missing
                    })
                    .or_else(|| selections.iter().find(|(_, selection)| uses_address(selection.kind)))
                    .map(|(job, _)| Fix::Provider(*job, ProviderField::Address))
            }
            InvalidInputReason::NoModels => selections
                .iter()
                .find(|(_, selection)| matches!(selection.kind, ProviderKind::Ollama | ProviderKind::OpenAiCompatible))
                .map(|(job, _)| Fix::Provider(*job, ProviderField::Model)),
            InvalidInputReason::WorkflowNeedsPrompt
            | InvalidInputReason::WorkflowNotApiFormat
            | InvalidInputReason::WorkflowInvalid => Some(Fix::Provider(ProviderJob::Images, ProviderField::Workflow)),
            _ => None,
        },
        _ => None,
    }
}

/// How often the clocks are compared to notice a resume without logind.
const CLOCK_TICK_SECS: u32 = 30;

/// Whether the computer slept between two (wall clock, monotonic clock) readings in µs: the wall clock moved on more
/// than a minute further than the monotonic one, which stops while suspended. (A wall clock set forward by a minute or
/// more looks the same; it only means the schedule is checked once more.)
fn slept_between(before: (i64, i64), now: (i64, i64)) -> bool {
    let real = now.0 - before.0;
    let monotonic = now.1 - before.1;
    real - monotonic > 60_000_000
}

fn unix_now() -> i64 {
    glib::real_time() / 1_000_000
}

/// The app's GSettings. A build run from `target/` uses the schema build.rs compiled; an installed one the
/// system's.
fn load_prefs() -> Result<gio::Settings, String> {
    let default = gio::SettingsSchemaSource::default();
    let built = Path::new(concat!(env!("OUT_DIR"), "/schemas"));
    let schema = if built.join("gschemas.compiled").is_file() {
        gio::SettingsSchemaSource::from_directory(built, default.as_ref(), false)
            .ok()
            .and_then(|source| source.lookup(APP_ID, false))
    } else {
        None
    }
    .or_else(|| default.as_ref().and_then(|source| source.lookup(APP_ID, true)));
    match schema {
        Some(schema) => Ok(gio::Settings::new_full(&schema, None::<&gio::SettingsBackend>, None)),
        None => Err(format!("the GSettings schema {APP_ID} isn't installed (scripts/linux-install.sh installs it)")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use autopaper_core::ProviderSelection;

    #[test]
    fn budget_blocks_link_to_the_monthly_limit() {
        assert_eq!(
            fix_for(&Settings::default(), &[], &AutoPaperError::BudgetReached { budget_cents: 500 }),
            Some(Fix::Budget)
        );
    }

    fn comfy_painting(workflow: Option<&str>) -> Settings {
        Settings {
            image_provider: ProviderSelection { kind: ProviderKind::ComfyUi, model: String::new(), base_url: None },
            comfyui_workflow: workflow.map(str::to_string),
            ..Settings::default()
        }
    }

    #[test]
    fn a_painting_failure_links_to_the_painting_setting_that_fixes_it() {
        let failed = AutoPaperError::PaintingFailed {
            provider: ProviderKind::ComfyUi,
            model: "Qwen-Image 2.1".into(),
            detail: "english detail".into(),
        };
        // AutoPaper's workflow: the model chosen is the thing to change.
        assert_eq!(
            fix_for(&comfy_painting(None), &[], &failed),
            Some(Fix::Provider(ProviderJob::Images, ProviderField::Model))
        );
        // A blank workflow counts as none (the core's rule).
        assert_eq!(
            fix_for(&comfy_painting(Some("  ")), &[], &failed),
            Some(Fix::Provider(ProviderJob::Images, ProviderField::Model))
        );
        // The person's own workflow: the workflow is.
        assert_eq!(
            fix_for(&comfy_painting(Some("{\"1\": {}}")), &[], &failed),
            Some(Fix::Provider(ProviderJob::Images, ProviderField::Workflow))
        );
        // A provider that isn't the painter in these settings has nothing to link to.
        assert_eq!(fix_for(&Settings::default(), &[], &failed), None);
    }

    #[test]
    fn key_problems_link_to_the_key_and_say_what_to_do() {
        let settings = Settings {
            text_provider: ProviderSelection { kind: ProviderKind::OpenAi, model: String::new(), base_url: None },
            ..Settings::default()
        };
        let missing = AutoPaperError::MissingKey { provider: ProviderKind::OpenAi };
        assert_eq!(fix_for(&settings, &[], &missing), Some(Fix::Key("openai.api_key".into())));
    }

    #[test]
    fn a_resume_is_the_wall_clock_running_ahead_of_the_monotonic_one() {
        let second = 1_000_000;
        // Awake: both clocks move on together (a little jitter is nothing).
        assert!(!slept_between((0, 0), (30 * second, 30 * second)));
        assert!(!slept_between((0, 0), (31 * second, 30 * second)));
        // Asleep for an hour between two ticks 30 s apart on the monotonic clock.
        assert!(slept_between((0, 0), (3630 * second, 30 * second)));
        // A wall clock set back never counts as sleep.
        assert!(!slept_between((100 * second, 0), (0, 30 * second)));
    }

    #[test]
    fn a_keyword_not_followed_links_to_its_mood_while_it_exists() {
        let mood = Mood {
            id: "night".into(),
            name: "Night city".into(),
            position: 0,
            surprise: 0.35,
            created_at: 0,
            keywords: Vec::new(),
            active: false,
        };
        let error = AutoPaperError::KeywordNotFollowed {
            keyword: "lighthouse".into(),
            weight: autopaper_core::KeywordWeight::Must,
            mood_id: "night".into(),
        };
        assert_eq!(fix_for(&Settings::default(), std::slice::from_ref(&mood), &error), Some(Fix::Mood("night".into())));
        // A mood deleted since has nowhere to go: the line shows without a link.
        assert_eq!(fix_for(&Settings::default(), &[], &error), None);
    }
}
