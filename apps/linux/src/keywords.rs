//! A mood's keywords, as the Moods detail pane shows them: "Add a keyword" (Return adds it as a Must; a duplicate
//! highlights the row it already has), then one row per keyword — editable inline (Return or the apply button
//! renames), a Must / Maybe / Avoid AdwToggleGroup, and a menu with Move up, Move down and Remove. Rows reorder by
//! dragging their handle or with Alt+Up / Alt+Down; each row speaks "rain, Must". Remove has Undo, on its toast and
//! as Ctrl+Z (the toast goes after a while; the shortcut doesn't). Esc in a keyword's field puts its text back (in
//! "Add a keyword", empties it): Esc is the field's while the keyboard is in one, and never stops a wallpaper being
//! made (window.rs).
//!
//! The editor works on any mood, not only the one in use: adding goes through `add_mood_keyword`, and the core's
//! keyword methods that take an id work within the keyword's own mood. Every change goes through the app's ordered
//! writer and the moods are read again, so the list and the header follow.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use adw::prelude::*;
use autopaper_core::{AutoPaperError, Keyword, KeywordWeight, Mood};
use gtk::{gdk, gio, glib};

use crate::app::App;
use crate::strings;

const WEIGHTS: [(KeywordWeight, &str, &str); 3] = [
    (KeywordWeight::Must, "must", "Must"),
    (KeywordWeight::Maybe, "maybe", "Maybe"),
    (KeywordWeight::Avoid, "avoid", "Avoid"),
];

fn weight_name(weight: KeywordWeight) -> &'static str {
    WEIGHTS.iter().find(|(w, _, _)| *w == weight).map(|(_, name, _)| *name).unwrap_or("must")
}

fn weight_label(weight: KeywordWeight) -> &'static str {
    WEIGHTS.iter().find(|(w, _, _)| *w == weight).map(|(_, _, label)| *label).unwrap_or("Must")
}

fn weight_from_name(name: &str) -> Option<KeywordWeight> {
    WEIGHTS.iter().find(|(_, n, _)| *n == name).map(|(weight, _, _)| *weight)
}

/// Empties an entry row without bringing its apply button back (clearing counts as an edit otherwise).
pub fn clear_entry(row: &adw::EntryRow) {
    row.set_show_apply_button(false);
    row.set_text("");
    row.set_show_apply_button(true);
}

/// Marks an entry row as holding text the engine refused: red, and "invalid" to screen readers.
pub fn mark_invalid(row: &adw::EntryRow, invalid: bool) {
    if invalid == row.has_css_class("error") {
        return;
    }
    if invalid {
        row.add_css_class("error");
    } else {
        row.remove_css_class("error");
    }
    let state = if invalid { gtk::AccessibleInvalidState::True } else { gtk::AccessibleInvalidState::False };
    if let Some(text) = row.delegate() {
        text.update_state(&[gtk::accessible::State::Invalid(state)]);
    }
}

/// Esc on `row` (while the keyboard is in it) runs `undo`, which says whether it changed anything: if it didn't, Esc goes
/// on up (and does nothing there while the keyboard is in a text field, window.rs).
fn add_escape(row: &adw::EntryRow, undo: impl Fn() -> bool + 'static) {
    let shortcuts = gtk::ShortcutController::new();
    shortcuts.set_scope(gtk::ShortcutScope::Local);
    let escape = gtk::CallbackAction::new(move |_, _| if undo() { glib::Propagation::Stop } else { glib::Propagation::Proceed });
    if let Some(trigger) = gtk::ShortcutTrigger::parse_string("Escape") {
        shortcuts.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(escape)));
    }
    row.add_controller(shortcuts);
}

