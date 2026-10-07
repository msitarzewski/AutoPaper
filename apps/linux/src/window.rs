//! The main window (docs/app-spec.md, "Main window"): an AdwNavigationSplitView, GNOME's sidebar layout.
//!
//! The sidebar is a navigation list: Now; Moods (the summary of every mood, "Your moods") with each mood under it, in
//! the person's order, the current one checked; and History. Its header bar has New mood (+, Ctrl+N) and the primary
//! menu. The content shows the page chosen, each with its own header bar: Now and a mood have the reload/stop button
//! (`MakeOrStop`: New wallpaper now while idle, Stop while one is being made, like a browser's); a mood's header has its
//! name in the title's place (renamable in place) and Use this mood or "Current mood"; History has Grid | Gallery;
//! the pages without a stop button show a progress button while a wallpaper is being made (it opens Now, where Cancel
//! is). In a narrow window the split view collapses: the sidebar, then the page pushed over it with a back button.
//!
//! Keyboard: Ctrl+R or F5 is the reload button of the page shown (on another mood's page it makes that mood current
//! first; elsewhere it's New wallpaper now); Esc stops the wallpaper being made, unless the keyboard is in a text field
//! (there Esc is the field's: it puts the text back); Ctrl+N is New mood; Alt+1, 2, 3 go to Now, Moods and History.
//!
//! Before the engine opens the window shows a spinner; on first run, the welcome (an AdwStatusPage) instead.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::{gio, glib};

use crate::app::{App, Event};
use crate::desktop::{self, APP_ID};
use crate::history::HistoryPage;
use crate::moods::MoodsPage;
use crate::now::NowPage;
use crate::preferences;
use crate::summary::SummaryPage;
use crate::welcome::Welcome;

/// The project's website (GitHub Pages): About's links and Help.
pub const WEBSITE: &str = "https://msitarzewski.github.io/AutoPaper/";
pub const HELP: &str = "https://msitarzewski.github.io/AutoPaper/help.html";
pub const PRIVACY: &str = "https://msitarzewski.github.io/AutoPaper/privacy.html";
pub const ISSUES: &str = "https://github.com/msitarzewski/AutoPaper/issues";

type WindowAction = Box<dyn Fn(&Rc<MainWindow>)>;

/// Where the content pane is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    Now,
    /// The summary of every mood ("Your moods").
    Moods,
    Mood(String),
    History,
}

impl Destination {
    fn page(&self) -> &'static str {
        match self {
            Destination::Now => "now",
            Destination::Moods => "moods",
            Destination::Mood(_) => "mood",
            Destination::History => "history",
        }
    }
}

pub struct MainWindow {
    window: adw::ApplicationWindow,
    app: Weak<App>,
    outer: gtk::Stack,
    split: adw::NavigationSplitView,
    sidebar: gtk::ListBox,
    now_row: gtk::ListBoxRow,
    moods_row: gtk::ListBoxRow,
    history_row: gtk::ListBoxRow,
    content_page: adw::NavigationPage,
    content: gtk::Stack,
    toasts: adw::ToastOverlay,
    error_page: adw::StatusPage,
    now: Rc<NowPage>,
    moods: Rc<MoodsPage>,
    /// Owned here: its handlers hold it weakly.
    _summary: Rc<SummaryPage>,
    history: Rc<HistoryPage>,
    destination: RefCell<Destination>,
    /// The sidebar's selection is being changed by the window itself (not the person choosing a row).
    selecting: Cell<bool>,
    welcome: RefCell<Option<Rc<Welcome>>>,
    preferences: glib::WeakRef<adw::PreferencesDialog>,
    /// The open dialog's pages (to go to the field that fixes a problem).
    pages: RefCell<Weak<preferences::Pages>>,
}

