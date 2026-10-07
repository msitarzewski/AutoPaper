//! History: every wallpaper, newest first, loaded a page at a time (`history_by_mood(filter, mood, limit, offset)`),
//! filtered All · Liked · Disliked · Echoes and by mood (All moods · each mood), in one of two layouts switched in the
//! header bar and remembered (docs/app-spec.md, History): **Grid** and **Gallery**.
//!
//! Grid: each item shows its thumbnail, title, date and badges (icon and text, never colour alone) and speaks
//! `describe(id)` plus its rating. Its actions (wallpapers.rs: Show on desktop, Like, Dislike, Make an echo, Show
//! original and echoes, Show in Files, Delete…) are in an actions button, the context menu (right click, long press,
//! Menu or Shift+F10) and on Enter; Space opens it in the image viewer and Delete deletes it (after asking). Tab reaches
//! the focused item's actions button and then leaves the grid; arrow keys move between items.
//!
//! Gallery (like Files' or Finder's): the chosen wallpaper large, a filmstrip below, and its details beside it (under it
//! in a narrower window) — title, summary, echo note (a link to its original and echoes), mood, keywords used, which
//! models made it, date, Like / Dislike, and every action as a button with Show on desktop first. The filmstrip is one
//! Tab stop: Left and Right choose (and show) a wallpaper, Return shows it on the desktop, Space opens it in the image
//! viewer, Delete deletes it (after asking); the next page loads as the choice nears the end of what's loaded.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use autopaper_core::{Generation, HistoryFilter, Rating};
use gtk::{gdk, gio, glib};

use crate::app::{App, Event};
use crate::runtime::blocking;
use crate::strings;
use crate::wallpapers::{self, Item, ProvenanceLine, WallpaperActions};

const PAGE_SIZE: u32 = 48;
const THUMB_WIDTH: i32 = 224;
const THUMB_HEIGHT: i32 = 126;
const STRIP_WIDTH: i32 = 120;
const STRIP_HEIGHT: i32 = 68;
/// Load the next page once the gallery's choice is this close to the end of what's loaded.
const LOOKAHEAD: u32 = 8;

const FILTERS: [(HistoryFilter, &str, &str); 4] = [
    (HistoryFilter::All, "all", "All"),
    (HistoryFilter::Liked, "liked", "Liked"),
    (HistoryFilter::Disliked, "disliked", "Disliked"),
    (HistoryFilter::Echoes, "echoes", "Echoes"),
];

pub struct HistoryPage {
    root: adw::ToolbarView,
    app: Weak<App>,
    filter: adw::ToggleGroup,
    /// All moods, then each mood (`mood_ids` in the same order; `None` is All moods).
    mood_filter: gtk::DropDown,
    mood_ids: RefCell<Vec<Option<String>>>,
    mood_names: RefCell<Vec<String>>,
    /// Changes to the mood list that the page makes itself don't count as the person choosing.
    updating_moods: Cell<bool>,
    /// Grid | Gallery, in the header bar.
    layout: adw::ToggleGroup,
    store: gio::ListStore,
    grid: gtk::GridView,
    /// grid · gallery · empty.
    views: gtk::Stack,
    empty: adw::StatusPage,
    gallery: Rc<Gallery>,
    loading: Cell<bool>,
    exhausted: Cell<bool>,
    /// Bumped on every reload, so a page that arrives after a newer reload started is dropped.
    epoch: Cell<u64>,
    /// Shown, not loaded yet: loads when the History view is first shown.
    stale: Cell<bool>,
    /// The widgets of each bound grid item, by position (for Enter, Space, Delete and Shift+F10 on the grid).
    bound: RefCell<HashMap<u32, Weak<ItemWidgets>>>,
    /// Where keyboard focus goes after the next refresh: the place of a wallpaper just deleted (its neighbour takes
    /// focus, rather than focus being lost with it).
    focus_after: Cell<Option<u32>>,
    /// The filters the grid was loaded with (new filters start from the top; anything else updates in place).
    loaded_filter: RefCell<Option<(HistoryFilter, Option<String>)>>,
}

impl HistoryPage {
    pub fn new(app: &Rc<App>) -> Rc<Self> {
        let filter = adw::ToggleGroup::builder().halign(gtk::Align::Center).build();
        for (_, name, label) in FILTERS {
            filter.add(adw::Toggle::builder().name(name).label(label).build());
        }
        filter.set_active_name(Some("all"));
        filter.update_property(&[gtk::accessible::Property::Label("Show")]);
        let mood_filter = gtk::DropDown::from_strings(&["All moods"]);
        mood_filter.set_valign(gtk::Align::Center);
        mood_filter.update_property(&[gtk::accessible::Property::Label("Mood")]);
        mood_filter.set_tooltip_text(Some("Show wallpapers from one mood"));
        // The drop-down's focusable part is its button, which screen readers name by the choice it shows ("All
        // moods"): its description says what the choice is for.
        if let Some(button) = mood_filter.first_child() {
            button.update_property(&[gtk::accessible::Property::Description("Show wallpapers from one mood")]);
        }

        // Grid | Gallery: icons with tooltips, named for screen readers; the choice is remembered.
        let layout = adw::ToggleGroup::builder().valign(gtk::Align::Center).css_classes(["flat"]).build();
        layout.add(adw::Toggle::builder().name("grid").icon_name("view-grid-symbolic").tooltip("Grid").label("Grid").build());
        layout.add(
            adw::Toggle::builder().name("gallery").icon_name("view-paged-symbolic").tooltip("Gallery").label("Gallery").build(),
        );
        // Icons only (the toggles keep their labels as accessible names).
        for index in 0..layout.n_toggles() {
            if let Some(toggle) = layout.toggle(index) {
                toggle.set_label(None);
            }
        }
        layout.update_property(&[gtk::accessible::Property::Label("Layout")]);
        let remembered = app.prefs.string("history-layout");
        layout.set_active_name(Some(if remembered == "gallery" { "gallery" } else { "grid" }));

        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let selection = gtk::NoSelection::new(Some(store.clone()));
        let factory = gtk::SignalListItemFactory::new();
        let grid = gtk::GridView::builder()
            .model(&selection)
            .factory(&factory)
            .max_columns(8)
            .min_columns(1)
            .single_click_activate(false)
            .build();
        grid.update_property(&[gtk::accessible::Property::Label("Wallpapers")]);
        // Tab goes to the focused item's actions button and then out of the grid (arrows move between items),
        // instead of two stops per wallpaper.
        grid.set_tab_behavior(gtk::ListTabBehavior::Item);
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&grid)
            .build();

