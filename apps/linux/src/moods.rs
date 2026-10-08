//! Moods (docs/app-spec.md, "Moods"): each mood is a row in the window's sidebar, under Moods, in the person's order —
//! its name, and a checkmark on the current one (said as "Rainy beach, current mood"; its keywords are its
//! description). Right click, long press, Shift+F10 or Menu on a row opens Use, Duplicate, Rename…, Move up, Move down
//! and Delete… (Delete off for the last mood). Return or a double click uses a mood; Delete deletes it (after asking);
//! Alt+Up / Alt+Down and dragging reorder; Ctrl+N or the sidebar's + adds one. The window owns the list and its other
//! rows (Now, Moods, History); this page puts the moods' rows in it and keeps them in step.
//!
//! A mood's page edits it, current or not. Its header bar has the name in the title's place (click it, or press Enter
//! on it, to rename it in place; Enter keeps the new name, Esc puts the old one back), Use this mood or "Current mood",
//! and the reload button (`window::MakeOrStop`: on a mood that isn't current, it makes the mood current and then a new
//! wallpaper). Below: what it has made (stat tiles), its keywords, its Surprise, its latest wallpapers (a click shows
//! one on the desktop; History's menu on each), Duplicate mood and Delete mood….
//!
//! Switching never makes a wallpaper by itself: the next one, scheduled or asked for, uses the mood.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use adw::prelude::*;
use autopaper_core::{AutoPaperError, HistoryFilter, Mood};
use gtk::{gdk, gio, glib};

use crate::app::{App, Event};
use crate::keywords::KeywordEditor;
use crate::runtime::blocking;
use crate::strings;
use crate::summary::StatTile;
use crate::wallpapers::{self, Item};
use crate::window::MakeOrStop;

/// Wallpapers shown in a mood's "Made in this mood".
const RECENT: u32 = 6;
const RECENT_WIDTH: i32 = 112;
const RECENT_HEIGHT: i32 = 63;

type MoodAction = Box<dyn Fn(&Rc<MoodsPage>, String)>;

pub struct MoodsPage {
    app: Weak<App>,
    /// The window's sidebar; the moods' rows go from `first` on.
    list: gtk::ListBox,
    first: i32,
    rows: RefCell<Vec<Rc<MoodRow>>>,
    /// The moods' ids in the order last shown, to find a deleted mood's neighbour.
    previous: RefCell<Vec<String>>,
    detail: Rc<MoodDetail>,
}

struct MoodRow {
    mood: RefCell<Mood>,
    row: gtk::ListBoxRow,
    name: gtk::Label,
    check: gtk::Image,
    actions: gio::SimpleActionGroup,
}

impl MoodsPage {
    pub fn new(app: &Rc<App>, list: &gtk::ListBox, first: i32, toasts: &adw::ToastOverlay) -> Rc<Self> {
        let detail = MoodDetail::new(app, toasts);
        let this = Rc::new(Self {
            app: Rc::downgrade(app),
            list: list.clone(),
            first,
            rows: RefCell::new(Vec::new()),
            previous: RefCell::new(Vec::new()),
            detail,
        });
        this.detail.page.replace(Rc::downgrade(&this));
        let weak = Rc::downgrade(&this);
        app.subscribe(move |event| {
            let Some(this) = weak.upgrade() else { return false };
            match event {
                Event::Ready | Event::Moods => this.show_moods(),
                Event::Status => this.detail.show_status(),
                Event::MoodStats => this.detail.show_stats(),
                Event::History => this.detail.load_recent(),
                _ => {}
            }
            true
        });
        if app.is_ready() {
            this.show_moods();
        }
        this
    }

    /// The mood page (its header bar, then its content), for the window's content pane.
    pub fn detail_widget(&self) -> &adw::ToolbarView {
        &self.detail.root
    }

    /// A very narrow window drops the keywords' drag handles (see `KeywordEditor::set_compact`) and shortens the
    /// mood's header (Use; the checkmark alone for the current mood), so the name keeps some room.
    pub fn set_compact(&self, compact: bool) {
        self.detail.keywords.set_compact(compact);
        self.detail.set_compact(compact);
    }

    pub fn row_for_mood(&self, id: &str) -> Option<gtk::ListBoxRow> {
        self.row_for(id).map(|row| row.row.clone())
    }

    pub fn mood_of_row(&self, row: &gtk::ListBoxRow) -> Option<String> {
        self.rows.borrow().iter().find(|item| item.row == *row).map(|item| item.mood.borrow().id.clone())
    }

    /// The mood that takes a deleted one's place: the one now where it was in the list, else the last.
    pub fn replacement_for(&self, deleted: &str) -> Option<String> {
        let ids: Vec<String> = self.rows.borrow().iter().map(|row| row.mood.borrow().id.clone()).collect();
        let index = self.previous.borrow().iter().position(|id| id == deleted)?;
        ids.get(index.min(ids.len().checked_sub(1)?)).cloned()
    }