impl MainWindow {
    pub fn new(app: &Rc<App>) -> Rc<Self> {
        let window = adw::ApplicationWindow::builder()
            .application(&app.gtk)
            .title("AutoPaper")
            .icon_name(APP_ID)
            .default_width(app.prefs.int("window-width"))
            .default_height(app.prefs.int("window-height"))
            .width_request(360)
            .height_request(400)
            .build();
        if app.prefs.boolean("window-maximized") {
            window.maximize();
        }

        let toasts = adw::ToastOverlay::new();
        let now = NowPage::new(app);
        let summary = SummaryPage::new(app);
        let history = HistoryPage::new(app);

        // The sidebar: Now, Moods (with each mood under it: MoodsPage puts them there), History.
        let sidebar = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .activate_on_single_click(false)
            .css_classes(["navigation-sidebar"])
            .build();
        sidebar.update_property(&[gtk::accessible::Property::Label("AutoPaper")]);
        // One Tab stop: the focused row; the arrows move between rows (and show each). Tab then goes to the page.
        sidebar.set_tab_behavior(gtk::ListTabBehavior::Item);
        let now_row = nav_row("preferences-desktop-wallpaper-symbolic", "Now");
        let moods_row = nav_row("view-list-bullet-symbolic", "Moods");
        let history_row = nav_row("document-open-recent-symbolic", "History");
        sidebar.append(&now_row);
        sidebar.append(&moods_row);
        sidebar.append(&history_row);
        let moods = MoodsPage::new(app, &sidebar, 2, &toasts);

        let new_mood = gtk::Button::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text("New mood (Ctrl+N)")
            .action_name("win.new-mood")
            .build();
        new_mood.update_property(&[gtk::accessible::Property::Label("New mood")]);
        let sidebar_header = adw::HeaderBar::new();
        sidebar_header.pack_start(&new_mood);
        sidebar_header.pack_end(&primary_menu_button());
        let sidebar_scroll =
            gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&sidebar).build();
        let sidebar_view = adw::ToolbarView::new();
        sidebar_view.add_top_bar(&sidebar_header);
        sidebar_view.set_content(Some(&sidebar_scroll));
        let sidebar_page = adw::NavigationPage::builder().title("AutoPaper").tag("sidebar").child(&sidebar_view).build();