        let gallery = Gallery::new(app, &store);
        let empty = adw::StatusPage::builder().icon_name("document-open-recent-symbolic").vexpand(true).build();
        let views = gtk::Stack::new();
        views.add_named(&scrolled, Some("grid"));
        views.add_named(gallery.widget(), Some("gallery"));
        views.add_named(&empty, Some("empty"));

        // The two filters side by side; in a narrow window the mood list wraps below.
        let bar = adw::WrapBox::builder()
            .child_spacing(12)
            .line_spacing(6)
            .justify(adw::JustifyMode::None)
            .align(0.5)
            .margin_top(12)
            .margin_bottom(6)
            .margin_start(12)
            .margin_end(12)
            .build();
        bar.append(&filter);
        bar.append(&mood_filter);
        let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        content.append(&bar);
        content.append(&views);
        let header = adw::HeaderBar::new();
        header.pack_end(&layout);
        header.pack_end(&crate::window::progress_button(app));
        let root = adw::ToolbarView::new();
        root.add_top_bar(&header);
        root.set_content(Some(&content));

        let this = Rc::new(Self {
            root,
            app: Rc::downgrade(app),
            filter,
            mood_filter,
            mood_ids: RefCell::new(vec![None]),
            mood_names: RefCell::new(vec!["All moods".to_string()]),
            updating_moods: Cell::new(false),
            layout,
            store,
            grid,
            views,
            empty,
            gallery,
            loading: Cell::new(false),
            exhausted: Cell::new(false),
            epoch: Cell::new(0),
            stale: Cell::new(true),
            bound: RefCell::new(HashMap::new()),
            focus_after: Cell::new(None),
            loaded_filter: RefCell::new(None),
        });
        this.gallery.page.replace(Rc::downgrade(&this));
        this.setup_factory(&factory);
        this.connect_signals(&scrolled);
        let weak = Rc::downgrade(&this);
        app.subscribe(move |event| {
            let Some(this) = weak.upgrade() else { return false };
            if matches!(event, Event::Ready | Event::Moods) {
                this.show_moods();
            }
            if matches!(event, Event::Ready | Event::History) {
                if this.root.is_mapped() {
                    this.refresh();
                } else {
                    this.stale.set(true);
                }
            }
            true
        });
        if app.is_ready() {
            this.show_moods();
        }
        this
    }

    pub fn widget(&self) -> &adw::ToolbarView {
        &self.root
    }

    /// A narrower window: the gallery's details go under the picture.
    pub fn set_stacked(&self, stacked: bool) {
        self.gallery.set_stacked(stacked);
    }

    /// The mood list: All moods, then each mood in the person's order. The mood filtered on stays chosen; if it was
    /// deleted, the filter goes back to All moods.
    fn show_moods(self: &Rc<Self>) {
        let Some(app) = self.app.upgrade() else { return };
        let moods = app.state().moods.clone();
        let chosen = self.chosen_mood();
        let mut ids = vec![None];
        let mut names = vec!["All moods".to_string()];
        for mood in &moods {
            ids.push(Some(mood.id.clone()));
            names.push(mood.name.clone());
        }
        let position = ids.iter().position(|id| *id == chosen).unwrap_or(0);
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let same = *self.mood_ids.borrow() == ids && *self.mood_names.borrow() == names;
        if !same {
            self.mood_names.replace(names.clone());
            self.updating_moods.set(true);
            self.mood_filter.set_model(Some(&gtk::StringList::new(&refs)));
            self.mood_ids.replace(ids);
            self.mood_filter.set_selected(position as u32);
            self.updating_moods.set(false);
            if position == 0 && chosen.is_some() {
                // The mood filtered on is gone: show everything.
                self.reload();
            }
        }
    }

    /// The mood History is filtered to (`None`: all moods).
    fn chosen_mood(&self) -> Option<String> {
        self.mood_ids.borrow().get(self.mood_filter.selected() as usize).cloned().flatten()
    }

    /// Filters History to one mood's wallpapers (`None`: all moods), as a mood's "Show in History" asks.
    pub fn show_mood(self: &Rc<Self>, mood_id: Option<String>) {
        let position = self.mood_ids.borrow().iter().position(|id| *id == mood_id).unwrap_or(0);
        self.filter.set_active_name(Some("all"));
        if self.mood_filter.selected() as usize != position {
            self.mood_filter.set_selected(position as u32);
        } else {
            self.reload();
        }
    }

    fn gallery_shown(&self) -> bool {
        self.layout.active_name().as_deref() == Some("gallery")
    }

    fn connect_signals(self: &Rc<Self>, scrolled: &gtk::ScrolledWindow) {
        let weak = Rc::downgrade(self);
        self.root.connect_map(move |_| {
            if let Some(this) = weak.upgrade()
                && this.stale.get()
            {
                this.refresh();
            }
        });
        let weak = Rc::downgrade(self);
        self.filter.connect_active_name_notify(move |_| {
            if let Some(this) = weak.upgrade() {
                this.reload();
            }
        });
        let weak = Rc::downgrade(self);
        self.mood_filter.connect_selected_notify(move |_| {
            if let Some(this) = weak.upgrade()
                && !this.updating_moods.get()
            {
                this.reload();
            }
        });
        let weak = Rc::downgrade(self);
        self.layout.connect_active_name_notify(move |layout| {
            let Some(this) = weak.upgrade() else { return };
            let name = layout.active_name().unwrap_or_else(|| "grid".into());
            if let Some(app) = this.app.upgrade() {
                let _ = app.prefs.set_string("history-layout", &name);
            }
            // The gallery opens on the wallpaper the grid had focus on.
            if name == "gallery"
                && let Some(position) = this.focused_position()
            {
                this.gallery.select(position);
            }
            this.show_view();
        });
        // When everything loaded fits without scrolling (a tall window), the bottom edge is never reached: load the
        // next page as soon as the grid is laid out and there's room for more.
        let weak = Rc::downgrade(self);
        scrolled.vadjustment().connect_changed(move |adjustment| {
            if let Some(this) = weak.upgrade()
                && this.store.n_items() > 0
                && !this.gallery_shown()
                && adjustment.upper() <= adjustment.page_size() + 1.0
            {
                this.load_more();
            }
        });
        let weak = Rc::downgrade(self);
        scrolled.connect_edge_reached(move |_, edge| {
            if edge == gtk::PositionType::Bottom
                && let Some(this) = weak.upgrade()
            {
                this.load_more();
            }
        });
        // Enter (and double click) opens the item's actions.
        let weak = Rc::downgrade(self);
        self.grid.connect_activate(move |_, position| {
            if let Some(widgets) = weak.upgrade().and_then(|this| this.widgets_at(position)) {
                widgets.menu_button.popup();
            }
        });
        // Shift+F10 / Menu: the focused item's context menu; Space: open it in the image viewer; Delete: delete it.
        let shortcuts = gtk::ShortcutController::new();
        let on_focused = |run: fn(&Rc<HistoryPage>, &Rc<ItemWidgets>)| {
            let weak = Rc::downgrade(self);
            gtk::CallbackAction::new(move |_, _| {
                let Some(this) = weak.upgrade() else { return glib::Propagation::Proceed };
                match this.focused_position().and_then(|position| this.widgets_at(position)) {
                    Some(widgets) => {
                        run(&this, &widgets);
                        glib::Propagation::Stop
                    }
                    None => glib::Propagation::Proceed,
                }
            })
        };
        let menu = on_focused(|_, widgets| widgets.menu_button.popup());
        for keys in ["<Shift>F10", "Menu"] {
            if let Some(trigger) = gtk::ShortcutTrigger::parse_string(keys) {
                shortcuts.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(menu.clone())));
            }
        }
        let preview = on_focused(|this, widgets| {
            if let (Some(app), Some(generation)) = (this.app.upgrade(), widgets.actions.generation()) {
                wallpapers::preview(&app, &generation);
            }
        });
        if let Some(trigger) = gtk::ShortcutTrigger::parse_string("space") {
            shortcuts.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(preview)));
        }
        let delete = on_focused(|_, widgets| {
            widgets.actions.activate("delete");
        });
        if let Some(trigger) = gtk::ShortcutTrigger::parse_string("Delete") {
            shortcuts.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(delete)));
        }
        self.grid.add_controller(shortcuts);
    }

    fn widgets_at(&self, position: u32) -> Option<Rc<ItemWidgets>> {
        self.bound.borrow().get(&position).and_then(Weak::upgrade)
    }

    fn current_filter(&self) -> HistoryFilter {
        let name = self.filter.active_name();
        FILTERS
            .iter()
            .find(|(_, n, _)| Some(*n) == name.as_deref())
            .map(|(filter, _, _)| *filter)
            .unwrap_or(HistoryFilter::All)
    }

    /// Loads the first page afresh (another filter: the grid starts from the top).
    fn reload(self: &Rc<Self>) {
        self.fetch(0, PAGE_SIZE, Load::Replace);
    }

    /// Something in History changed (a rating, a delete, a new wallpaper, one shown again): re-reads everything
    /// loaded so far and updates it in place, so the scroll position, keyboard focus and the gallery's choice stay on
    /// the wallpaper the person was on, and pages loaded further down stay loaded.
    fn refresh(self: &Rc<Self>) {
        let filters = Some((self.current_filter(), self.chosen_mood()));
        if self.store.n_items() == 0 || *self.loaded_filter.borrow() != filters {
            self.reload();
            return;
        }
        let count = self.store.n_items().max(PAGE_SIZE);
        self.fetch(0, count, Load::Update);
    }

    fn load_more(self: &Rc<Self>) {
        if self.loading.get() || self.exhausted.get() {
            return;
        }
        self.fetch(self.store.n_items(), PAGE_SIZE, Load::Append);
    }

    /// The gallery's choice moved to `position`: the next page loads in time.
    fn chosen(self: &Rc<Self>, position: u32) {
        if position + LOOKAHEAD >= self.store.n_items() {
            self.load_more();
        }
    }

    fn fetch(self: &Rc<Self>, offset: u32, limit: u32, load: Load) {
        let Some(engine) = self.app.upgrade().and_then(|app| app.engine()) else { return };
        if load != Load::Append {
            // A newer full read supersedes any page still on its way.
            self.stale.set(false);
            self.epoch.set(self.epoch.get() + 1);
        }
        self.loading.set(true);
        let epoch = self.epoch.get();
        let filter = self.current_filter();
        let mood = self.chosen_mood();
        let asked_mood = mood.clone();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let page = blocking(move || {
                let generations = engine.history_by_mood(filter, asked_mood, limit, offset)?;
                Ok(generations.into_iter().map(|generation| Item::read(&engine, generation)).collect::<Vec<_>>())
            })
            .await;
            let Some(this) = weak.upgrade() else { return };
            if this.epoch.get() != epoch {
                return;
            }
            this.loading.set(false);
            match page {
                Ok(items) => {
                    this.exhausted.set((items.len() as u32) < limit);
                    match load {
                        Load::Replace => {
                            let objects: Vec<glib::BoxedAnyObject> =
                                items.into_iter().map(glib::BoxedAnyObject::new).collect();
                            this.store.splice(0, this.store.n_items(), &objects);
                            this.gallery.select(0);
                        }
                        Load::Append => {
                            let objects: Vec<glib::BoxedAnyObject> =
                                items.into_iter().map(glib::BoxedAnyObject::new).collect();
                            this.store.extend_from_slice(&objects);
                        }
                        Load::Update => this.update_in_place(items),
                    }
                    this.loaded_filter.replace(Some((filter, mood)));
                    this.show_view();
                    this.gallery.show_chosen();
                }
                Err(error) => tracing::warn!(%error, "couldn't read the history"),
            }
        });
    }

    fn item_at(&self, position: u32) -> Option<glib::BoxedAnyObject> {
        self.store.item(position).and_downcast::<glib::BoxedAnyObject>()
    }

    fn id_at(&self, position: u32) -> Option<String> {
        self.item_at(position).map(|object| object.borrow::<Item>().generation.id.clone())
    }

    /// The grid item holding keyboard focus, when focus is in the grid.
    fn focused_position(&self) -> Option<u32> {
        let focus = gtk::prelude::RootExt::focus(&self.grid.root()?)?;
        if focus != *self.grid.upcast_ref::<gtk::Widget>() && !focus.is_ancestor(&self.grid) {
            return None;
        }
        let focused_child = self.grid.focus_child()?;
        self.bound
            .borrow()
            .iter()
            .find(|(_, widgets)| widgets.upgrade().is_some_and(|widgets| widgets.root.parent().as_ref() == Some(&focused_child)))
            .map(|(position, _)| *position)
    }

    /// Applies a fresh read of the loaded items without rebuilding the grid: items that changed are re-bound
    /// where they are (same objects, so GTK keeps focus on them), removed ones are taken out and new ones put in.
    /// Keyboard focus stays on its wallpaper; if that one is gone (deleted, or no longer matching the filter), its
    /// neighbour takes it. The gallery keeps its choice the same way.
    fn update_in_place(self: &Rc<Self>, items: Vec<Item>) {
        let focused = self.focused_position();
        let focused_id = focused.and_then(|position| self.id_at(position));
        let chosen = self.gallery.chosen_position();
        let chosen_id = chosen.and_then(|position| self.id_at(position));
        let old_ids: Vec<String> = (0..self.store.n_items()).filter_map(|position| self.id_at(position)).collect();
        let new_ids: Vec<&str> = items.iter().map(|item| item.generation.id.as_str()).collect();
        let (old_set, new_set): (HashSet<&str>, HashSet<&str>) =
            (old_ids.iter().map(String::as_str).collect(), new_ids.iter().copied().collect());
        let kept_old: Vec<&str> = old_ids.iter().map(String::as_str).filter(|id| new_set.contains(id)).collect();
        let kept_new: Vec<&str> = new_ids.iter().copied().filter(|id| old_set.contains(id)).collect();

        if kept_old != kept_new {
            // History is newest first and never reorders, so this shouldn't happen; replace everything if it does.
            let objects: Vec<glib::BoxedAnyObject> = items.into_iter().map(glib::BoxedAnyObject::new).collect();
            self.store.splice(0, self.store.n_items(), &objects);
        } else {
            for position in (0..old_ids.len()).rev() {
                if !new_set.contains(old_ids[position].as_str()) {
                    self.store.remove(position as u32);
                }
            }
            for (position, item) in items.into_iter().enumerate() {
                let position = position as u32;
                let existing = self
                    .item_at(position)
                    .filter(|object| object.borrow::<Item>().generation.id == item.generation.id);
                match existing {
                    Some(object) => {
                        let changed = *object.borrow::<Item>() != item;
                        if changed {
                            let _previous = object.replace(item.clone());
                            self.rebind(&item);
                            self.gallery.rebind(&item);
                        }
                    }
                    None => self.store.insert(position, &glib::BoxedAnyObject::new(item)),
                }
            }
        }

        // The gallery's choice: the same wallpaper, else the one now in its place.
        let find = |id: &str| (0..self.store.n_items()).find(|position| self.id_at(*position).as_deref() == Some(id));
        if let Some(chosen) = chosen {
            let position = chosen_id.as_deref().and_then(find).unwrap_or(chosen);
            self.gallery.select(position.min(self.store.n_items().saturating_sub(1)));
        }

        // Where focus should be now: the wallpaper that had it, or the place of one just deleted or gone.
        let target = match (self.focus_after.take(), focused_id) {
            (Some(position), _) => Some(position),
            (None, Some(id)) => match find(&id) {
                Some(position) if self.focused_position() == Some(position) => None,
                Some(position) => Some(position),
                None => focused,
            },
            (None, None) => None,
        };
        if let Some(position) = target
            && self.store.n_items() > 0
            && !self.gallery_shown()
        {
            let position = position.min(self.store.n_items() - 1);
            self.grid.scroll_to(position, gtk::ListScrollFlags::FOCUS, None);
        }
    }

    /// Shows a changed item's new state in the grid widgets bound to it (badges, menu checkmarks, spoken name).
    fn rebind(self: &Rc<Self>, item: &Item) {
        let bound: Vec<Rc<ItemWidgets>> = self.bound.borrow().values().filter_map(Weak::upgrade).collect();
        for widgets in bound {
            if widgets.id.borrow().as_deref() == Some(item.generation.id.as_str()) {
                if let Some(list_item) = widgets.list_item.upgrade() {
                    list_item.set_accessible_label(&item.spoken);
                }
                widgets.bind(item.clone());
            }
        }
    }

    /// The grid, the gallery, or the empty page (whatever the layout, when nothing matches).
    fn show_view(&self) {
        if self.store.n_items() > 0 {
            self.views.set_visible_child_name(if self.gallery_shown() { "gallery" } else { "grid" });
            return;
        }
        let filtered_to_mood = self.chosen_mood().is_some();
        let (title, description) = match self.current_filter() {
            HistoryFilter::All if filtered_to_mood => {
                ("Nothing made in this mood yet", "Wallpapers made while this mood is in use appear here.")
            }
            HistoryFilter::All => ("No wallpapers yet", "Wallpapers appear here as AutoPaper makes them."),
            HistoryFilter::Liked => ("Nothing liked yet", "Wallpapers you like appear here."),
            HistoryFilter::Disliked => ("Nothing disliked", "Wallpapers you dislike appear here."),
            HistoryFilter::Echoes => {
                ("No echoes yet", "When an old idea comes back in a new form, it appears here with its original.")
            }
        };
        self.empty.set_title(title);
        self.empty.set_description(Some(description));
        self.views.set_visible_child_name("empty");
    }

    fn setup_factory(self: &Rc<Self>, factory: &gtk::SignalListItemFactory) {
        let weak = Rc::downgrade(self);
        factory.connect_setup(move |_, object| {
            let Some(this) = weak.upgrade() else { return };
            let Some(list_item) = object.downcast_ref::<gtk::ListItem>() else { return };
            let Some(app) = this.app.upgrade() else { return };
            let widgets = ItemWidgets::new(&app, &this);
            list_item.set_child(Some(&widgets.root));
            widgets.list_item.set(Some(list_item));
            let page = Rc::downgrade(&this);
            let held = widgets.clone();
            // Rebinds as the grid recycles the item for another wallpaper.
            list_item.connect_item_notify(move |list_item| {
                let Some(page) = page.upgrade() else { return };
                let item = list_item.item().and_downcast::<glib::BoxedAnyObject>();
                match item {
                    Some(object) => {
                        let item = object.borrow::<Item>().clone();
                        list_item.set_accessible_label(&item.spoken);
                        held.bind(item);
                        page.bound.borrow_mut().insert(list_item.position(), Rc::downgrade(&held));
                    }
                    None => {
                        held.unbind();
                        page.bound.borrow_mut().retain(|_, widgets| widgets.upgrade().is_some_and(|w| !Rc::ptr_eq(&w, &held)));
                    }
                }
            });
            list_item.connect_position_notify({
                let page = Rc::downgrade(&this);
                let held = Rc::downgrade(&widgets);
                move |list_item| {
                    if let Some(page) = page.upgrade() {
                        let mut bound = page.bound.borrow_mut();
                        bound.retain(|_, widgets| !widgets.ptr_eq(&held));
                        if list_item.item().is_some() {
                            bound.insert(list_item.position(), held.clone());
                        }
                    }
                }
            });
        });
    }

    /// Before a wallpaper is deleted from History: its neighbour takes keyboard focus once the grid has updated (focus
    /// would go with it otherwise).
    fn delete_hook(self: &Rc<Self>) -> wallpapers::DeleteHook {
        let weak = Rc::downgrade(self);
        Rc::new(move |generation: &Generation, done: Option<bool>| {
            let Some(page) = weak.upgrade() else { return };
            match done {
                None => {
                    let position = (0..page.store.n_items()).find(|p| page.id_at(*p).as_deref() == Some(generation.id.as_str()));
                    page.focus_after.set(position);
                }
                Some(false) => page.focus_after.set(None),
                Some(true) => {}
            }
        })
    }
}