    /// Shows a mood on its page (the window chose it).
    pub fn show_mood(&self, id: &str) {
        let Some(app) = self.app.upgrade() else { return };
        let moods = app.state().moods.clone();
        if let Some(mood) = moods.iter().find(|mood| mood.id == id) {
            self.detail.show(mood, moods.len() <= 1);
        }
    }

    fn row_for(&self, id: &str) -> Option<Rc<MoodRow>> {
        self.rows.borrow().iter().find(|row| row.mood.borrow().id == id).cloned()
    }

    /// The mood whose row holds keyboard focus.
    fn focused_row(&self) -> Option<String> {
        let focus = gtk::prelude::RootExt::focus(&self.list.root()?)?;
        self.rows
            .borrow()
            .iter()
            .find(|row| focus == *row.row.upcast_ref::<gtk::Widget>() || focus.is_ancestor(&row.row))
            .map(|row| row.mood.borrow().id.clone())
    }

    /// Shows the moods: rows are updated in place while the list is the same moods in the same order (so focus and
    /// selection stay), and rebuilt otherwise, with keyboard focus kept on the same mood or moved to its neighbour when
    /// it's gone. The window then selects the row of the page it shows (it subscribed after this page).
    fn show_moods(self: &Rc<Self>) {
        let Some(app) = self.app.upgrade() else { return };
        let moods = app.state().moods.clone();
        if moods.is_empty() {
            return;
        }
        let ids: Vec<String> = moods.iter().map(|mood| mood.id.clone()).collect();
        let shown: Vec<String> = self.rows.borrow().iter().map(|row| row.mood.borrow().id.clone()).collect();
        let last = moods.len() == 1;
        if ids == shown {
            for (row, mood) in self.rows.borrow().iter().zip(&moods) {
                row.show(mood, last, moods.len());
            }
        } else {
            let focused = self.focused_row();
            for row in self.rows.take() {
                self.list.remove(&row.row);
            }
            let rows: Vec<Rc<MoodRow>> = moods.iter().map(|mood| self.make_row(mood, last, moods.len())).collect();
            for (index, row) in rows.iter().enumerate() {
                self.list.insert(&row.row, self.first + index as i32);
            }
            self.rows.replace(rows);
            // The same mood if it's still here; else the one now in its place.
            let pick = focused.and_then(|id| {
                if ids.contains(&id) {
                    return Some(id);
                }
                let index = shown.iter().position(|shown| *shown == id)?.min(ids.len() - 1);
                ids.get(index).cloned()
            });
            if let Some(row) = pick.and_then(|id| self.row_for(&id)) {
                row.row.grab_focus();
            }
        }
        self.previous.replace(shown);
        // The page follows its mood's fresh state.
        if let Some(mood) = self.detail.mood_id().and_then(|id| moods.iter().find(|mood| mood.id == id).cloned()) {
            self.detail.show(&mood, last);
        }
    }

    fn make_row(self: &Rc<Self>, mood: &Mood, last: bool, count: usize) -> Rc<MoodRow> {
        // Under Moods, with native sidebar insets; the checkmark (the current mood) at the end. The labels are presentation:
        // the row speaks its mood once ("Fog, Night, current mood"), with its keywords as the description.
        let name = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .accessible_role(gtk::AccessibleRole::Presentation)
            .build();
        let check = crate::history::decorative_icon("object-select-symbolic");
        check.set_tooltip_text(Some("Current mood"));
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        content.append(&name);
        content.append(&check);
        let row = gtk::ListBoxRow::builder().child(&content).build();
        let actions = gio::SimpleActionGroup::new();
        row.insert_action_group("mood", Some(&actions));
        crate::wallpapers::attach_menu(&row, &mood_menu());
        let item = Rc::new(MoodRow { mood: RefCell::new(mood.clone()), row, name, check, actions });
        let target = {
            let item = Rc::downgrade(&item);
            Rc::new(move || item.upgrade().map(|item| item.mood.borrow().id.clone())) as Rc<dyn Fn() -> Option<String>>
        };
        self.install_mood_actions(&item.actions, target);
        // Moving (Alt+Up / Alt+Down, and the menu's Move up / Move down) belongs to the row.
        for (name, step) in [("move-up", -1i64), ("move-down", 1)] {
            let action = gio::SimpleAction::new(name, None);
            let (weak, row) = (Rc::downgrade(self), Rc::downgrade(&item));
            action.connect_activate(move |_, _| {
                if let (Some(this), Some(row)) = (weak.upgrade(), row.upgrade()) {
                    let position = i64::from(row.mood.borrow().position) + step;
                    if position >= 0 {
                        this.move_mood(row.mood.borrow().id.clone(), position as u32);
                    }
                }
            });
            item.actions.add_action(&action);
        }
        self.connect_row_keys(&item);
        self.connect_row_drag(&item);
        item.show(mood, last, count);
        item
    }