        // Not homogeneous: each page asks only for its own size (a wide one mustn't widen the others).
        let content = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::None)
            .hhomogeneous(false)
            .vhomogeneous(false)
            .build();
        content.add_named(now.widget(), Some("now"));
        content.add_named(summary.widget(), Some("moods"));
        content.add_named(moods.detail_widget(), Some("mood"));
        content.add_named(history.widget(), Some("history"));
        let content_page = adw::NavigationPage::builder().title("Now").tag("content").child(&content).build();

        let split = adw::NavigationSplitView::builder()
            .sidebar(&sidebar_page)
            .content(&content_page)
            .min_sidebar_width(200.0)
            .max_sidebar_width(280.0)
            .sidebar_width_fraction(0.26)
            .build();
        toasts.set_child(Some(&split));

        // Narrow windows: History's gallery puts the details under the picture; narrower, the split view collapses
        // (the sidebar, then a page pushed over it); a phone's width also drops the keywords' drag handles, so each
        // row's Must / Maybe / Avoid keeps its words (rows still move with their menu and Alt+Up / Alt+Down). Only one
        // breakpoint applies at a time (the last added that matches), so the window reads which one it is.
        let mut breakpoints = Vec::new();
        for width in ["860sp", "620sp", "400sp"] {
            let Ok(condition) = adw::BreakpointCondition::parse(&format!("max-width: {width}")) else { continue };
            let breakpoint = adw::Breakpoint::new(condition);
            window.add_breakpoint(breakpoint.clone());
            breakpoints.push(breakpoint);
        }

        let loading = adw::StatusPage::builder()
            .title("Opening AutoPaper…")
            .paintable(&adw::SpinnerPaintable::new(None::<&gtk::Widget>))
            .build();
        let error_page = adw::StatusPage::builder()
            .icon_name("dialog-error-symbolic")
            .title("AutoPaper couldn't start")
            .build();
        let outer = gtk::Stack::new();
        outer.add_named(&bare_page(&loading), Some("loading"));
        outer.add_named(&bare_page(&error_page), Some("error"));
        outer.add_named(&toasts, Some("main"));
        window.set_content(Some(&outer));

        let this = Rc::new(Self {
            window,
            app: Rc::downgrade(app),
            outer,
            split,
            sidebar,
            now_row,
            moods_row,
            history_row,
            content_page,
            content,
            toasts,
            error_page,
            now,
            moods,
            _summary: summary,
            history,
            destination: RefCell::new(Destination::Now),
            selecting: Cell::new(false),
            welcome: RefCell::new(None),
            preferences: glib::WeakRef::new(),
            pages: RefCell::new(Weak::new()),
        });
        this.follow_breakpoints(breakpoints);
        this.install_actions();
        this.install_keys();
        this.save_size_on_close();
        this.connect_sidebar();
        // A dialog gives focus back to what had it before; when that went away meanwhile (a problem's link that the
        // change in Preferences fixed), focus would rest on a control that isn't shown, or nowhere: it goes to the
        // page's first control instead, once no dialog holds it.
        let weak = Rc::downgrade(&this);
        this.window.connect_focus_widget_notify(move |window| {
            if !gtk::prelude::RootExt::focus(window).is_some_and(|focus| focus.is_mapped())
                && let Some(this) = weak.upgrade()
            {
                this.recover_focus(8);
            }
        });

        let weak = Rc::downgrade(&this);
        app.subscribe(move |event| {
            let Some(this) = weak.upgrade() else { return false };
            if matches!(event, Event::Ready | Event::Settings) {
                this.update_mode();
            }
            // The moods page has rebuilt its rows by now (it subscribed first): the selection follows the page shown.
            if matches!(event, Event::Ready | Event::Moods) {
                this.moods_changed();
            }
            true
        });
        this.update_mode();
        this.select_row_for(&Destination::Now);
        this
    }

    // ── Where the content is ────────────────────────────────────────────────────────────────────────────────

    pub fn widget(&self) -> &adw::ApplicationWindow {
        &self.window
    }

    pub fn destination(&self) -> Destination {
        self.destination.borrow().clone()
    }

    pub fn show_now(&self) {
        self.go(Destination::Now, true);
    }

    /// The summary of every mood.
    pub fn show_moods(&self) {
        self.go(Destination::Moods, true);
    }

    /// One mood's page (a summary card, a problem's link, Rename…). Unknown moods go to the summary.
    pub fn show_mood(&self, id: &str) {
        let known = self.app.upgrade().is_some_and(|app| app.state().moods.iter().any(|mood| mood.id == id));
        self.go(if known { Destination::Mood(id.to_string()) } else { Destination::Moods }, true);
    }

    pub fn show_history(&self) {
        self.go(Destination::History, true);
    }

    /// History, filtered to the wallpapers made in one mood (a mood's "Show in History").
    pub fn show_mood_history(&self, mood_id: &str) {
        self.history.show_mood(Some(mood_id.to_string()));
        self.show_history();
    }

    /// Shows `destination` and selects its row. `reveal`: in a collapsed window, push the page over the sidebar (the
    /// person went there); otherwise only what the page is changes.
    fn go(&self, destination: Destination, reveal: bool) {
        self.select_row_for(&destination);
        self.show_destination(destination);
        if reveal && self.split.is_collapsed() {
            self.split.set_show_content(true);
        }
    }

    fn select_row_for(&self, destination: &Destination) {
        let row = match destination {
            Destination::Now => Some(self.now_row.clone()),
            Destination::Moods => Some(self.moods_row.clone()),
            Destination::History => Some(self.history_row.clone()),
            Destination::Mood(id) => self.moods.row_for_mood(id),
        };
        if row.as_ref() != self.sidebar.selected_row().as_ref() {
            // Keyboard focus in the sidebar follows the selection (else the arrows would move on from the row left).
            let focus_in_sidebar = gtk::prelude::RootExt::focus(&self.window)
                .is_some_and(|focus| focus.is_ancestor(&self.sidebar));
            self.selecting.set(true);
            self.sidebar.select_row(row.as_ref());
            self.selecting.set(false);
            if focus_in_sidebar && let Some(row) = &row {
                row.grab_focus();
            }
        }
    }

    fn show_destination(&self, destination: Destination) {
        let Some(app) = self.app.upgrade() else { return };
        // Keyboard focus on the page that's going would be left nowhere: it goes to the sidebar's row for the new page
        // (side by side), where the person can go on choosing, or into the new page (collapsed).
        let leaving_focus = gtk::prelude::RootExt::focus(&self.window)
            .zip(self.content.visible_child())
            .is_some_and(|(focus, page)| focus.is_ancestor(&page));
        let changing = self.destination() != destination;
        let title = match &destination {
            Destination::Now => "Now".to_string(),
            Destination::Moods => "Moods".to_string(),
            Destination::History => "History".to_string(),
            Destination::Mood(id) => {
                let name = app.state().moods.iter().find(|mood| mood.id == *id).map(|mood| mood.name.clone());
                self.moods.show_mood(id);
                name.unwrap_or_else(|| "Mood".into())
            }
        };
        self.content_page.set_title(&title);
        self.content.set_visible_child_name(destination.page());
        self.destination.replace(destination);
        if leaving_focus && changing {
            if self.split.is_collapsed() {
                self.content.child_focus(gtk::DirectionType::TabForward);
            } else if let Some(row) = self.sidebar.selected_row() {
                row.grab_focus();
            }
        }
    }

    /// The moods changed (one added, renamed, deleted or reordered): the page shown keeps its mood if it's still there;
    /// a deleted one gives way to its neighbour (the mood MoodsPage now has in its place), else the summary.
    fn moods_changed(&self) {
        let destination = self.destination();
        match &destination {
            Destination::Mood(id) if self.moods.row_for_mood(id).is_none() => {
                match self.moods.replacement_for(id) {
                    Some(next) => self.go(Destination::Mood(next), false),
                    None => self.go(Destination::Moods, false),
                }
            }
            Destination::Mood(id) => {
                let id = id.clone();
                self.select_row_for(&destination);
                if let Some(app) = self.app.upgrade()
                    && let Some(mood) = app.state().moods.iter().find(|mood| mood.id == id)
                {
                    self.content_page.set_title(&mood.name);
                }
            }
            _ => self.select_row_for(&destination),
        }
    }

    fn connect_sidebar(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.sidebar.connect_row_selected(move |_, row| {
            let (Some(this), Some(row)) = (weak.upgrade(), row) else { return };
            if this.selecting.get() {
                return;
            }
            if let Some(destination) = this.destination_of(row)
                && destination != this.destination()
            {
                this.show_destination(destination);
            }
        });
        // Return or a double click on a mood uses it (MoodsPage); in a collapsed split view, a click (or Return) on any
        // row opens its page.
        let weak = Rc::downgrade(self);
        self.sidebar.connect_row_activated(move |_, row| {
            let Some(this) = weak.upgrade() else { return };
            if this.split.is_collapsed() {
                if let Some(destination) = this.destination_of(row) {
                    this.go(destination, true);
                }
            } else if let Some(id) = this.moods.mood_of_row(row) {
                this.moods.use_mood(id);
            }
        });
    }

    fn destination_of(&self, row: &gtk::ListBoxRow) -> Option<Destination> {
        if *row == self.now_row {
            Some(Destination::Now)
        } else if *row == self.moods_row {
            Some(Destination::Moods)
        } else if *row == self.history_row {
            Some(Destination::History)
        } else {
            self.moods.mood_of_row(row).map(Destination::Mood)
        }
    }

    fn follow_breakpoints(self: &Rc<Self>, breakpoints: Vec<adw::Breakpoint>) {
        let weak = Rc::downgrade(self);
        self.window.connect_current_breakpoint_notify(move |window| {
            let Some(this) = weak.upgrade() else { return };
            let current = window.current_breakpoint();
            let level = current.and_then(|current| breakpoints.iter().position(|breakpoint| *breakpoint == current));
            // 0: the gallery stacks; 1: the split view collapses too; 2: and the keywords go compact.
            let stacked = level.is_some();
            let collapsed = level.is_some_and(|level| level >= 1);
            let compact = level == Some(2);
            this.split.set_collapsed(collapsed);
            // Collapsed, a row is a link to its page (one click opens it); side by side, a click selects it.
            this.sidebar.set_activate_on_single_click(collapsed);
            this.moods.set_compact(compact);
            this.history.set_stacked(stacked);
        });
    }

    // ── Focus ───────────────────────────────────────────────────────────────────────────────────────────────

    /// Focus is nowhere, or on a control that isn't shown (a dialog gave it back to one that went meanwhile): once no
    /// dialog is open (one closing still holds it for a moment; `tries` checks 150 ms apart), it goes to the Now page's
    /// first control (New wallpaper now, or Cancel while one is being made), else to the page's first control.
    fn recover_focus(self: &Rc<Self>, tries: u32) {
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
            let Some(this) = weak.upgrade() else { return };
            let shown = gtk::prelude::RootExt::focus(&this.window).is_some_and(|focus| focus.is_mapped());
            if shown || !this.window.is_visible() {
                return;
            }
            if this.window.visible_dialog().is_some() {
                if tries > 1 {
                    this.recover_focus(tries - 1);
                }
                return;
            }
            let main = this.outer.visible_child_name().as_deref() == Some("main");
            if main && this.destination() == Destination::Now {
                this.now.focus_first();
            } else {
                this.window.child_focus(gtk::DirectionType::TabForward);
            }
        });
    }

    // ── Modes: loading, the open error, the welcome, the views ──────────────────────────────────────────────

    fn update_mode(self: &Rc<Self>) {
        let Some(app) = self.app.upgrade() else { return };
        if let Some(problem) = app.open_error() {
            self.error_page.set_description(Some(&glib::markup_escape_text(&problem)));
            self.outer.set_visible_child_name("error");
            return;
        }
        if !app.is_ready() {
            self.outer.set_visible_child_name("loading");
            return;
        }
        if Welcome::wanted(&app) {
            if self.welcome.borrow().is_none() {
                let weak = Rc::downgrade(self);
                let welcome = Welcome::new(&app, move || {
                    if let Some(this) = weak.upgrade() {
                        this.leave_welcome();
                    }
                });
                self.outer.add_named(welcome.widget(), Some("welcome"));
                self.welcome.replace(Some(welcome));
            }
            self.outer.set_visible_child_name("welcome");
        } else if self.welcome.borrow().is_none() || self.outer.visible_child_name().as_deref() != Some("welcome") {
            let first_time = self.outer.visible_child_name().as_deref() != Some("main");
            self.outer.set_visible_child_name("main");
            if first_time {
                self.focus_start();
            }
        }
        self.update_actions();
    }

    /// Where keyboard focus starts: the sidebar's selected row (Now), as in GNOME's sidebar apps, so the arrows go
    /// between pages and Tab into the page. GTK picks its own first focus when the window first becomes active, so the
    /// row is focused then too (after GTK's choice).
    fn focus_start(self: &Rc<Self>) {
        let Some(row) = self.sidebar.selected_row() else { return };
        gtk::prelude::GtkWindowExt::set_focus(&self.window, Some(&row));
        if self.window.is_active() {
            return;
        }
        let handler: Rc<RefCell<Option<glib::SignalHandlerId>>> = Rc::new(RefCell::new(None));
        let once = handler.clone();
        let weak = Rc::downgrade(self);
        let id = self.window.connect_is_active_notify(move |window| {
            if !window.is_active() {
                return;
            }
            let weak = weak.clone();
            glib::idle_add_local_once(move || {
                if let Some(this) = weak.upgrade()
                    && let Some(row) = this.sidebar.selected_row()
                    && !gtk::prelude::RootExt::focus(&this.window).is_some_and(|focus| focus.is_ancestor(&this.sidebar))
                {
                    row.grab_focus();
                }
            });
            if let Some(id) = once.take() {
                window.disconnect(id);
            }
        });
        handler.replace(Some(id));
    }

    fn leave_welcome(&self) {
        self.outer.set_visible_child_name("main");
        self.show_now();
        self.now.focus_main_action();
        if let Some(welcome) = self.welcome.take() {
            self.outer.remove(welcome.widget());
        }
        self.update_actions();
    }

    fn main_shown(&self) -> bool {
        self.outer.visible_child_name().as_deref() == Some("main")
    }

    fn update_actions(&self) {
        let main = self.main_shown();
        for name in ["new-mood", "make", "view"] {
            if let Some(action) = self.window.lookup_action(name).and_downcast::<gio::SimpleAction>() {
                action.set_enabled(main);
            }
        }
    }

    // ── Toasts, Preferences ─────────────────────────────────────────────────────────────────────────────────

    /// A toast in the window.
    pub fn toast(&self, text: &str) {
        self.toasts.add_toast(toast(text));
    }

    /// Preferences opened where a problem is fixed, with the field focused (the link a problem shows).
    pub fn open_fix(&self, fix: &preferences::Fix) {
        self.open_preferences(None);
        if let (Some(dialog), Some(pages)) = (self.preferences.upgrade(), self.pages.borrow().upgrade()) {
            pages.show_fix(&dialog, fix);
        }
    }

    /// Preferences, on the page shown last (AudioPaper: settings reopen where they were left).
    pub fn open_preferences(&self, page: Option<&str>) {
        let Some(app) = self.app.upgrade() else { return };
        if let Some(open) = self.preferences.upgrade() {
            if let Some(page) = page {
                open.set_visible_page_name(page);
            }
            return;
        }
        let (dialog, pages) = preferences::build(&app);
        self.pages.replace(Rc::downgrade(&pages));
        let page = page.map(str::to_string).unwrap_or_else(|| app.prefs.string("preferences-page").to_string());
        dialog.set_visible_page_name(&page);
        let prefs = app.prefs.clone();
        dialog.connect_visible_page_name_notify(move |dialog| {
            if let Some(name) = dialog.visible_page_name() {
                let _ = prefs.set_string("preferences-page", &name);
            }
        });
        dialog.present(Some(&self.window));
        self.preferences.set(Some(&dialog));
    }

    // ── Actions and keys ────────────────────────────────────────────────────────────────────────────────────

    /// What Ctrl+R and F5 do on the page shown: its reload button's make (a mood that isn't current is made current
    /// first); New wallpaper now where the page has none. Nothing while a wallpaper is being made (Esc stops it).
    fn make(&self) {
        let Some(app) = self.app.upgrade() else { return };
        if !self.main_shown() || app.is_busy() {
            return;
        }
        match self.destination() {
            Destination::Mood(id) => app.new_wallpaper_from(id),
            _ => app.new_wallpaper(autopaper_core::Trigger::Manual),
        }
    }

    fn install_actions(self: &Rc<Self>) {
        let add = |name: &str, run: WindowAction| {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(this) = weak.upgrade() {
                    run(&this);
                }
            });
            self.window.add_action(&action);
        };
        // New mood (Ctrl+N and the sidebar's +): not under a dialog (Ctrl+N there would make a mood behind it).
        add(
            "new-mood",
            Box::new(|this| {
                if this.main_shown() && this.window.visible_dialog().is_none() {
                    this.moods.new_mood();
                }
            }),
        );
        add("make", Box::new(|this| this.make()));

        let view = gio::SimpleAction::new("view", Some(glib::VariantTy::STRING));
        let weak = Rc::downgrade(self);
        view.connect_activate(move |_, parameter| {
            if let (Some(this), Some(name)) = (weak.upgrade(), parameter.and_then(|p| p.get::<String>()))
                && this.main_shown()
            {
                match name.as_str() {
                    "moods" => this.show_moods(),
                    "history" => this.show_history(),
                    _ => this.show_now(),
                }
            }
        });
        self.window.add_action(&view);
    }

    /// Esc stops the wallpaper being made — unless the keyboard is in a text field: there Esc belongs to the field (a
    /// keyword or the mood's name puts its text back), so it never stops a wallpaper by accident. Dialogs and popovers
    /// take their own Esc first.
    fn install_keys(self: &Rc<Self>) {
        let shortcuts = gtk::ShortcutController::new();
        shortcuts.set_scope(gtk::ShortcutScope::Local);
        let weak = Rc::downgrade(self);
        let stop = gtk::CallbackAction::new(move |_, _| {
            let Some(this) = weak.upgrade() else { return glib::Propagation::Proceed };
            let Some(app) = this.app.upgrade() else { return glib::Propagation::Proceed };
            let focus = gtk::prelude::RootExt::focus(&this.window);
            let in_text = focus.is_some_and(|focus| is_text_field(&focus));
            if in_text || !this.main_shown() || !app.is_busy() || !app.state().making {
                return glib::Propagation::Proceed;
            }
            app.cancel();
            glib::Propagation::Stop
        });
        if let Some(trigger) = gtk::ShortcutTrigger::parse_string("Escape") {
            shortcuts.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(stop)));
        }
        self.window.add_controller(shortcuts);
    }

    fn save_size_on_close(self: &Rc<Self>) {
        let weak_app = self.app.clone();
        self.window.connect_close_request(move |window| {
            if let Some(app) = weak_app.upgrade() {
                let (width, height) = window.default_size();
                let _ = app.prefs.set_int("window-width", width);
                let _ = app.prefs.set_int("window-height", height);
                let _ = app.prefs.set_boolean("window-maximized", window.is_maximized());
            }
            // The window goes; AutoPaper keeps running in the background (the app holds itself).
            glib::Propagation::Proceed
        });
    }

    /// Moves keyboard focus to Cancel on Now's progress pill (a progress button opened Now).
    pub fn focus_cancel(&self) {
        self.now.focus_cancel();
    }
}