/// How a page of History goes into the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Load {
    /// The first page, replacing everything (another filter).
    Replace,
    /// The next page, at the end.
    Append,
    /// Everything loaded so far, read again and applied in place.
    Update,
}

/// The widgets of one grid item, reused as the grid recycles it.
struct ItemWidgets {
    root: gtk::Box,
    picture: gtk::Picture,
    title: gtk::Label,
    date: gtk::Label,
    liked: gtk::Box,
    disliked: gtk::Box,
    echo: gtk::Box,
    menu_button: gtk::MenuButton,
    actions: Rc<WallpaperActions>,
    id: Rc<RefCell<Option<String>>>,
    /// The grid's item these widgets belong to (its accessible name is the wallpaper's description).
    list_item: glib::WeakRef<gtk::ListItem>,
}

impl ItemWidgets {
    fn new(app: &Rc<App>, page: &Rc<HistoryPage>) -> Rc<Self> {
        // The item itself speaks the description; the picture is decorative inside it. Every item is one
        // fixed-width column (thumbnail, title, date, badges aligned), centred in its grid cell.
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Cover)
            .width_request(THUMB_WIDTH)
            .height_request(THUMB_HEIGHT)
            .css_classes(["history-thumb"])
            .overflow(gtk::Overflow::Hidden)
            .can_shrink(true)
            .accessible_role(gtk::AccessibleRole::Presentation)
            .build();
        let menu_button = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .menu_model(&wallpapers::item_menu())
            .halign(gtk::Align::End)
            .valign(gtk::Align::Start)
            .margin_top(6)
            .margin_end(6)
            .css_classes(["circular", "thumb-actions"])
            .tooltip_text("Actions")
            .build();
        // A picture asks for its image's own size; clamps hold it to the thumbnail size.
        let picture_height =
            adw::Clamp::builder().orientation(gtk::Orientation::Vertical).maximum_size(THUMB_HEIGHT).child(&picture).build();
        let overlay = gtk::Overlay::builder().child(&picture_height).build();
        overlay.add_overlay(&menu_button);