    /// Use, Duplicate, Rename… and Delete… for the mood `target` names (a row's own).
    fn install_mood_actions(self: &Rc<Self>, group: &gio::SimpleActionGroup, target: Rc<dyn Fn() -> Option<String>>) {
        let add = |name: &str, run: MoodAction| {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(self);
            let target = target.clone();
            action.connect_activate(move |_, _| {
                if let (Some(this), Some(id)) = (weak.upgrade(), target()) {
                    run(&this, id);
                }
            });
            group.add_action(&action);
        };
        add("use", Box::new(|this, id| this.use_mood(id)));
        add("duplicate", Box::new(|this, id| this.duplicate(id)));
        add("rename", Box::new(|this, id| this.rename(id)));
        add("delete", Box::new(|this, id| this.confirm_delete(id)));
    }

    /// Delete: Delete…; Alt+Up / Alt+Down: move.
    fn connect_row_keys(self: &Rc<Self>, item: &Rc<MoodRow>) {
        let shortcuts = gtk::ShortcutController::new();
        shortcuts.set_scope(gtk::ShortcutScope::Local);
        // Delete asks first; on the last mood (whose menu item is off) it says why it can't go, rather than nothing.
        let (weak, row) = (Rc::downgrade(self), Rc::downgrade(item));
        let delete = gtk::CallbackAction::new(move |_, _| {
            if let (Some(this), Some(row)) = (weak.upgrade(), row.upgrade()) {
                this.confirm_delete(row.mood.borrow().id.clone());
            }
            glib::Propagation::Stop
        });
        if let Some(trigger) = gtk::ShortcutTrigger::parse_string("Delete") {
            shortcuts.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(delete)));
        }
        for (keys, action) in [("<Alt>Up", "mood.move-up"), ("<Alt>Down", "mood.move-down")] {
            if let (Some(trigger), Some(action)) =
                (gtk::ShortcutTrigger::parse_string(keys), gtk::ShortcutAction::parse_string(&format!("action({action})")))
            {
                shortcuts.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(action)));
            }
        }
        item.row.add_controller(shortcuts);
    }

    /// Drag a row onto another to move it there.
    fn connect_row_drag(self: &Rc<Self>, item: &Rc<MoodRow>) {
        let source = gtk::DragSource::builder().actions(gdk::DragAction::MOVE).build();
        let row = Rc::downgrade(item);
        source.connect_prepare(move |_, _, _| {
            row.upgrade().map(|row| gdk::ContentProvider::for_value(&row.mood.borrow().id.to_value()))
        });
        let row = Rc::downgrade(item);
        source.connect_drag_begin(move |source, _| {
            if let Some(row) = row.upgrade() {
                source.set_icon(Some(&gtk::WidgetPaintable::new(Some(&row.row))), 0, 0);
            }
        });
        item.row.add_controller(source);
        let target = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);
        let (weak, onto) = (Rc::downgrade(self), Rc::downgrade(item));
        target.connect_drop(move |_, value, _, _| {
            let (Some(this), Some(onto), Ok(dragged)) = (weak.upgrade(), onto.upgrade(), value.get::<String>()) else {
                return false;
            };
            if onto.mood.borrow().id == dragged || this.row_for(&dragged).is_none() {
                return false;
            }
            let position = onto.mood.borrow().position;
            this.move_mood(dragged, position);
            true
        });
        item.row.add_controller(target);
    }

    // ── Changes ─────────────────────────────────────────────────────────────────────────────────────────────

    fn toast_error(&self, app: &App, error: &AutoPaperError) {
        app.toast(&strings::error_sentence(error, app.state().settings.fallback));
    }

    pub fn use_mood(&self, id: String) {
        if let Some(app) = self.app.upgrade() {
            app.use_mood(id);
        }
    }

    /// Shows a mood on its page and starts renaming it, its name selected, to type a new one.
    fn open_with_name_focused(self: &Rc<Self>, id: &str) {
        let Some(window) = self.app.upgrade().and_then(|app| app.main_window()) else { return };
        window.show_mood(id);
        let detail = Rc::downgrade(&self.detail);
        glib::idle_add_local_once(move || {
            if let Some(detail) = detail.upgrade() {
                detail.start_renaming();
            }
        });
    }

    /// + or Ctrl+N: "New mood" (or "New mood 2", …), empty, not in use yet; its name is ready to type over.
    pub fn new_mood(self: &Rc<Self>) {
        let Some(app) = self.app.upgrade() else { return };
        let name = strings::unused_mood_name("New mood", &app.state().moods);
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let created = app.change_moods(false, move |engine| engine.create_mood(name, None)).await;
            let Some(this) = weak.upgrade() else { return };
            match created {
                Ok(mood) => this.open_with_name_focused(&mood.id),
                Err(error) => this.toast_error(&app, &error),
            }
        });
    }

    fn duplicate(self: &Rc<Self>, id: String) {
        let Some(app) = self.app.upgrade() else { return };
        let Some(source) = app.state().moods.iter().find(|mood| mood.id == id).cloned() else { return };
        let name = strings::mood_copy_name(&source.name, &app.state().moods);
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let created = app.change_moods(false, move |engine| engine.create_mood(name, Some(id))).await;
            let Some(this) = weak.upgrade() else { return };
            match created {
                Ok(mood) => this.open_with_name_focused(&mood.id),
                Err(error) => this.toast_error(&app, &error),
            }
        });
    }

    /// Rename…: the mood's page, its name ready to type over.
    fn rename(self: &Rc<Self>, id: String) {
        self.open_with_name_focused(&id);
    }

    fn move_mood(self: &Rc<Self>, id: String, position: u32) {
        let Some(app) = self.app.upgrade() else { return };
        let Some(name) = app.state().moods.iter().find(|mood| mood.id == id).map(|mood| mood.name.clone()) else { return };
        let count = app.state().moods.len() as u32;
        if position >= count {
            return;
        }
        let weak = Rc::downgrade(self);
        let moved = id.clone();
        glib::spawn_future_local(async move {
            let result = app.change_moods(false, move |engine| engine.move_mood(id, position)).await;
            let Some(this) = weak.upgrade() else { return };
            match result {
                Ok(()) => {
                    if let Some(row) = this.row_for(&moved) {
                        row.row.grab_focus();
                    }
                    this.list.announce(&format!("Moved {name} to position {}.", position + 1), gtk::AccessibleAnnouncementPriority::Medium);
                }
                Err(error) => this.toast_error(&app, &error),
            }
        });
    }

    pub fn confirm_delete(self: &Rc<Self>, id: String) {
        let Some(app) = self.app.upgrade() else { return };
        let moods = app.state().moods.clone();
        let Some(mood) = moods.iter().find(|mood| mood.id == id).cloned() else { return };
        if moods.len() <= 1 {
            // The action is off for the last mood; Delete on its row says why instead of doing nothing.
            self.toast_error(&app, &AutoPaperError::InvalidInput {
                reason: autopaper_core::InvalidInputReason::LastMood,
                detail: String::new(),
            });
            return;
        }
        // The core's rule for who takes over: the next mood in the list, or the one before when it was the last.
        let index = moods.iter().position(|other| other.id == mood.id).unwrap_or(0);
        let next = moods.get(index + 1).or_else(|| index.checked_sub(1).and_then(|before| moods.get(before)));
        let body = match next.filter(|_| mood.active) {
            Some(next) => format!(
                "Its keywords and Surprise are deleted, and “{}” becomes the current mood. Wallpapers made in it stay \
                 in History.",
                next.name
            ),
            None => "Its keywords and Surprise are deleted. Wallpapers made in it stay in History.".to_string(),
        };
        let dialog = adw::AlertDialog::new(Some(&format!("Delete “{}”?", mood.name)), Some(&body));
        dialog.add_responses(&[("cancel", "_Cancel"), ("delete", "_Delete")]);
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let weak = Rc::downgrade(self);
        let anchor: gtk::Widget = self.list.clone().upcast();
        glib::spawn_future_local(async move {
            if dialog.choose_future(Some(&anchor)).await != "delete" {
                return;
            }
            let was_active = mood.active;
            let result = app.change_moods(was_active, move |engine| engine.delete_mood(id)).await;
            let Some(this) = weak.upgrade() else { return };
            match result {
                Ok(()) => {
                    // The neighbour now selected takes focus (the deleted row's went with it).
                    if let Some(row) = this.list.selected_row() {
                        row.grab_focus();
                    }
                    let mut message = format!("Deleted “{}”.", mood.name);
                    if was_active && let Some(active) = app.active_mood() {
                        message.push(' ');
                        message.push_str(&strings::mood_in_use(&active.name));
                    }
                    app.toast(&message);
                }
                Err(error) => this.toast_error(&app, &error),
            }
        });
    }
}