/// Whether `widget` is a text field taking typing (an entry's text, a label being renamed).
fn is_text_field(widget: &gtk::Widget) -> bool {
    widget.is::<gtk::Text>() || widget.is::<gtk::TextView>() || widget.ancestor(gtk::EditableLabel::static_type()).is_some()
}

/// A sidebar row: an icon and its name. The row speaks its name (the icon is decoration).
fn nav_row(icon: &str, title: &str) -> gtk::ListBoxRow {
    let content = gtk::Box::builder().spacing(12).build();
    content.append(&crate::history::decorative_icon(icon));
    content.append(&gtk::Label::builder().label(title).xalign(0.0).hexpand(true).build());
    let row = gtk::ListBoxRow::builder().child(&content).build();
    row.update_property(&[gtk::accessible::Property::Label(title)]);
    row
}

// ── New wallpaper now ↔ Stop ────────────────────────────────────────────────────────────────────────────────────

/// Now's and a mood's reload button (docs/app-spec.md, Now toolbar): New wallpaper now (view-refresh) while idle, Stop
/// (process-stop) while a wallpaper is being made, like a browser's reload/stop — one button that changes, so keyboard
/// focus stays on it. On a mood that isn't current it's Use this mood and make a new wallpaper (`App::
/// new_wallpaper_from`). Stop is off while a finished wallpaper is only being put on the desktop (nothing to stop).
pub struct MakeOrStop {
    button: gtk::Button,
    app: Weak<App>,
    /// The mood whose page shows the button; `None` on Now.
    mood: RefCell<Option<String>>,
}