        let title = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(24)
            .css_classes(["heading"])
            .build();
        let date = gtk::Label::builder().xalign(0.0).css_classes(["secondary-text", "caption"]).build();
        let liked = badge("autopaper-like-symbolic", "Liked");
        let disliked = badge("autopaper-dislike-symbolic", "Disliked");
        let echo = badge("media-playlist-repeat-symbolic", "Echo");
        let badges = gtk::Box::builder().spacing(6).build();
        badges.append(&liked);
        badges.append(&disliked);
        badges.append(&echo);

        let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).build();
        column.append(&overlay);
        column.append(&title);
        column.append(&date);
        column.append(&badges);
        let item = adw::Clamp::builder().maximum_size(THUMB_WIDTH).tightening_threshold(THUMB_WIDTH).child(&column).build();
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .valign(gtk::Align::Start)
            .css_classes(["history-item"])
            .build();
        root.append(&item);
        let actions = WallpaperActions::new(app, &root);
        actions.on_delete(page.delete_hook());
        wallpapers::attach_menu(&root, &wallpapers::item_menu());
        Rc::new(Self {
            root,
            picture,
            title,
            date,
            liked,
            disliked,
            echo,
            menu_button,
            actions,
            id: Rc::new(RefCell::new(None)),
            list_item: glib::WeakRef::new(),
        })
    }

    fn bind(&self, item: Item) {
        let generation = &item.generation;
        self.actions.set(Some(generation), item.has_echoes);
        self.title.set_label(&generation.concept.title);
        self.date.set_label(&strings::day_label(generation.created_at));
        self.liked.set_visible(generation.rating == Rating::Liked);
        self.disliked.set_visible(generation.rating == Rating::Disliked);
        self.echo.set_visible(generation.echo_of.is_some());
        let title = &generation.concept.title;
        self.menu_button.set_tooltip_text(Some(&format!("Actions for {title}")));
        self.menu_button.update_property(&[gtk::accessible::Property::Label(&format!("Actions for {title}"))]);
        self.id.replace(Some(generation.id.clone()));
        let (id, wanted) = (self.id.clone(), generation.id.clone());
        // Still showing the same wallpaper once decoded (the grid recycles items while scrolling)?
        wallpapers::show_picture(&self.picture, wallpapers::thumb_path(generation), move || {
            id.borrow().as_deref() == Some(wanted.as_str())
        });
    }

    fn unbind(&self) {
        self.id.replace(None);
        self.actions.set(None, false);
        self.picture.set_paintable(None::<&gdk::Paintable>);
    }
}