impl MoodRow {
    fn show(&self, mood: &Mood, last: bool, count: usize) {
        self.mood.replace(mood.clone());
        self.name.set_label(&mood.name);
        // A checkmark, never colour alone; the row's name says it in words.
        self.check.set_opacity(if mood.active { 1.0 } else { 0.0 });
        self.row.set_tooltip_text(mood.active.then_some("Current mood"));
        self.row.update_property(&[
            gtk::accessible::Property::Label(&strings::mood_spoken(mood)),
            gtk::accessible::Property::Description(&strings::mood_keywords(&mood.keywords)),
        ]);
        let position = mood.position as usize;
        for (name, enabled) in [
            ("use", !mood.active),
            ("delete", !last),
            ("move-up", position > 0),
            ("move-down", position + 1 < count),
        ] {
            if let Some(action) = self.actions.lookup_action(name).and_downcast::<gio::SimpleAction>() {
                action.set_enabled(enabled);
            }
        }
    }
}

/// A mood's page: its header bar (name, Use this mood or "Current mood", reload), then what it has made, its keywords,
/// its Surprise, its latest wallpapers, Duplicate mood and Delete mood….
struct MoodDetail {
    root: adw::ToolbarView,
    app: Weak<App>,
    page: RefCell<Weak<MoodsPage>>,
    banner: adw::Banner,
    prefs_page: adw::PreferencesPage,
    /// The name, renamable in place, in the header's title position.
    name: gtk::EditableLabel,
    current: gtk::Box,
    current_words: gtk::Label,
    use_button: gtk::Button,
    make: Rc<MakeOrStop>,
    tiles: [StatTile; 4],
    tiles_group: adw::PreferencesGroup,
    keywords: Rc<KeywordEditor>,
    scale: gtk::Scale,
    band: gtk::Label,
    band_detail: gtk::Label,
    surprise_save: RefCell<Option<glib::SourceId>>,
    recent_box: gtk::Box,
    recent_empty: gtk::Label,
    recent_group: adw::PreferencesGroup,
    delete_group: adw::PreferencesGroup,
    duplicate: adw::ButtonRow,
    delete: adw::ButtonRow,
    mood: RefCell<Option<Mood>>,
    updating: Cell<bool>,
    /// The latest wallpapers were loaded for this mood (reloaded on History changes).
    recent_for: RefCell<Option<String>>,
}