impl MakeOrStop {
    pub fn new(app: &Rc<App>) -> Rc<Self> {
        let button = gtk::Button::builder().icon_name("view-refresh-symbolic").build();
        let this = Rc::new(Self { button, app: Rc::downgrade(app), mood: RefCell::new(None) });
        let weak = Rc::downgrade(&this);
        this.button.connect_clicked(move |_| {
            let Some(this) = weak.upgrade() else { return };
            let Some(app) = this.app.upgrade() else { return };
            if app.is_busy() {
                app.cancel();
                return;
            }
            match this.mood.borrow().clone() {
                Some(id) => app.new_wallpaper_from(id),
                None => app.new_wallpaper(autopaper_core::Trigger::Manual),
            }
        });
        let weak = Rc::downgrade(&this);
        app.subscribe(move |event| {
            let Some(this) = weak.upgrade() else { return false };
            if matches!(event, Event::Ready | Event::Progress | Event::Moods) {
                this.update();
            }
            true
        });
        this.update();
        this
    }

    pub fn widget(&self) -> &gtk::Button {
        &self.button
    }

    /// The mood whose page this button is on (`None`: Now).
    pub fn set_mood(&self, mood: Option<String>) {
        self.mood.replace(mood);
        self.update();
    }

    fn update(&self) {
        let Some(app) = self.app.upgrade() else { return };
        let busy = app.is_busy();
        let (icon, label, tooltip, enabled) = if busy {
            ("process-stop-symbolic", "Stop", "Stop making this wallpaper (Esc)".to_string(), app.state().making)
        } else {
            let other = self
                .mood
                .borrow()
                .as_ref()
                .is_some_and(|id| app.state().moods.iter().any(|mood| mood.id == *id && !mood.active));
            let label =
                if other { "Use this mood and make a new wallpaper" } else { "New wallpaper now" };
            ("view-refresh-symbolic", label, format!("{label} (Ctrl+R)"), app.is_ready())
        };
        self.button.set_icon_name(icon);
        self.button.set_tooltip_text(Some(&tooltip));
        self.button.update_property(&[gtk::accessible::Property::Label(label)]);
        self.button.set_sensitive(enabled);
    }
}

