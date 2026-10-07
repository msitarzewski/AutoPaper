//! The tray on desktops other than GNOME (KDE Plasma and others with a StatusNotifierItem host): the same menu
//! as the macOS menu bar and the Windows notification area — the current wallpaper's title (and echo note), Like
//! and Dislike (checked for the current rating; choosing again clears it), Mood ▸ (the moods as radio items, the
//! one in use checked, then Edit moods…), New wallpaper now, the stage and Cancel while one is being made,
//! Pause/Resume new wallpapers, Show AutoPaper, Preferences…, Quit AutoPaper.
//! GNOME has no tray (its HIG): there AutoPaper is a background app with notifications.
//!
//! ksni runs on the tokio runtime; menu clicks come back to the GTK main loop over a channel.

use std::cell::RefCell;
use std::rc::Rc;

use autopaper_core::{Rating, Trigger};
use gtk::glib;
use ksni::menu::{CheckmarkItem, MenuItem, RadioGroup, RadioItem, StandardItem, SubMenu};
use ksni::{Category, Status, ToolTip, TrayMethods};

use crate::app::App;
use crate::desktop::{self, APP_ID};
use crate::runtime::runtime;
use crate::strings;

#[derive(Debug, Clone)]
enum Command {
    Show,
    UseMood(String),
    EditMoods,
    RestoreWallpaper,
    Preferences,
    Quit,
    Like,
    Dislike,
    New,
    Cancel,
    TogglePause,
}

/// What the menu shows; copied from the app's state on every change.
#[derive(Debug, Clone, Default)]
struct Snapshot {
    title: Option<String>,
    echo_note: Option<String>,
    rating: Option<Rating>,
    stage: Option<&'static str>,
    busy: bool,
    paused: bool,
    ready: bool,
    /// Every mood (id, name) in the person's order, and which is in use.
    moods: Vec<(String, String)>,
    active_mood: usize,
    /// The person's own wallpaper can be put back (`None`: not on this desktop).
    can_restore: Option<bool>,
}

struct Model {
    snapshot: Snapshot,
    commands: async_channel::Sender<Command>,
}

impl Model {
    fn send(&self, command: Command) {
        let _ = self.commands.try_send(command);
    }
}

fn item(label: &str, enabled: bool, command: Command) -> MenuItem<Model> {
    StandardItem {
        label: label.replace('_', "__"),
        enabled,
        activate: Box::new(move |model: &mut Model| model.send(command.clone())),
        ..Default::default()
    }
    .into()
}

fn text(label: &str) -> MenuItem<Model> {
    StandardItem { label: label.replace('_', "__"), enabled: false, ..Default::default() }.into()
}

impl ksni::Tray for Model {
    const MENU_ON_ACTIVATE: bool = false;

    fn id(&self) -> String {
        APP_ID.into()
    }

    fn title(&self) -> String {
        "AutoPaper".into()
    }

    fn icon_name(&self) -> String {
        format!("{APP_ID}-symbolic")
    }

    fn category(&self) -> Category {
        Category::ApplicationStatus
    }

    fn status(&self) -> Status {
        Status::Active
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "AutoPaper".into(),
            description: self.snapshot.stage.map(str::to_string).or_else(|| self.snapshot.title.clone()).unwrap_or_default(),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(Command::Show);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let snapshot = &self.snapshot;
        let mut menu = Vec::new();
        match &snapshot.title {
            Some(title) => {
                menu.push(text(title));
                if let Some(note) = &snapshot.echo_note {
                    menu.push(text(note));
                }
                menu.push(MenuItem::Separator);
                for (label, rating, command) in
                    [("Like", Rating::Liked, Command::Like), ("Dislike", Rating::Disliked, Command::Dislike)]
                {
                    menu.push(
                        CheckmarkItem {
                            label: label.into(),
                            checked: snapshot.rating == Some(rating),
                            enabled: !snapshot.busy,
                            activate: Box::new(move |model: &mut Model| model.send(command.clone())),
                            ..Default::default()
                        }
                        .into(),
                    );
                }
            }
            None => menu.push(text("No wallpaper yet")),
        }
        if !snapshot.moods.is_empty() {
            // Mood ▸: choosing one uses it (nothing is made by itself); Edit moods… opens Moods.
            let mut submenu = vec![
                RadioGroup {
                    selected: snapshot.active_mood,
                    select: Box::new(|model: &mut Model, index| {
                        if let Some((id, _)) = model.snapshot.moods.get(index) {
                            let command = Command::UseMood(id.clone());
                            model.send(command);
                        }
                    }),
                    options: snapshot
                        .moods
                        .iter()
                        .map(|(_, name)| RadioItem { label: name.replace('_', "__"), ..Default::default() })
                        .collect(),
                }
                .into(),
            ];
            submenu.push(MenuItem::Separator);
            submenu.push(item("Edit moods…", true, Command::EditMoods));
            menu.push(MenuItem::Separator);
            menu.push(SubMenu { label: "Mood".into(), enabled: snapshot.ready, submenu, ..Default::default() }.into());
        }
        menu.push(MenuItem::Separator);
        if snapshot.busy {
            menu.push(text(snapshot.stage.unwrap_or(strings::MAKING)));
            menu.push(item("Cancel", true, Command::Cancel));
        } else {
            menu.push(item("New wallpaper now", snapshot.ready, Command::New));
        }
        menu.push(item(
            if snapshot.paused { "Resume new wallpapers" } else { "Pause new wallpapers" },
            snapshot.ready,
            Command::TogglePause,
        ));
        if let Some(can_restore) = snapshot.can_restore {
            menu.push(item("Restore my wallpaper", can_restore, Command::RestoreWallpaper));
        }
        menu.push(MenuItem::Separator);
        menu.push(item("Show AutoPaper", true, Command::Show));
        menu.push(item("Preferences…", snapshot.ready, Command::Preferences));
        menu.push(item("Quit AutoPaper", true, Command::Quit));
        menu
    }
}