impl MoodDetail {
    fn new(app: &Rc<App>, toasts: &adw::ToastOverlay) -> Rc<Self> {
        let banner = adw::Banner::builder()
            .title("Your keywords are narrow, so new ideas are getting hard to find. Add some Maybes or raise Surprise for more variety.")
            .build();

        // A text box to screen readers ("Mood name, Fog, Night"): GTK gives an editable label the generic role, whose
        // name it drops, so it would be read as nothing.
        let name = gtk::EditableLabel::builder()
            .xalign(0.5)
            // Small at least (a phone's header has room for little else), up to 28 characters when there's room.
            .width_chars(4)
            .max_width_chars(28)
            .css_classes(["mood-title"])
            .accessible_role(gtk::AccessibleRole::TextBox)
            .build();
        name.update_property(&[
            gtk::accessible::Property::Label("Mood name"),
            gtk::accessible::Property::Description("Press Enter to rename"),
        ]);
        name.set_tooltip_text(Some("Click to rename. Enter keeps the new name, Esc puts the old one back."));
        let use_button = gtk::Button::builder().label("_Use this mood").use_underline(true).css_classes(["suggested-action"]).build();
        use_button.set_tooltip_text(Some("Make this the current mood. Nothing changes until the next wallpaper."));
        // "Current mood": a checkmark and the words, never colour alone (in a phone-width window the checkmark alone,
        // the words still its accessible name).
        let current = gtk::Box::builder().spacing(6).valign(gtk::Align::Center).accessible_role(gtk::AccessibleRole::Group).build();
        current.update_property(&[gtk::accessible::Property::Label("Current mood")]);
        current.append(&crate::history::decorative_icon("object-select-symbolic"));
        let current_words = gtk::Label::builder().label("Current mood").css_classes(["heading"]).build();
        current.append(&current_words);
        current.set_tooltip_text(Some("Current mood: new wallpapers are made from its keywords."));
        let make = MakeOrStop::new(app);
        let header = adw::HeaderBar::builder().title_widget(&name).build();
        header.pack_end(make.widget());
        header.pack_end(&use_button);
        header.pack_end(&current);

        let tiles = [
            StatTile::new("image-x-generic-symbolic", "Wallpapers", true),
            StatTile::new("autopaper-like-symbolic", "Liked", true),
            StatTile::new("media-playlist-repeat-symbolic", "Echoes", true),
            StatTile::new("document-open-recent-symbolic", "Last made", true),
        ];
        let tile_box = adw::WrapBox::builder().child_spacing(8).line_spacing(8).justify(adw::JustifyMode::Fill).build();
        for tile in &tiles {
            tile_box.append(tile.widget());
        }
        let tiles_group = adw::PreferencesGroup::new();
        tiles_group.add(&tile_box);

        let keywords = KeywordEditor::new(app, toasts);

        let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
        scale.set_draw_value(false);
        scale.set_hexpand(true);
        scale.set_increments(1.0, 10.0);
        scale.add_mark(0.0, gtk::PositionType::Bottom, Some("Faithful"));
        scale.add_mark(100.0, gtk::PositionType::Bottom, Some("Wild"));
        scale.update_property(&[gtk::accessible::Property::Label("Surprise")]);
        let band = gtk::Label::builder().xalign(0.0).css_classes(["heading"]).build();
        let band_detail = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["secondary-text"]).build();
        let surprise_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .build();
        surprise_box.append(&scale);
        surprise_box.append(&band);
        surprise_box.append(&band_detail);
        let surprise = adw::PreferencesGroup::builder()
            .title("Surprise")
            .description("How much freedom AutoPaper takes with this mood's keywords.")
            .build();
        surprise.add(&surprise_box);