// ── While a wallpaper is being made ─────────────────────────────────────────────────────────────────────────────

/// A header button for the pages without a stop button (Moods, History): while a wallpaper is being made, a spinner and
/// the stage ("Painting…"); it opens Now, with focus on Cancel. Its accessible name says what it is ("Making a wallpaper
/// Painting…") and its description where it goes; the spinner is hidden from screen readers (the name says it all).
pub fn progress_button(app: &Rc<App>) -> gtk::Button {
    let spinner = adw::Spinner::builder().accessible_role(gtk::AccessibleRole::Presentation).build();
    let label = gtk::Label::builder().ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(22).build();
    let content = gtk::Box::builder().spacing(8).build();
    content.append(&spinner);
    content.append(&label);
    let button = gtk::Button::builder()
        .child(&content)
        .tooltip_text("Show progress on Now")
        .css_classes(["flat"])
        .visible(false)
        .build();
    button.update_property(&[gtk::accessible::Property::Description("Shows it on Now, with Cancel")]);
    let weak = Rc::downgrade(app);
    button.connect_clicked(move |_| {
        if let Some(window) = weak.upgrade().and_then(|app| app.main_window()) {
            window.show_now();
            // The button goes as Now shows: focus goes to Cancel on the pill, the control it stood for.
            window.focus_cancel();
        }
    });
    let (weak_button, weak_label) = (button.downgrade(), label.downgrade());
    let update = move |app: &App| {
        let (Some(button), Some(label)) = (weak_button.upgrade(), weak_label.upgrade()) else { return false };
        let stage = app.state().stage.and_then(crate::strings::stage_label).unwrap_or(crate::strings::MAKING);
        label.set_label(stage);
        button.update_property(&[gtk::accessible::Property::Label(&format!(
            "{} {}",
            crate::strings::MAKING.trim_end_matches('…'),
            stage
        ))]);
        let busy = app.is_busy();
        // Focus never stays on a button that goes away.
        if !busy && button.has_focus() {
            button.child_focus(gtk::DirectionType::TabForward);
        }
        button.set_visible(busy);
        true
    };
    update(app);
    let weak = Rc::downgrade(app);
    app.subscribe(move |event| match weak.upgrade() {
        Some(app) if matches!(event, Event::Progress | Event::Ready) => update(&app),
        Some(_) => true,
        None => false,
    });
    button
}