// ── Gallery ─────────────────────────────────────────────────────────────────────────────────────────────────────

/// The gallery: the chosen wallpaper large, the filmstrip (the same items as the grid, with a single selection), and
/// the chosen one's details and actions.
struct Gallery {
    root: gtk::Box,
    page: RefCell<Weak<HistoryPage>>,
    app: Weak<App>,
    selection: gtk::SingleSelection,
    strip: gtk::ListView,
    picture: gtk::Picture,
    /// Holds the picture to a height when the details are under it.
    picture_clamp: adw::Clamp,
    viewer: gtk::Box,
    details_scroller: gtk::ScrolledWindow,
    title: gtk::Label,
    summary: gtk::Label,
    echo: gtk::LinkButton,
    facts: gtk::Label,
    keywords: gtk::Label,
    provenance: ProvenanceLine,
    /// Show original and echoes: there only when the wallpaper has an original or echoes (as in the menu).
    lineage_button: gtk::Button,
    actions: Rc<WallpaperActions>,
    /// The picture shown is this wallpaper's (decoding finishes after the choice may have moved on).
    shown: Rc<RefCell<Option<String>>>,
}

impl Gallery {
    fn new(app: &Rc<App>, store: &gio::ListStore) -> Rc<Self> {
        let selection = gtk::SingleSelection::builder().model(store).autoselect(true).can_unselect(false).build();
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, object| {
            let Some(list_item) = object.downcast_ref::<gtk::ListItem>() else { return };
            let picture = gtk::Picture::builder()
                .content_fit(gtk::ContentFit::Cover)
                .width_request(STRIP_WIDTH)
                .height_request(STRIP_HEIGHT)
                .can_shrink(true)
                .css_classes(["history-thumb"])
                .overflow(gtk::Overflow::Hidden)
                .accessible_role(gtk::AccessibleRole::Presentation)
                .build();
            let tall = adw::Clamp::builder().orientation(gtk::Orientation::Vertical).maximum_size(STRIP_HEIGHT).child(&picture).build();
            let wide = adw::Clamp::builder().maximum_size(STRIP_WIDTH).tightening_threshold(STRIP_WIDTH).child(&tall).build();
            list_item.set_child(Some(&wide));
            let id: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
            list_item.connect_item_notify(move |list_item| {
                match list_item.item().and_downcast::<glib::BoxedAnyObject>() {
                    Some(object) => {
                        let item = object.borrow::<Item>().clone();
                        list_item.set_accessible_label(&item.spoken);
                        id.replace(Some(item.generation.id.clone()));
                        let (id, wanted) = (id.clone(), item.generation.id.clone());
                        wallpapers::show_picture(&picture, wallpapers::thumb_path(&item.generation), move || {
                            id.borrow().as_deref() == Some(wanted.as_str())
                        });
                    }
                    None => {
                        id.replace(None);
                        picture.set_paintable(None::<&gdk::Paintable>);
                    }
                }
            });
        });
        let strip = gtk::ListView::builder()
            .model(&selection)
            .factory(&factory)
            .orientation(gtk::Orientation::Horizontal)
            .single_click_activate(false)
            .css_classes(["filmstrip"])
            .build();
        strip.update_property(&[
            gtk::accessible::Property::Label("Wallpapers"),
            gtk::accessible::Property::Description(
                "Left and right arrows choose a wallpaper. Return shows it on the desktop, Space opens it in the image \
                 viewer, and Delete deletes it.",
            ),
        ]);
        strip.set_tab_behavior(gtk::ListTabBehavior::Item);
        let strip_scroller = gtk::ScrolledWindow::builder()
            .vscrollbar_policy(gtk::PolicyType::Never)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .height_request(STRIP_HEIGHT + 34)
            .child(&strip)
            .build();

        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Contain)
            .can_shrink(true)
            .vexpand(true)
            .hexpand(true)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .css_classes(["gallery-picture"])
            .build();
        let picture_clamp = adw::Clamp::builder()
            .orientation(gtk::Orientation::Vertical)
            .maximum_size(4000)
            .vexpand(true)
            .child(&picture)
            .build();
        let viewer = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).build();
        viewer.append(&picture_clamp);
        viewer.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        viewer.append(&strip_scroller);

        // The summary is selectable text (as on Now); the other details are plain labels, so Tab doesn't stop on each.
        let title = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["title-3"]).build();
        let summary = gtk::Label::builder().xalign(0.0).wrap(true).selectable(true).build();
        // The echo note links to the original and its echoes.
        let echo = gtk::LinkButton::builder().uri("autopaper:lineage").halign(gtk::Align::Start).build();
        echo.update_property(&[gtk::accessible::Property::Description("Shows the original and its echoes")]);
        let facts = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["secondary-text"]).build();
        let keywords = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["secondary-text"]).build();
        let provenance = ProvenanceLine::new(app);
        let like = gallery_toggle("autopaper-like-symbolic", "_Like", "item.like");
        let dislike = gallery_toggle("autopaper-dislike-symbolic", "_Dislike", "item.dislike");
        let ratings = gtk::Box::builder().spacing(8).build();
        ratings.append(&like);
        ratings.append(&dislike);
        let buttons = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build();
        let mut lineage_button = None;
        for (label, action, suggested) in [
            ("_Show on desktop", "item.show", true),
            ("Make an _echo", "item.echo", false),
            ("Show _original and echoes", "item.lineage", false),
            ("Show in _Files", "item.open-folder", false),
            ("_Delete…", "item.delete", false),
        ] {
            let button = gtk::Button::builder().label(label).use_underline(true).action_name(action).halign(gtk::Align::Start).build();
            if suggested {
                button.add_css_class("suggested-action");
            }
            if action == "item.delete" {
                button.add_css_class("destructive-action");
            }
            if action == "item.lineage" {
                lineage_button = Some(button.clone());
            }
            buttons.append(&button);
        }
        let lineage_button = lineage_button.expect("the lineage button is in the list");
        let details = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .margin_top(18)
            .margin_bottom(18)
            .margin_start(18)
            .margin_end(18)
            .build();
        details.append(&title);
        details.append(&summary);
        details.append(&echo);
        details.append(&facts);
        details.append(&keywords);
        details.append(provenance.widget());
        details.append(&ratings);
        details.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        details.append(&buttons);
        let details_scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .width_request(300)
            .child(&details)
            .build();
        details_scroller.update_property(&[gtk::accessible::Property::Label("Details")]);

        let root = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).vexpand(true).build();
        root.append(&viewer);
        root.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        root.append(&details_scroller);
        let actions = WallpaperActions::new(app, &root);

        let this = Rc::new(Self {
            root,
            page: RefCell::new(Weak::new()),
            app: Rc::downgrade(app),
            selection,
            strip,
            picture,
            picture_clamp,
            viewer,
            details_scroller,
            title,
            summary,
            echo,
            facts,
            keywords,
            provenance,
            lineage_button,
            actions,
            shown: Rc::new(RefCell::new(None)),
        });
        this.connect();
        this
    }

    fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Side by side (wide) or the details under the picture (narrower windows).
    fn set_stacked(&self, stacked: bool) {
        self.root.set_orientation(if stacked { gtk::Orientation::Vertical } else { gtk::Orientation::Horizontal });
        if let Some(separator) = self.viewer.next_sibling().and_downcast::<gtk::Separator>() {
            separator.set_orientation(if stacked { gtk::Orientation::Horizontal } else { gtk::Orientation::Vertical });
        }
        self.details_scroller.set_width_request(if stacked { -1 } else { 300 });
        self.details_scroller.set_vexpand(stacked);
        self.details_scroller.set_min_content_height(if stacked { 200 } else { -1 });
        // Under the picture, the details need the room: the picture keeps a modest height.
        self.picture_clamp.set_maximum_size(if stacked { 240 } else { 4000 });
        self.picture_clamp.set_tightening_threshold(if stacked { 240 } else { 4000 });
        self.picture_clamp.set_vexpand(!stacked);
    }

    fn chosen_position(&self) -> Option<u32> {
        let position = self.selection.selected();
        (position != gtk::INVALID_LIST_POSITION).then_some(position)
    }

    fn select(&self, position: u32) {
        if position < self.selection.n_items() {
            self.selection.set_selected(position);
        }
    }

    fn connect(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.selection.connect_selected_notify(move |selection| {
            let Some(this) = weak.upgrade() else { return };
            this.show_chosen();
            let position = selection.selected();
            if position != gtk::INVALID_LIST_POSITION
                && let Some(page) = this.page.borrow().upgrade()
            {
                page.chosen(position);
            }
        });
        // Return on the filmstrip (or a double click): Show on desktop.
        let weak = Rc::downgrade(self);
        self.strip.connect_activate(move |_, position| {
            let Some(this) = weak.upgrade() else { return };
            this.select(position);
            this.actions.activate("show");
        });
        let weak = Rc::downgrade(self);
        self.echo.connect_activate_link(move |_| {
            if let Some(this) = weak.upgrade() {
                this.actions.activate("lineage");
            }
            glib::Propagation::Stop
        });
        // Space: the image viewer; Delete: Delete…. Caught before the list's own Space (which would select).
        let shortcuts = gtk::ShortcutController::new();
        shortcuts.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        let preview = gtk::CallbackAction::new(move |_, _| {
            let Some(this) = weak.upgrade() else { return glib::Propagation::Proceed };
            match (this.app.upgrade(), this.actions.generation()) {
                (Some(app), Some(generation)) => {
                    wallpapers::preview(&app, &generation);
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        if let Some(trigger) = gtk::ShortcutTrigger::parse_string("space") {
            shortcuts.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(preview)));
        }
        let weak = Rc::downgrade(self);
        let delete = gtk::CallbackAction::new(move |_, _| {
            let Some(this) = weak.upgrade() else { return glib::Propagation::Proceed };
            if this.actions.activate("delete") { glib::Propagation::Stop } else { glib::Propagation::Proceed }
        });
        if let Some(trigger) = gtk::ShortcutTrigger::parse_string("Delete") {
            shortcuts.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(delete)));
        }
        self.strip.add_controller(shortcuts);
    }

    fn chosen_item(&self) -> Option<Item> {
        self.selection.selected_item().and_downcast::<glib::BoxedAnyObject>().map(|object| object.borrow::<Item>().clone())
    }

    /// An item changed in place (a rating): the details follow if it's the one chosen.
    fn rebind(&self, item: &Item) {
        if self.chosen_item().is_some_and(|chosen| chosen.generation.id == item.generation.id) {
            self.show_item(item);
        }
    }

    /// Shows the chosen wallpaper (or nothing, when History is empty).
    fn show_chosen(&self) {
        match self.chosen_item() {
            Some(item) => self.show_item(&item),
            None => {
                self.actions.set(None, false);
                self.picture.set_paintable(None::<&gdk::Paintable>);
                self.shown.replace(None);
            }
        }
    }

    fn show_item(&self, item: &Item) {
        let generation = &item.generation;
        self.actions.set(Some(generation), item.has_echoes);
        if !item.has_echoes && self.lineage_button.has_focus() {
            self.lineage_button.child_focus(gtk::DirectionType::TabForward);
        }
        self.lineage_button.set_visible(item.has_echoes);
        self.title.set_label(&generation.concept.title);
        self.summary.set_label(&generation.concept.summary);
        match generation.echo_note.as_deref().filter(|note| !note.is_empty()) {
            Some(note) => {
                self.echo.set_label(note);
                self.echo.set_visible(true);
            }
            None => self.echo.set_visible(false),
        }
        let mood = match (&generation.mood_name, &generation.mood_id) {
            (Some(name), _) => format!("Mood: {name}"),
            (None, Some(_)) => "Mood: one since deleted".to_string(),
            (None, None) => String::new(),
        };
        let mut facts = vec![format!("Made {}", strings::day_label(generation.created_at))];
        if !mood.is_empty() {
            facts.push(mood);
        }
        if let Some(rating) = strings::rating_words(generation.rating) {
            facts.push(rating.to_string());
        }
        self.facts.set_label(&facts.join("\n"));
        let groups = strings::keyword_groups(generation.keywords.iter().map(|keyword| (keyword.text.as_str(), keyword.weight)));
        self.keywords.set_label(&groups.join("\n"));
        self.keywords.set_visible(!groups.is_empty());
        self.provenance.set(generation);
        self.picture.set_alternative_text(Some(&item.spoken));
        self.picture.update_property(&[gtk::accessible::Property::Label(&item.spoken)]);
        if self.shown.borrow().as_deref() != Some(generation.id.as_str()) {
            self.shown.replace(Some(generation.id.clone()));
            let (shown, wanted) = (self.shown.clone(), generation.id.clone());
            // The original (larger) when it's still there, else the thumbnail.
            wallpapers::show_picture(&self.picture, wallpapers::large_path(generation), move || {
                shown.borrow().as_deref() == Some(wanted.as_str())
            });
        }
    }
}

/// Like / Dislike in the gallery: toggle buttons on the item's stateful actions (their state is the rating).
fn gallery_toggle(icon: &str, label: &str, action: &str) -> gtk::ToggleButton {
    let content = adw::ButtonContent::builder().icon_name(icon).label(label).use_underline(true).build();
    gtk::ToggleButton::builder().child(&content).action_name(action).build()
}

/// A badge: icon and text, so it never relies on colour.
pub fn badge(icon: &str, text: &str) -> gtk::Box {
    let badge = gtk::Box::builder().spacing(4).css_classes(["badge"]).visible(false).build();
    badge.append(&decorative_icon(icon));
    badge.append(&gtk::Label::new(Some(text)));
    badge
}

/// An icon next to text that already says what it means: hidden from screen readers.
pub fn decorative_icon(name: &str) -> gtk::Image {
    gtk::Image::builder().icon_name(name).accessible_role(gtk::AccessibleRole::Presentation).build()
}