        let recent_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        let recent_empty = gtk::Label::builder()
            .label("Nothing yet. Wallpapers made while this mood is current appear here.")
            .xalign(0.0)
            .wrap(true)
            .css_classes(["secondary-text"])
            .build();
        let show_all = gtk::Button::builder().label("Show in History").valign(gtk::Align::Center).css_classes(["flat"]).build();
        let recent_group = adw::PreferencesGroup::builder()
            .title("Made in this mood")
            .description("Click one to show it on your desktop.")
            .header_suffix(&show_all)
            .build();
        recent_group.add(&recent_box);
        recent_group.add(&recent_empty);

        // Duplicate is here as well as in the row's menu: the page has no menu of its own.
        let duplicate = adw::ButtonRow::builder().title("Duplicate mood").build();
        let delete = adw::ButtonRow::builder().title("Delete mood…").build();
        delete.add_css_class("destructive-action");
        let delete_list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
        delete_list.append(&duplicate);
        delete_list.append(&delete);
        let delete_group = adw::PreferencesGroup::new();
        delete_group.add(&delete_list);

        let prefs_page = adw::PreferencesPage::new();
        prefs_page.add(&tiles_group);
        prefs_page.add(keywords.widget());
        prefs_page.add(&surprise);
        prefs_page.add(&recent_group);
        prefs_page.add(&delete_group);
        prefs_page.set_vexpand(true);
        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        body.append(&banner);
        body.append(&prefs_page);
        let root = adw::ToolbarView::new();
        root.add_top_bar(&header);
        root.set_content(Some(&body));