/// A toast that shows its text as written. AdwToast reads its title as Pango markup by default, so a keyword like
/// “rock & roll”, a server's address or a provider's words would come out empty; every toast goes through here.
pub fn toast(text: &str) -> adw::Toast {
    adw::Toast::builder().title(text).use_markup(false).build()
}

/// A page with just a header bar (so the window can still be moved and closed).
fn bare_page(content: &impl IsA<gtk::Widget>) -> adw::ToolbarView {
    let page = adw::ToolbarView::new();
    page.add_top_bar(&adw::HeaderBar::new());
    page.set_content(Some(content));
    page
}

/// GNOME's primary menu, in the sidebar's header bar. "Quit" is here because closing the window keeps AutoPaper running
/// in the background.
fn primary_menu_button() -> gtk::MenuButton {
    let menu = gio::Menu::new();
    let making = gio::Menu::new();
    let new = gio::MenuItem::new(Some("New wallpaper now"), Some("app.new-wallpaper"));
    // Ctrl+R is the page's reload button (`win.make`), which is New wallpaper now on Now; shown here as GTK shows a
    // shortcut.
    new.set_attribute_value("accel", Some(&"<Control>r".to_variant()));
    making.append_item(&new);
    making.append(Some("Pause new wallpapers"), Some("app.pause"));
    // The person's own wallpaper back on the desktop (docs/app-spec.md 3a); off while their own is showing. Hidden
    // where it can't work.
    if desktop::can_restore_wallpaper() {
        making.append(Some("Restore my wallpaper"), Some("app.restore-wallpaper"));
    }
    menu.append_section(None, &making);
    let app = gio::Menu::new();
    app.append(Some("Preferences"), Some("app.preferences"));
    app.append(Some("Keyboard shortcuts"), Some("app.shortcuts"));
    app.append(Some("Help"), Some("app.help"));
    app.append(Some("About AutoPaper"), Some("app.about"));
    menu.append_section(None, &app);
    let quit = gio::Menu::new();
    quit.append(Some("Quit AutoPaper"), Some("app.quit"));
    menu.append_section(None, &quit);
    gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&menu)
        .primary(true)
        .tooltip_text("Main menu")
        .build()
}