pub struct Tray {
    handle: Rc<RefCell<Option<ksni::Handle<Model>>>>,
    /// The latest snapshot, applied as soon as the tray is up.
    pending: Rc<RefCell<Option<Snapshot>>>,
}

impl Tray {
    pub fn start(app: &Rc<App>) -> Self {
        let (commands, incoming) = async_channel::unbounded::<Command>();
        let handle: Rc<RefCell<Option<ksni::Handle<Model>>>> = Rc::new(RefCell::new(None));
        let pending: Rc<RefCell<Option<Snapshot>>> = Rc::new(RefCell::new(None));
        let model = Model { snapshot: Snapshot::default(), commands };
        let sandboxed = desktop::is_sandboxed();
        let started = runtime().spawn(async move { model.disable_dbus_name(sandboxed).spawn().await });
        {
            let handle = handle.clone();
            let pending = pending.clone();
            glib::spawn_future_local(async move {
                match started.await {
                    Ok(Ok(running)) => {
                        if let Some(snapshot) = pending.borrow_mut().take() {
                            let update = running.clone();
                            runtime().spawn(async move {
                                update.update(move |model| model.snapshot = snapshot).await;
                            });
                        }
                        handle.replace(Some(running));
                    }
                    Ok(Err(error)) => tracing::warn!(%error, "no tray host (StatusNotifierWatcher) to show the tray in"),
                    Err(error) => tracing::warn!(%error, "the tray didn't start"),
                }
            });
        }
        let weak = Rc::downgrade(app);
        glib::spawn_future_local(async move {
            while let Ok(command) = incoming.recv().await {
                let Some(app) = weak.upgrade() else { break };
                run(&app, command);
            }
        });
        Self { handle, pending }
    }

    /// Copies the app's state into the menu.
    pub fn refresh(&self, app: &App) {
        let state = app.state();
        let current = state.current.as_ref();
        let snapshot = Snapshot {
            title: current.map(|generation| generation.concept.title.clone()),
            echo_note: current.and_then(|generation| generation.echo_note.clone()).filter(|note| !note.is_empty()),
            rating: current.map(|generation| generation.rating),
            stage: state.stage.and_then(strings::stage_label),
            busy: app.is_busy(),
            paused: state.settings.paused,
            ready: app.is_ready(),
            moods: state.moods.iter().map(|mood| (mood.id.clone(), mood.name.clone())).collect(),
            active_mood: state.moods.iter().position(|mood| mood.active).unwrap_or(0),
            can_restore: desktop::can_restore_wallpaper().then(|| app.can_restore_own_wallpaper()),
        };
        drop(state);
        match self.handle.borrow().clone() {
            Some(handle) => {
                runtime().spawn(async move {
                    handle.update(move |model| model.snapshot = snapshot).await;
                });
            }
            None => {
                self.pending.replace(Some(snapshot));
            }
        }
    }

    pub fn shutdown(&self) {
        if let Some(handle) = self.handle.borrow_mut().take() {
            // Sends the request now; nothing needs to wait for the tray to go.
            drop(handle.shutdown());
        }
    }
}

fn run(app: &Rc<App>, command: Command) {
    match command {
        Command::Show => app.present_window(),
        Command::UseMood(id) => app.use_mood(id),
        Command::EditMoods => app.edit_moods(),
        Command::RestoreWallpaper => {
            app.restore_own_wallpaper();
        }
        Command::Preferences => app.open_preferences(None),
        Command::Quit => app.quit(),
        Command::Like => app.rate_current(Rating::Liked),
        Command::Dislike => app.rate_current(Rating::Disliked),
        Command::New => app.new_wallpaper(Trigger::Manual),
        Command::Cancel => app.cancel(),
        Command::TogglePause => {
            let paused = app.state().settings.paused;
            app.set_paused(!paused);
        }
    }
}