        let this = Rc::new(Self {
            root,
            app: Rc::downgrade(app),
            page: RefCell::new(Weak::new()),
            banner,
            prefs_page,
            name,
            current,
            current_words,
            use_button,
            make,
            tiles,
            tiles_group,
            keywords,
            scale,
            band,
            band_detail,
            surprise_save: RefCell::new(None),
            recent_box,
            recent_empty,
            recent_group,
            delete_group,
            duplicate,
            delete,
            mood: RefCell::new(None),
            updating: Cell::new(false),
            recent_for: RefCell::new(None),
        });
        this.connect(&show_all);
        this
    }

    fn mood_id(&self) -> Option<String> {
        self.mood.borrow().as_ref().map(|mood| mood.id.clone())
    }

    /// A phone-width header: "Use" (named "Use this mood", which holds the word shown) and the checkmark alone.
    fn set_compact(&self, compact: bool) {
        self.use_button.set_label(if compact { "_Use" } else { "_Use this mood" });
        self.use_button.update_property(&[gtk::accessible::Property::Label("Use this mood")]);
        self.current_words.set_visible(!compact);
    }

    fn connect(self: &Rc<Self>, show_all: &gtk::Button) {
        // Renaming in place: when editing ends, a changed name is saved (Esc has put the old one back already).
        let weak = Rc::downgrade(self);
        self.name.connect_editing_notify(move |label| {
            if label.is_editing() {
                return;
            }
            if let Some(this) = weak.upgrade() {
                this.rename(label.text().to_string());
            }
        });
        let weak = Rc::downgrade(self);
        self.use_button.connect_clicked(move |_| {
            let Some(this) = weak.upgrade() else { return };
            // The button goes once the mood is current: focus moves to the reload button beside it.
            this.make.widget().grab_focus();
            if let (Some(app), Some(id)) = (this.app.upgrade(), this.mood_id()) {
                app.use_mood(id);
            }
        });
        let weak = Rc::downgrade(self);
        self.duplicate.connect_activated(move |_| {
            let Some(this) = weak.upgrade() else { return };
            let page = this.page.borrow().upgrade();
            if let (Some(page), Some(id)) = (page, this.mood_id()) {
                page.duplicate(id);
            }
        });
        let weak = Rc::downgrade(self);
        self.delete.connect_activated(move |_| {
            let Some(this) = weak.upgrade() else { return };
            let page = this.page.borrow().upgrade();
            if let (Some(page), Some(id)) = (page, this.mood_id()) {
                page.confirm_delete(id);
            }
        });
        let weak = Rc::downgrade(self);
        show_all.connect_clicked(move |_| {
            let Some(this) = weak.upgrade() else { return };
            if let (Some(window), Some(id)) = (this.app.upgrade().and_then(|app| app.main_window()), this.mood_id()) {
                window.show_mood_history(&id);
            }
        });
        let weak = Rc::downgrade(self);
        self.scale.connect_value_changed(move |scale| {
            let Some(this) = weak.upgrade() else { return };
            let surprise = (scale.value() / 100.0) as f32;
            this.show_band(surprise);
            if this.updating.get() {
                return;
            }
            this.schedule_surprise_save(surprise);
        });
    }

    /// Saved once the slider rests (not on every step of a drag), to the mood shown — current or not.
    fn schedule_surprise_save(self: &Rc<Self>, surprise: f32) {
        if let Some(pending) = self.surprise_save.take() {
            pending.remove();
        }
        let Some(mood_id) = self.mood_id() else { return };
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local_once(Duration::from_millis(400), move || {
            let Some(this) = weak.upgrade() else { return };
            this.surprise_save.replace(None);
            let Some(app) = this.app.upgrade() else { return };
            let active = app.active_mood().is_some_and(|mood| mood.id == mood_id);
            glib::spawn_future_local(async move {
                let saved = app.change_moods(active, move |engine| engine.set_mood_surprise(mood_id, surprise)).await;
                if let Err(error) = saved {
                    app.toast(&strings::error_sentence(&error, app.state().settings.fallback));
                }
            });
        });
        self.surprise_save.replace(Some(source));
    }

    /// Saves a Surprise still waiting for the slider to rest, to the mood it was set on (another mood is about to be
    /// shown).
    fn flush_surprise(&self) {
        let Some(pending) = self.surprise_save.take() else { return };
        pending.remove();
        let (Some(app), Some(mood_id)) = (self.app.upgrade(), self.mood_id()) else { return };
        let surprise = (self.scale.value() / 100.0) as f32;
        let active = app.active_mood().is_some_and(|mood| mood.id == mood_id);
        glib::spawn_future_local(async move {
            if let Err(error) = app.change_moods(active, move |engine| engine.set_mood_surprise(mood_id, surprise)).await {
                tracing::warn!(%error, "couldn't save a mood's Surprise");
            }
        });
    }

    fn show_band(&self, surprise: f32) {
        let (band, detail) = strings::surprise_band(surprise);
        self.band.set_label(band);
        self.band_detail.set_label(detail);
        self.scale.update_property(&[gtk::accessible::Property::ValueText(&strings::surprise_spoken(surprise))]);
    }

    /// Starts renaming: the name becomes a field with its text selected.
    fn start_renaming(&self) {
        self.name.grab_focus();
        self.name.start_editing();
        self.name.select_region(0, -1);
    }

    fn rename(self: &Rc<Self>, text: String) {
        let Some(mood) = self.mood.borrow().clone() else { return };
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text == mood.name {
            // Unchanged, or only spaces added: show it as it's stored.
            self.name.set_text(&mood.name);
            return;
        }
        let Some(app) = self.app.upgrade() else { return };
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let id = mood.id.clone();
            let renamed = app.change_moods(false, move |engine| engine.rename_mood(id, text)).await;
            let Some(this) = weak.upgrade() else { return };
            match renamed {
                Ok(mood) => {
                    this.name.set_text(&mood.name);
                    this.name.announce(&format!("Renamed to {}.", mood.name), gtk::AccessibleAnnouncementPriority::Medium);
                }
                Err(error) => {
                    // The stored name comes back; the toast says why the new one wasn't kept.
                    if let Some(stored) = this.mood.borrow().as_ref() {
                        this.name.set_text(&stored.name);
                    }
                    app.toast(&strings::error_sentence(&error, app.state().settings.fallback));
                }
            }
        });
    }

    /// Shows `mood`; `last`: it's the only mood (Delete is off, with the reason said).
    fn show(self: &Rc<Self>, mood: &Mood, last: bool) {
        let another = self.mood_id().as_deref() != Some(mood.id.as_str());
        if another {
            self.flush_surprise();
            if self.name.is_editing() {
                self.name.stop_editing(false);
            }
            // Each mood starts at the top.
            self.prefs_page.scroll_to_top();
        }
        self.mood.replace(Some(mood.clone()));
        if another || !self.name.is_editing() {
            self.name.set_text(&mood.name);
        }
        self.name.update_property(&[gtk::accessible::Property::Label(&format!("Mood name, {}", mood.name))]);
        self.make.set_mood(Some(mood.id.clone()));
        // The button goes once the mood is current: focus never stays on it.
        if mood.active && self.use_button.has_focus() {
            self.make.widget().grab_focus();
        }
        self.current.set_visible(mood.active);
        self.use_button.set_visible(!mood.active);
        self.use_button.update_property(&[gtk::accessible::Property::Description(&mood.name)]);
        self.keywords.show(mood);
        if self.surprise_save.borrow().is_none() {
            let value = f64::from((mood.surprise * 100.0).round());
            if another || (self.scale.value() - value).abs() > 0.5 || self.band.label().is_empty() {
                self.updating.set(true);
                self.scale.set_value(value);
                self.updating.set(false);
            }
            self.show_band(mood.surprise);
        }
        self.duplicate.update_property(&[gtk::accessible::Property::Description(&mood.name)]);
        self.delete.set_sensitive(!last);
        self.delete_group.set_description(last.then_some("AutoPaper always keeps one mood, so the last one can't be deleted."));
        self.show_status();
        self.show_stats();
        if another {
            self.load_recent();
        }
    }

    /// The narrow-keywords note: about the mood in use, so shown only on it.
    fn show_status(&self) {
        let Some(app) = self.app.upgrade() else { return };
        let active = self.mood.borrow().as_ref().is_some_and(|mood| mood.active);
        self.banner.set_revealed(active && app.state().narrow);
    }

    /// What the mood has made: wallpapers, liked, echoes, and when the last one was made.
    fn show_stats(&self) {
        let (Some(app), Some(mood)) = (self.app.upgrade(), self.mood.borrow().clone()) else { return };
        let stats = app.state().mood_stats.iter().find(|stats| stats.mood_id == mood.id).cloned();
        let now = glib::real_time() / 1_000_000;
        let (made, liked, echoes, last) = stats
            .as_ref()
            .map(|stats| (stats.wallpapers, stats.liked, stats.echoes, stats.last_made_at))
            .unwrap_or_default();
        self.tiles[0].set(&made.to_string(), &strings::count(made, "wallpaper", "wallpapers"));
        self.tiles[1].set(&liked.to_string(), &format!("{liked} liked"));
        self.tiles[2].set(&echoes.to_string(), &strings::count(echoes, "echo", "echoes"));
        let last_made = strings::last_made_tile(last, now);
        let spoken = match last {
            Some(_) => format!("Last made {}", last_made.to_lowercase()),
            None => "Nothing made yet".to_string(),
        };
        self.tiles[3].set(&last_made, &spoken);
        self.tiles_group.update_property(&[gtk::accessible::Property::Label(&format!("Made in {}", mood.name))]);
    }

    /// The wallpapers made in this mood, newest first (a click shows one on the desktop; each has History's menu).
    fn load_recent(self: &Rc<Self>) {
        let (Some(app), Some(mood_id)) = (self.app.upgrade(), self.mood_id()) else { return };
        let Some(engine) = app.engine() else { return };
        self.recent_for.replace(Some(mood_id.clone()));
        let weak = Rc::downgrade(self);
        let asked = mood_id.clone();
        glib::spawn_future_local(async move {
            let read = blocking(move || {
                let made = engine.history_by_mood(HistoryFilter::All, Some(asked), RECENT, 0)?;
                Ok(made.into_iter().map(|generation| Item::read(&engine, generation)).collect::<Vec<Item>>())
            })
            .await;
            let Some(this) = weak.upgrade() else { return };
            if this.recent_for.borrow().as_deref() != Some(mood_id.as_str()) {
                return; // Another mood is showing now.
            }
            match read {
                Ok(items) => this.fill_recent(&app, items),
                Err(error) => tracing::warn!(%error, "couldn't read a mood's wallpapers"),
            }
        });
    }

    fn fill_recent(&self, app: &Rc<App>, items: Vec<Item>) {
        // Focus on a thumbnail stays on the one in its place after the strip is rebuilt.
        let focused_index = self.root.root().and_then(|root| gtk::prelude::RootExt::focus(&root)).and_then(|focus| {
            let mut index = 0;
            let mut child = self.recent_box.first_child().and_then(|strip| strip.first_child());
            while let Some(flow_child) = child {
                if focus.is_ancestor(&flow_child) {
                    return Some(index);
                }
                index += 1;
                child = flow_child.next_sibling();
            }
            None
        });
        while let Some(child) = self.recent_box.first_child() {
            self.recent_box.remove(&child);
        }
        self.recent_empty.set_visible(items.is_empty());
        self.recent_box.set_visible(!items.is_empty());
        if let Some(suffix) = self.recent_group.header_suffix() {
            suffix.set_visible(!items.is_empty());
        }
        self.recent_group.set_description((!items.is_empty()).then_some("Click one to show it on your desktop."));
        if items.is_empty() {
            return;
        }
        let name = self.mood.borrow().as_ref().map(|mood| mood.name.clone()).unwrap_or_default();
        let strip = wallpapers::thumbnail_strip(app, &items, RECENT_WIDTH, RECENT_HEIGHT, &format!("Wallpapers made in {name}"));
        self.recent_box.append(&strip);
        if let Some(index) = focused_index
            && let Some(child) = strip.child_at_index(index.min(items.len() as i32 - 1))
            && let Some(button) = child.child()
        {
            button.grab_focus();
        }
    }
}