/// About AutoPaper (AdwAboutDialog), with the website, Help and Privacy.
pub fn show_about(app: &Rc<App>) {
    let dialog = adw::AboutDialog::builder()
        .application_icon(APP_ID)
        .application_name("AutoPaper")
        .version(format!("{} ({})", desktop::VERSION, desktop::BUILD))
        .comments("Keeps making new wallpapers from a few keywords, and remembers everything it has made, so you're unlikely to see the same idea twice.")
        .developer_name("Michael Sitarzewski")
        .developers(vec!["Michael Sitarzewski"])
        .license_type(gtk::License::MitX11)
        .copyright("© 2026 Michael Sitarzewski")
        .website(WEBSITE)
        .issue_url(ISSUES)
        .build();
    dialog.add_link("_Help", HELP);
    dialog.add_link("_Privacy", PRIVACY);
    dialog.add_legal_section(
        "Heroicons",
        Some("© Tailwind Labs, Inc."),
        gtk::License::MitX11,
        Some("The sparkles in AutoPaper's icon are Heroicons' “sparkles”."),
    );
    match app.window_widget() {
        Some(window) => dialog.present(Some(&window)),
        None => dialog.present(None::<&gtk::Widget>),
    }
}

/// Help: the website's help page (GNOME's primary menu and F1).
pub fn show_help(app: &Rc<App>) {
    desktop::open_uri(HELP, app.window_widget().as_ref());
}

/// The keyboard shortcuts (AdwShortcutsDialog).
pub fn show_shortcuts(app: &Rc<App>) {
    let dialog = adw::ShortcutsDialog::new();
    let general = adw::ShortcutsSection::new(None);
    for (title, accelerator) in [
        ("New wallpaper now (on a mood's page: use it and make one)", "<Control>r"),
        ("New wallpaper now", "F5"),
        ("Stop making a wallpaper", "Escape"),
        ("Preferences", "<Control>comma"),
        ("Keyboard shortcuts", "<Control>question"),
        ("Help", "F1"),
        ("Close the window", "<Control>w"),
        ("Quit AutoPaper", "<Control>q"),
    ] {
        general.add(adw::ShortcutsItem::new(title, accelerator));
    }
    dialog.add(general);
    let views = adw::ShortcutsSection::new(Some("Going places"));
    for (title, accelerator) in [("Now", "<Alt>1"), ("Moods", "<Alt>2"), ("History", "<Alt>3")] {
        views.add(adw::ShortcutsItem::new(title, accelerator));
    }
    dialog.add(views);
    let moods = adw::ShortcutsSection::new(Some("Moods"));
    for (title, accelerator) in [
        ("New mood", "<Control>n"),
        ("Use the selected mood", "Return"),
        ("Show a mood's actions", "<Shift>F10"),
        ("Delete a mood", "Delete"),
        ("Move a mood or keyword up", "<Alt>Up"),
        ("Move a mood or keyword down", "<Alt>Down"),
        ("Bring back a removed keyword", "<Control>z"),
    ] {
        moods.add(adw::ShortcutsItem::new(title, accelerator));
    }
    dialog.add(moods);
    let history = adw::ShortcutsSection::new(Some("History"));
    for (title, accelerator) in [
        ("Show a wallpaper's actions", "<Shift>F10"),
        ("Gallery: show the chosen wallpaper on the desktop", "Return"),
        ("Gallery: open it in the image viewer", "space"),
        ("Delete a wallpaper", "Delete"),
    ] {
        history.add(adw::ShortcutsItem::new(title, accelerator));
    }
    dialog.add(history);
    match app.window_widget() {
        Some(window) => dialog.present(Some(&window)),
        None => dialog.present(None::<&gtk::Widget>),
    }
}