/// How the core compares keywords: trimmed, single-spaced, case-insensitive.
fn normalized(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

pub struct KeywordEditor {
    group: adw::PreferencesGroup,
    app: Weak<App>,
    toasts: adw::ToastOverlay,
    add_row: adw::EntryRow,
    list: gtk::ListBox,
    empty_row: adw::ActionRow,
    rows: RefCell<Vec<Rc<KeywordRow>>>,
    /// The mood being edited, and its keywords as last shown (to rebuild only when they really changed, so a
    /// field being typed in isn't rebuilt under the person).
    mood_id: RefCell<Option<String>>,
    shown: RefCell<Vec<Keyword>>,
    /// The last keyword removed, for Ctrl+Z.
    removed: RefCell<Option<Keyword>>,
    /// A very narrow window: rows leave out their drag handles (see `set_compact`).
    compact: Cell<bool>,
}

type RowAction = Box<dyn Fn(&Rc<KeywordEditor>, &Rc<KeywordRow>)>;

struct KeywordRow {
    keyword: RefCell<Keyword>,
    row: adw::EntryRow,
    handle: gtk::Image,
    toggles: adw::ToggleGroup,
    menu: gtk::MenuButton,
    actions: gio::SimpleActionGroup,
    updating: Cell<bool>,
}

impl KeywordEditor {
    pub fn new(app: &Rc<App>, toasts: &adw::ToastOverlay) -> Rc<Self> {
        let add_row = adw::EntryRow::builder().title("Add a keyword").show_apply_button(true).build();
        let empty_row = adw::ActionRow::builder()
            .title("No keywords yet")
            .subtitle("Add a few words, like “rain”, “ruins” or “night”. AutoPaper makes each wallpaper from them.")
            .build();
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list-separate"])
            .build();
        list.update_property(&[gtk::accessible::Property::Label("Keywords")]);
        list.append(&add_row);
        list.append(&empty_row);
        let group = adw::PreferencesGroup::builder()
            .title("Keywords")
            .description("Must: always in the scene. Maybe: sometimes. Avoid: never, and never named to the painter.")
            .build();
        group.add(&list);

        let this = Rc::new(Self {
            group,
            app: Rc::downgrade(app),
            toasts: toasts.clone(),
            add_row,
            list,
            empty_row,
            rows: RefCell::new(Vec::new()),
            mood_id: RefCell::new(None),
            shown: RefCell::new(Vec::new()),
            removed: RefCell::new(None),
            compact: Cell::new(false),
        });
        this.connect_signals();
        this
    }

    pub fn widget(&self) -> &adw::PreferencesGroup {
        &self.group
    }

    fn connect_signals(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.add_row.connect_apply(move |row| {
            if let Some(this) = weak.upgrade() {
                this.add(row.text().to_string());
            }
        });
        // A refused keyword stays in the field, marked, until it's edited.
        self.add_row.connect_changed(|row| mark_invalid(row, false));
        // Ctrl+Z in the keywords brings back the last one removed. A text field with something typed in it keeps
        // Ctrl+Z for its own undo; an empty one (focus goes to the empty "Add a keyword" after a removal) has nothing
        // of its own to undo. Caught before the field sees it: GTK's text fields take Ctrl+Z even with nothing to undo.
        let shortcuts = gtk::ShortcutController::new();
        shortcuts.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        let undo = gtk::CallbackAction::new(move |widget, _| {
            let Some(this) = weak.upgrade() else { return glib::Propagation::Proceed };
            let focus = widget.root().and_then(|root| gtk::prelude::RootExt::focus(&root));
            let typing = focus.and_downcast::<gtk::Text>().is_some_and(|text| !text.text().is_empty());
            if typing || this.removed.borrow().is_none() {
                return glib::Propagation::Proceed;
            }
            this.undo_remove();
            glib::Propagation::Stop
        });
        if let Some(trigger) = gtk::ShortcutTrigger::parse_string("<Control>z") {
            shortcuts.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(undo)));
        }
        self.group.add_controller(shortcuts);
        // Esc empties "Add a keyword" when something is typed in it.
        let add_row = self.add_row.downgrade();
        add_escape(&self.add_row, move || {
            let Some(row) = add_row.upgrade() else { return false };
            if row.text().is_empty() {
                return false;
            }
            clear_entry(&row);
            mark_invalid(&row, false);
            true
        });
    }

    /// In a very narrow window (a phone's 360 px) the drag handles go, so Must / Maybe / Avoid keep their words
    /// and the keyword its room; rows still move with their menu and Alt+Up / Alt+Down.
    pub fn set_compact(&self, compact: bool) {
        self.compact.set(compact);
        for row in self.rows.borrow().iter() {
            row.set_compact(compact);
        }
    }

    /// Shows `mood`'s keywords. Another mood starts afresh; the same mood is rebuilt only when its keywords changed
    /// (keyboard focus stays on the keyword that had it).
    pub fn show(self: &Rc<Self>, mood: &Mood) {
        let same_mood = self.mood_id.borrow().as_deref() == Some(mood.id.as_str());
        if same_mood && *self.shown.borrow() == mood.keywords {
            return;
        }
        if !same_mood {
            self.mood_id.replace(Some(mood.id.clone()));
            self.removed.replace(None);
            clear_entry(&self.add_row);
            mark_invalid(&self.add_row, false);
        }
        let focus = if same_mood { self.focused_keyword() } else { None };
        self.rebuild(mood.keywords.clone(), focus);
    }

    /// The keyword whose row holds keyboard focus.
    fn focused_keyword(&self) -> Option<String> {
        let focus = gtk::prelude::RootExt::focus(&self.group.root()?)?;
        self.rows
            .borrow()
            .iter()
            .find(|row| focus == *row.row.upcast_ref::<gtk::Widget>() || focus.is_ancestor(&row.row))
            .map(|row| row.keyword.borrow().id.clone())
    }

    fn rebuild(self: &Rc<Self>, keywords: Vec<Keyword>, focus: Option<String>) {
        for row in self.rows.take() {
            self.list.remove(&row.row);
        }
        let count = keywords.len();
        let rows: Vec<Rc<KeywordRow>> =
            keywords.iter().cloned().enumerate().map(|(index, keyword)| self.make_row(keyword, index, count)).collect();
        for row in &rows {
            self.list.append(&row.row);
        }
        self.empty_row.set_visible(rows.is_empty());
        if let Some(id) = focus
            && let Some(row) = rows.iter().find(|row| row.keyword.borrow().id == id)
        {
            row.row.grab_focus();
        }
        self.rows.replace(rows);
        self.shown.replace(keywords);
    }

    fn make_row(self: &Rc<Self>, keyword: Keyword, index: usize, count: usize) -> Rc<KeywordRow> {
        let row = adw::EntryRow::builder().text(&keyword.text).show_apply_button(true).build();
        let handle = gtk::Image::builder()
            .icon_name("list-drag-handle-symbolic")
            .tooltip_text("Drag to reorder")
            .css_classes(["secondary-text"])
            .build();
        handle.update_property(&[gtk::accessible::Property::Label("Drag to reorder")]);
        row.add_prefix(&handle);

        let toggles = adw::ToggleGroup::builder().valign(gtk::Align::Center).can_shrink(false).build();
        for (_, name, label) in WEIGHTS {
            toggles.add(adw::Toggle::builder().name(name).label(label).build());
        }
        toggles.set_active_name(Some(weight_name(keyword.weight)));
        row.add_suffix(&toggles);

        let menu_model = gio::Menu::new();
        menu_model.append(Some("Move up"), Some("row.move-up"));
        menu_model.append(Some("Move down"), Some("row.move-down"));
        let remove_section = gio::Menu::new();
        remove_section.append(Some("Remove"), Some("row.remove"));
        menu_model.append_section(None, &remove_section);
        let menu = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .menu_model(&menu_model)
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        row.add_suffix(&menu);

        let actions = gio::SimpleActionGroup::new();
        row.insert_action_group("row", Some(&actions));
        let shortcuts = gtk::ShortcutController::new();
        shortcuts.set_scope(gtk::ShortcutScope::Local);
        for (keys, action) in [("<Alt>Up", "row.move-up"), ("<Alt>Down", "row.move-down")] {
            if let (Some(trigger), Some(action)) =
                (gtk::ShortcutTrigger::parse_string(keys), gtk::ShortcutAction::parse_string(&format!("action({action})")))
            {
                shortcuts.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(action)));
            }
        }
        row.add_controller(shortcuts);

        let item = Rc::new(KeywordRow {
            keyword: RefCell::new(keyword),
            row,
            handle: handle.clone(),
            toggles,
            menu,
            actions,
            updating: Cell::new(false),
        });
        // Esc puts back the keyword's text while it's being changed.
        let held = Rc::downgrade(&item);
        add_escape(&item.row, move || {
            let Some(item) = held.upgrade() else { return false };
            let original = item.keyword.borrow().text.clone();
            if item.row.text() == original.as_str() {
                return false;
            }
            item.row.set_show_apply_button(false);
            item.row.set_text(&original);
            item.row.set_show_apply_button(true);
            true
        });
        item.describe();
        item.set_compact(self.compact.get());
        self.connect_row(&item, index, count);
        self.connect_drag(&item, &handle);
        item
    }

    fn connect_row(self: &Rc<Self>, item: &Rc<KeywordRow>, index: usize, count: usize) {
        let row_action = |name: &str, enabled: bool, run: RowAction| {
            let action = gio::SimpleAction::new(name, None);
            action.set_enabled(enabled);
            let editor = Rc::downgrade(self);
            let row = Rc::downgrade(item);
            action.connect_activate(move |_, _| {
                if let (Some(editor), Some(row)) = (editor.upgrade(), row.upgrade()) {
                    run(&editor, &row);
                }
            });
            item.actions.add_action(&action);
        };
        row_action("move-up", index > 0, Box::new(move |editor, row| editor.move_to(row, index.saturating_sub(1))));
        row_action("move-down", index + 1 < count, Box::new(move |editor, row| editor.move_to(row, index + 1)));
        row_action("remove", true, Box::new(|editor, row| editor.remove(row)));

        let editor = Rc::downgrade(self);
        let row = Rc::downgrade(item);
        item.toggles.connect_active_name_notify(move |toggles| {
            let (Some(editor), Some(row)) = (editor.upgrade(), row.upgrade()) else { return };
            if row.updating.get() {
                return;
            }
            if let Some(weight) = toggles.active_name().and_then(|name| weight_from_name(&name)) {
                editor.set_weight(&row, weight);
            }
        });
        let editor = Rc::downgrade(self);
        let row = Rc::downgrade(item);
        item.row.connect_apply(move |entry| {
            if let (Some(editor), Some(row)) = (editor.upgrade(), row.upgrade()) {
                editor.rename(&row, entry.text().to_string());
            }
        });
    }

    /// Drag a row by its handle onto another row to move it there.
    fn connect_drag(self: &Rc<Self>, item: &Rc<KeywordRow>, handle: &gtk::Image) {
        let source = gtk::DragSource::builder().actions(gdk::DragAction::MOVE).build();
        let row = Rc::downgrade(item);
        source.connect_prepare(move |_, _, _| {
            row.upgrade().map(|row| gdk::ContentProvider::for_value(&row.keyword.borrow().id.to_value()))
        });
        let row = Rc::downgrade(item);
        source.connect_drag_begin(move |source, _| {
            if let Some(row) = row.upgrade() {
                let icon = gtk::WidgetPaintable::new(Some(&row.row));
                source.set_icon(Some(&icon), 0, 0);
            }
        });
        handle.add_controller(source);

        let target = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);
        let editor = Rc::downgrade(self);
        let onto = Rc::downgrade(item);
        target.connect_drop(move |_, value, _, _| {
            let (Some(editor), Some(onto), Ok(dragged)) = (editor.upgrade(), onto.upgrade(), value.get::<String>()) else {
                return false;
            };
            let rows = editor.rows.borrow().clone();
            let target = rows.iter().position(|row| Rc::ptr_eq(row, &onto));
            let moved = rows.iter().find(|row| row.keyword.borrow().id == dragged);
            match (moved, target) {
                (Some(moved), Some(target)) if !Rc::ptr_eq(moved, &onto) => {
                    editor.move_to(moved, target);
                    true
                }
                _ => false,
            }
        });
        item.row.add_controller(target);
    }

    fn add(self: &Rc<Self>, text: String) {
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let Some(mood_id) = self.mood_id.borrow().clone() else { return };
        if text.is_empty() {
            return;
        }
        let key = normalized(&text);
        let existing = self.rows.borrow().iter().find(|row| normalized(&row.keyword.borrow().text) == key).cloned();
        if let Some(existing) = existing {
            self.highlight(&existing);
            // The toast says it; libadwaita announces toasts, so it isn't said a second time here.
            let message = format!("“{}” is already a keyword.", existing.keyword.borrow().text);
            self.toasts.add_toast(crate::window::toast(&message));
            return;
        }
        let Some(app) = self.app.upgrade() else { return };
        let weak = Rc::downgrade(self);
        let active = app.active_mood().is_some_and(|mood| mood.id == mood_id);
        glib::spawn_future_local(async move {
            let added =
                app.change_moods(active, move |engine| engine.add_mood_keyword(mood_id, text, KeywordWeight::Must)).await;
            let Some(this) = weak.upgrade() else { return };
            match added {
                Ok(keyword) => {
                    clear_entry(&this.add_row);
                    this.group.announce(&format!("Added {}, Must.", keyword.text), gtk::AccessibleAnnouncementPriority::Medium);
                }
                Err(error) => {
                    // Empty, too long or one too many: the field is the thing to change.
                    if matches!(error, AutoPaperError::InvalidInput { .. }) {
                        mark_invalid(&this.add_row, true);
                    }
                    this.toast_error(&app, &error);
                }
            }
        });
    }

    fn highlight(&self, row: &Rc<KeywordRow>) {
        row.row.add_css_class("keyword-highlight");
        let widget = row.row.downgrade();
        glib::timeout_add_local_once(Duration::from_secs(3), move || {
            if let Some(widget) = widget.upgrade() {
                widget.remove_css_class("keyword-highlight");
            }
        });
    }

    /// Whether `keyword` belongs to the mood in use (its changes may change what `keywords_are_narrow` says).
    fn edits_active_mood(&self, app: &App) -> bool {
        let mood = self.mood_id.borrow().clone();
        app.active_mood().is_some_and(|active| Some(active.id) == mood)
    }

    fn rename(self: &Rc<Self>, row: &Rc<KeywordRow>, text: String) {
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let (id, old) = {
            let keyword = row.keyword.borrow();
            (keyword.id.clone(), keyword.text.clone())
        };
        if text.is_empty() || text == old {
            row.row.set_text(&old);
            return;
        }
        let key = normalized(&text);
        let clash = self
            .rows
            .borrow()
            .iter()
            .find(|other| !Rc::ptr_eq(other, row) && normalized(&other.keyword.borrow().text) == key)
            .cloned();
        if let Some(clash) = clash {
            self.highlight(&clash);
            self.toasts.add_toast(crate::window::toast(&format!("“{}” is already a keyword.", clash.keyword.borrow().text)));
            row.row.set_text(&old);
            return;
        }
        let Some(app) = self.app.upgrade() else { return };
        let active = self.edits_active_mood(&app);
        let row = row.clone();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            match app.change_moods(active, move |engine| engine.rename_keyword(id, text)).await {
                Ok(keyword) => {
                    row.row.set_text(&keyword.text);
                    row.keyword.replace(keyword);
                    row.describe();
                }
                Err(error) => {
                    row.row.set_text(&old);
                    if let Some(this) = weak.upgrade() {
                        this.toast_error(&app, &error);
                    }
                }
            }
        });
    }

    fn set_weight(self: &Rc<Self>, row: &Rc<KeywordRow>, weight: KeywordWeight) {
        let Some(app) = self.app.upgrade() else { return };
        let id = row.keyword.borrow().id.clone();
        let previous = row.keyword.borrow().weight;
        row.keyword.borrow_mut().weight = weight;
        row.describe();
        // What's shown already has the new weight: re-reading the moods mustn't rebuild the rows for it.
        if let Some(shown) = self.shown.borrow_mut().iter_mut().find(|keyword| keyword.id == id) {
            shown.weight = weight;
        }
        let active = self.edits_active_mood(&app);
        let row = row.clone();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let changed_id = id.clone();
            if let Err(error) = app.change_moods(active, move |engine| engine.set_keyword_weight(id, weight)).await {
                row.keyword.borrow_mut().weight = previous;
                row.updating.set(true);
                row.toggles.set_active_name(Some(weight_name(previous)));
                row.updating.set(false);
                row.describe();
                if let Some(this) = weak.upgrade() {
                    if let Some(shown) = this.shown.borrow_mut().iter_mut().find(|keyword| keyword.id == changed_id) {
                        shown.weight = previous;
                    }
                    this.toast_error(&app, &error);
                }
            }
        });
    }

    fn move_to(self: &Rc<Self>, row: &Rc<KeywordRow>, position: usize) {
        let Some(app) = self.app.upgrade() else { return };
        let id = row.keyword.borrow().id.clone();
        let text = row.keyword.borrow().text.clone();
        let active = self.edits_active_mood(&app);
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let focus = id.clone();
            // The rows are rebuilt in their new order when the moods are read again; focus follows the moved one.
            let moved = app.change_moods(active, move |engine| engine.move_keyword(id, position as u32)).await;
            let Some(this) = weak.upgrade() else { return };
            match moved {
                Ok(()) => {
                    if let Some(row) = this.rows.borrow().iter().find(|row| row.keyword.borrow().id == focus) {
                        row.row.grab_focus();
                    }
                    let message = format!("Moved {text} to position {}.", position + 1);
                    this.group.announce(&message, gtk::AccessibleAnnouncementPriority::Medium);
                }
                Err(error) => this.toast_error(&app, &error),
            }
        });
    }

    fn remove(self: &Rc<Self>, row: &Rc<KeywordRow>) {
        let Some(app) = self.app.upgrade() else { return };
        let keyword = row.keyword.borrow().clone();
        let active = self.edits_active_mood(&app);
        let weak = Rc::downgrade(self);
        // Focus goes to "Add a keyword" before the row goes (it would be left nowhere otherwise).
        self.add_row.grab_focus();
        glib::spawn_future_local(async move {
            let id = keyword.id.clone();
            let removed = app.change_moods(active, move |engine| engine.remove_keyword(id)).await;
            let Some(this) = weak.upgrade() else { return };
            match removed {
                Ok(()) => {
                    this.removed.replace(Some(keyword.clone()));
                    let toast = adw::Toast::builder()
                        .title(format!("Removed “{}”", keyword.text))
                        .use_markup(false)
                        .button_label("Undo")
                        .build();
                    let weak = Rc::downgrade(&this);
                    toast.connect_button_clicked(move |_| {
                        if let Some(this) = weak.upgrade() {
                            this.undo_remove();
                        }
                    });
                    this.toasts.add_toast(toast);
                }
                Err(error) => this.toast_error(&app, &error),
            }
        });
    }

    /// Undo for Remove (the toast's button, or Ctrl+Z): adds the keyword back with its weight, at its old place.
    fn undo_remove(self: &Rc<Self>) {
        let (Some(keyword), Some(mood_id)) = (self.removed.take(), self.mood_id.borrow().clone()) else { return };
        let Some(app) = self.app.upgrade() else { return };
        let active = self.edits_active_mood(&app);
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let text = keyword.text.clone();
            let restored = app
                .change_moods(active, move |engine| {
                    let added = engine.add_mood_keyword(mood_id, keyword.text, keyword.weight)?;
                    engine.move_keyword(added.id.clone(), keyword.position)?;
                    Ok(added)
                })
                .await;
            let Some(this) = weak.upgrade() else { return };
            match restored {
                Ok(added) => {
                    if let Some(row) = this.rows.borrow().iter().find(|row| row.keyword.borrow().id == added.id) {
                        row.row.grab_focus();
                    }
                    this.group.announce(&format!("Restored {text}."), gtk::AccessibleAnnouncementPriority::Medium);
                }
                Err(error) => this.toast_error(&app, &error),
            }
        });
    }

    fn toast_error(&self, app: &Rc<App>, error: &AutoPaperError) {
        let text = strings::error_sentence(error, app.state().settings.fallback);
        self.toasts.add_toast(crate::window::toast(&text));
    }
}