/// The menu for a mood: a row's context menu.
fn mood_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let main = gio::Menu::new();
    let use_item = gio::MenuItem::new(Some("Use"), Some("mood.use"));
    // The current mood has nothing to use: the item goes rather than sit there off.
    use_item.set_attribute_value("hidden-when", Some(&"action-disabled".to_variant()));
    main.append_item(&use_item);
    main.append(Some("Duplicate"), Some("mood.duplicate"));
    main.append(Some("Rename…"), Some("mood.rename"));
    menu.append_section(None, &main);
    let order = gio::Menu::new();
    for (label, action, accel) in [("Move up", "mood.move-up", "<Alt>Up"), ("Move down", "mood.move-down", "<Alt>Down")] {
        let item = gio::MenuItem::new(Some(label), Some(action));
        item.set_attribute_value("accel", Some(&accel.to_variant()));
        order.append_item(&item);
    }
    menu.append_section(None, &order);
    let delete = gio::Menu::new();
    let item = gio::MenuItem::new(Some("Delete…"), Some("mood.delete"));
    // The Delete key on a mood's row (shown here, as GTK shows a shortcut).
    item.set_attribute_value("accel", Some(&"Delete".to_variant()));
    delete.append_item(&item);
    menu.append_section(None, &delete);
    menu
}