impl KeywordRow {
    /// Very narrow windows: no drag handle; the menu and keyboard still reorder rows.
    fn set_compact(&self, compact: bool) {
        self.handle.set_visible(!compact);
    }

    /// "rain, Must" for the row and its text field; the toggle group and menu name their keyword.
    fn describe(&self) {
        let keyword = self.keyword.borrow();
        let spoken = format!("{}, {}", keyword.text, weight_label(keyword.weight));
        // AdwEntryRow labels itself and its text field by its (empty) title, and a labelled-by relation wins over a
        // label: drop both, so the row and the field speak "rain, Must".
        self.row.reset_relation(gtk::AccessibleRelation::LabelledBy);
        self.row.update_property(&[gtk::accessible::Property::Label(&spoken)]);
        if let Some(text) = self.row.delegate() {
            text.reset_relation(gtk::AccessibleRelation::LabelledBy);
            text.update_property(&[gtk::accessible::Property::Label(&spoken)]);
        }
        self.toggles.update_property(&[gtk::accessible::Property::Label(&format!("Weight of {}", keyword.text))]);
        self.menu.set_tooltip_text(Some(&format!("More for {}", keyword.text)));
        self.menu.update_property(&[gtk::accessible::Property::Label(&format!("More for {}", keyword.text))]);
    }
}
