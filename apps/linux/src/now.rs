//! Now: the wallpaper showing, large; its title, rating (a badge as well as the pressed toggle), summary
//! (selectable), echo note and which models made it (the painter a link to Preferences → Providers); Like / Dislike
//! toggles; and the status lines (next one, budget, any problem or revisit, narrow keywords, the person's own wallpaper
//! showing). New wallpaper now is the header bar's reload button (`window::MakeOrStop`), which turns into Stop while
//! one is being made; before the first wallpaper the empty page has a New wallpaper now button too.
//!
//! While a wallpaper is being made, a pill over the picture shows the stage, a spinner (or, when something real
//! backs it, a progress bar and "about 2 minutes left") and Cancel, and the picture is veiled: the veil fades in
//! once (no fade when the system asks for reduced motion or has animations off) and then stays still. Before the
//! first wallpaper, the frame stands in for it while it's made.
//!
//! A problem with a setting is said once, with a link to its fix (docs/app-spec.md 6a): a button row ("Add your
//! OpenAI key", or "Check ComfyUI's settings" under the line saying what happened) that opens Preferences there.
//!
//! Screen readers hear, politely, from any view while the window is open: each stage once per wallpaper and at most
//! one stage every 1.5 s (the latest one), how it ended ("New wallpaper: …", "Stopped."), and any problem when it
//! appears. Keyboard focus moves with the controls: the reload button stays focused as it turns into Stop; Like /
//! Dislike (disabled while a wallpaper is made) and the empty page's button hand it to Cancel when a wallpaper starts,
//! and Cancel hands it back to the reload button when it ends; a problem's link that goes away hands it to the reload
//! button. So focus is never left on a control that just went away.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use adw::prelude::*;
use autopaper_core::Rating;
use gtk::{gdk, glib};

use crate::app::{App, Event, NoticeKind};
use crate::preferences::Fix;
use crate::strings;
use crate::wallpapers::ProvenanceLine;
use crate::window::MakeOrStop;

/// At most one stage announcement in this long; a stage that comes sooner waits, and only the latest is said.
const STAGE_ANNOUNCEMENT_GAP: Duration = Duration::from_millis(1500);
/// The frame's height while it stands in for a first wallpaper being made (no picture yet to size it).
const PLACEHOLDER_HEIGHT: i32 = 220;
/// The wallpaper's greatest height on the Now view: large, with its title and actions still in a default-sized window.
const FRAME_MAX_HEIGHT: i32 = 360;

pub struct NowPage {
    root: adw::ToolbarView,
    scroller: gtk::ScrolledWindow,
    app: Weak<App>,
    /// The header bar's New wallpaper now ↔ Stop.
    make: Rc<MakeOrStop>,
    content: gtk::Stack,
    frame: gtk::AspectFrame,
    picture: gtk::Picture,
    /// Over the picture while a new one is made (`.remaking-veil`; it fades only when motion is welcome).
    veil: gtk::Box,
    /// The title, badges, summary, echo note and actions: hidden while the frame stands in for a first wallpaper.
    details: gtk::Box,
    title: gtk::Label,
    liked_badge: gtk::Box,
    disliked_badge: gtk::Box,
    summary: gtk::Label,
    echo_note: gtk::Label,
    provenance: ProvenanceLine,
    like: gtk::ToggleButton,
    dislike: gtk::ToggleButton,
    /// New wallpaper now on the empty page (before the first wallpaper).
    empty_button: gtk::Button,
    pill: gtk::Box,
    spinner: adw::Spinner,
    stage: gtk::Label,
    bar: gtk::ProgressBar,
    time_left: gtk::Label,
    cancel: gtk::Button,
    next_row: adw::ActionRow,
    budget_row: adw::ActionRow,
    /// What happened (a problem without a link, a revisit, or the line above a painting failure's link).
    notice_row: adw::ActionRow,
    notice_icon: gtk::Image,
    /// The link to where the problem is fixed: a button row, so it's announced and activated as a button.
    notice_link: adw::ButtonRow,
    notice_fix: RefCell<Option<Fix>>,
    narrow_row: adw::ActionRow,
    background_row: adw::ActionRow,
    /// The person's own wallpaper is showing (they restored it, or chose another): the next new one covers it.
    own_row: adw::ActionRow,
    /// Programmatic updates of the toggles don't count as the person rating.
    updating: Cell<bool>,
    picture_path: RefCell<Option<String>>,
    /// Whether the pill is showing a job (to act on its start and end only).
    busy_shown: Cell<bool>,
    /// Focus was moved to Cancel when the job started, so it comes back when it ends.
    focus_moved: Cell<bool>,
    announcer: Announcer,
}

impl NowPage {
    pub fn new(app: &Rc<App>) -> Rc<Self> {
        // The picture fills a frame with the wallpaper's own aspect (no letterboxing).
        // Focusable, so keyboard and screen-reader users reach the wallpaper's description with Tab.
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Cover)
            .can_shrink(true)
            .focusable(true)
            .css_classes(["wallpaper-frame", "card"])
            .overflow(gtk::Overflow::Hidden)
            .build();
        let veil = gtk::Box::builder()
            .css_classes(["remaking-veil"])
            .can_target(false)
            .visible(false)
            .accessible_role(gtk::AccessibleRole::Presentation)
            .build();

        // Named by the stage, as the progress bar is when it takes the spinner's place.
        let spinner = adw::Spinner::new();
        let stage = gtk::Label::builder().xalign(0.0).wrap(true).max_width_chars(28).css_classes(["heading"]).build();
        let bar = gtk::ProgressBar::builder().hexpand(true).width_request(140).visible(false).build();
        let time_left = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["caption"]).visible(false).build();
        let words = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .valign(gtk::Align::Center)
            .hexpand(true)
            .build();
        words.append(&stage);
        words.append(&bar);
        words.append(&time_left);
        let cancel = gtk::Button::builder()
            .label("Cancel")
            .action_name("app.cancel")
            .valign(gtk::Align::Center)
            .css_classes(["pill"])
            .build();
        // On the picture, so it reads on any wallpaper: libadwaita's on-screen-display style.
        // A group, so screen readers have a name for it ("Making a wallpaper"): a plain box has none.
        let pill = gtk::Box::builder()
            .spacing(12)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::End)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .css_classes(["osd", "progress-pill"])
            .accessible_role(gtk::AccessibleRole::Group)
            .visible(false)
            .build();
        pill.update_property(&[gtk::accessible::Property::Label("Making a wallpaper")]);
        pill.append(&spinner);
        pill.append(&words);
        pill.append(&cancel);

        let overlay = gtk::Overlay::builder().child(&picture).build();
        overlay.add_overlay(&veil);
        overlay.add_overlay(&pill);
        let frame = gtk::AspectFrame::builder().ratio(16.0 / 9.0).obey_child(false).child(&overlay).build();
        // Large, but not so tall that the title and Like / Dislike / New wallpaper now leave the window.
        let frame_height = adw::Clamp::builder()
            .orientation(gtk::Orientation::Vertical)
            .maximum_size(FRAME_MAX_HEIGHT)
            .tightening_threshold(FRAME_MAX_HEIGHT)
            .child(&frame)
            .build();

        let title = gtk::Label::builder().wrap(true).xalign(0.0).css_classes(["title-2"]).build();
        // The rating in words beside the pressed toggle, so it never rests on shade alone.
        let liked_badge = crate::history::badge("autopaper-like-symbolic", "Liked");
        let disliked_badge = crate::history::badge("autopaper-dislike-symbolic", "Disliked");
        let badges = gtk::Box::builder().spacing(6).build();
        badges.append(&liked_badge);
        badges.append(&disliked_badge);
        let summary = gtk::Label::builder().wrap(true).xalign(0.0).selectable(true).build();
        let echo_note = gtk::Label::builder()
            .wrap(true)
            .xalign(0.0)
            .selectable(true)
            .css_classes(["secondary-text"])
            .visible(false)
            .build();

        let provenance = ProvenanceLine::new(app);

        // Constant labels: the pressed state says whether it's on (a toggle that renames itself contradicts its
        // state when it's turned off again).
        let like = rating_button("autopaper-like-symbolic", "_Like");
        let dislike = rating_button("autopaper-dislike-symbolic", "_Dislike");
        // New wallpaper now and Stop are the header bar's one button (no second one here to fight it).
        let actions = gtk::Box::builder().spacing(12).build();
        actions.append(&like);
        actions.append(&dislike);

        let details = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).build();
        details.append(&title);
        details.append(&badges);
        details.append(&summary);
        details.append(&echo_note);
        details.append(provenance.widget());
        details.append(&actions);
        let wallpaper = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).build();
        wallpaper.append(&frame_height);
        wallpaper.append(&details);

        let empty = adw::StatusPage::builder()
            .icon_name("preferences-desktop-wallpaper-symbolic")
            .title("No wallpaper yet")
            .description("AutoPaper makes one on your schedule, or now if you ask.")
            .build();
        let empty_button = gtk::Button::builder()
            .label("New wallpaper now")
            .action_name("app.new-wallpaper")
            .css_classes(["pill", "suggested-action"])
            .halign(gtk::Align::Center)
            .build();
        empty.set_child(Some(&empty_button));
        let content = gtk::Stack::new();
        content.add_named(&wallpaper, Some("wallpaper"));
        content.add_named(&empty, Some("empty"));

        // The status lines show AutoPaper's sentences and the person's own words (a server's address): plain
        // text, never markup.
        let next_row = status_row("alarm-symbolic");
        let budget_row = status_row("x-office-spreadsheet-symbolic");
        let notice_icon = crate::history::decorative_icon("dialog-warning-symbolic");
        let notice_row = adw::ActionRow::builder().title_selectable(true).use_markup(false).visible(false).build();
        notice_row.add_prefix(&notice_icon);
        // A problem with a setting is a link to where it's fixed ("Add your OpenAI key" opens Preferences → Keys
        // with the field focused): a button row in the list, in the text's own colour, with GNOME's arrow for "go
        // there"; screen readers hear a button, and its description says where it goes.
        let notice_link = adw::ButtonRow::builder().use_markup(false).end_icon_name("go-next-symbolic").visible(false).build();
        let narrow_row = status_row("dialog-information-symbolic");
        narrow_row.set_title(
            "Your keywords are narrow, so new ideas are getting hard to find. Add some Maybes or raise Surprise \
             for more variety.",
        );
        let background_row = status_row("dialog-information-symbolic");
        background_row.set_title(
            "AutoPaper isn't allowed to run in the background, so new wallpapers only come while this window is \
             open. You can allow it in Settings → Apps → AutoPaper.",
        );
        let own_row = status_row("preferences-desktop-wallpaper-symbolic");
        let status = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        status.update_property(&[gtk::accessible::Property::Label("Status")]);
        status.append(&notice_row);
        status.append(&notice_link);
        for row in [&next_row, &budget_row, &narrow_row, &background_row, &own_row] {
            status.append(row);
        }

        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(18)
            .margin_top(24)
            .margin_bottom(24)
            .margin_start(12)
            .margin_end(12)
            .build();
        column.append(&content);
        column.append(&status);
        let clamp = adw::Clamp::builder().maximum_size(760).child(&column).build();
        // Scrolled content is given its natural height (GTK's default gives it its minimum once it doesn't fit, and a
        // picture that can shrink would then shrink with a shorter window).
        let viewport = gtk::Viewport::builder().vscroll_policy(gtk::ScrollablePolicy::Natural).child(&clamp).build();
        let scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&viewport).build();
        let make = MakeOrStop::new(app);
        let header = adw::HeaderBar::new();
        header.pack_end(make.widget());
        let root = adw::ToolbarView::new();
        root.add_top_bar(&header);
        root.set_content(Some(&scroller));

        let this = Rc::new(Self {
            announcer: Announcer::new(scroller.upcast_ref()),
            root,
            scroller,
            app: Rc::downgrade(app),
            make,
            content,
            frame,
            picture,
            veil,
            details,
            title,
            liked_badge,
            disliked_badge,
            summary,
            echo_note,
            provenance,
            like,
            dislike,
            empty_button,
            pill,
            spinner,
            stage,
            bar,
            time_left,
            cancel,
            next_row,
            budget_row,
            notice_row,
            notice_icon,
            notice_link,
            notice_fix: RefCell::new(None),
            narrow_row,
            background_row,
            own_row,
            updating: Cell::new(false),
            picture_path: RefCell::new(None),
            busy_shown: Cell::new(false),
            focus_moved: Cell::new(false),
        });
        this.connect_ratings();
        this.follow_motion_settings();
        let weak = Rc::downgrade(&this);
        this.notice_link.connect_activated(move |_| {
            let Some(this) = weak.upgrade() else { return };
            let fix = this.notice_fix.borrow().clone();
            if let (Some(app), Some(fix)) = (this.app.upgrade(), fix) {
                app.open_fix(fix);
            }
        });
        let weak = Rc::downgrade(&this);
        app.subscribe(move |event| {
            let Some(this) = weak.upgrade() else { return false };
            match event {
                Event::Current | Event::Ready => this.show_current(),
                Event::Progress => this.show_progress(),
                Event::Detail => this.show_detail(),
                Event::Status | Event::Settings => this.show_status(),
                _ => {}
            }
            true
        });
        this.show_current();
        this.show_progress();
        this.show_status();
        this
    }

    pub fn widget(&self) -> &adw::ToolbarView {
        &self.root
    }

    /// Where keyboard focus starts: New wallpaper now (else the picture, the first focusable widget, would take it).
    /// At startup the views show before the current wallpaper has been read, and GTK picks its own first focus when
    /// the window becomes active, so the button (under the wallpaper, or on the empty page) is chosen when the grab
    /// happens: now, if the window is active, and again once it first is.
    pub fn focus_main_action(self: &Rc<Self>) {
        let Some(window) = self.root.root().and_downcast::<gtk::Window>() else { return };
        gtk::prelude::GtkWindowExt::set_focus(&window, Some(&self.main_action()));
        if window.is_active() && self.main_action().has_focus() {
            return;
        }
        let handler: Rc<RefCell<Option<glib::SignalHandlerId>>> = Rc::new(RefCell::new(None));
        let once = handler.clone();
        let page = Rc::downgrade(self);
        let id = window.connect_is_active_notify(move |window| {
            if !window.is_active() {
                return;
            }
            let page = page.clone();
            // After GTK has picked its own first focus for the newly active window.
            glib::idle_add_local_once(move || {
                if let Some(page) = page.upgrade() {
                    page.main_action().grab_focus();
                }
            });
            if let Some(id) = once.take() {
                window.disconnect(id);
            }
        });
        handler.replace(Some(id));
    }

    /// Focus to Cancel on the pill, while a wallpaper is being made (the header's progress button opened Now). It
    /// comes back to New wallpaper now when the wallpaper is done.
    pub fn focus_cancel(&self) {
        if self.pill.is_visible() && self.cancel.grab_focus() {
            self.focus_moved.set(true);
        }
    }

    /// Focus to New wallpaper now, or to Cancel while a wallpaper is being made (focus had nowhere to go back to).
    pub fn focus_first(&self) {
        if self.pill.is_visible() {
            self.cancel.grab_focus();
        } else {
            self.main_action().grab_focus();
        }
    }

    /// New wallpaper now: the header's reload button, or the empty page's button before the first wallpaper.
    fn main_action(&self) -> gtk::Button {
        if self.content.visible_child_name().as_deref() == Some("empty") {
            self.empty_button.clone()
        } else {
            self.make.widget().clone()
        }
    }

    fn connect_ratings(self: &Rc<Self>) {
        for (button, rating) in [(&self.like, Rating::Liked), (&self.dislike, Rating::Disliked)] {
            let weak = Rc::downgrade(self);
            button.connect_toggled(move |_| {
                let Some(this) = weak.upgrade() else { return };
                if this.updating.get() {
                    return;
                }
                let Some(app) = this.app.upgrade() else { return };
                let current = app.state().current.clone();
                if let Some(current) = current {
                    app.toggle_rating(&current, rating);
                }
            });
        }
    }

    /// The veil fades only while motion is welcome: GNOME's animations on (`gtk-enable-animations`) and no request
    /// for reduced motion (`gtk-interface-reduced-motion`). Either can change while a wallpaper is being made.
    fn follow_motion_settings(self: &Rc<Self>) {
        let Some(settings) = gtk::Settings::default() else { return };
        let weak = Rc::downgrade(self);
        settings.connect_gtk_enable_animations_notify(move |_| {
            if let Some(this) = weak.upgrade() {
                this.show_veil();
            }
        });
        let weak = Rc::downgrade(self);
        settings.connect_gtk_interface_reduced_motion_notify(move |_| {
            if let Some(this) = weak.upgrade() {
                this.show_veil();
            }
        });
    }

    fn show_veil(&self) {
        let Some(app) = self.app.upgrade() else { return };
        let making = app.is_busy() && app.state().making;
        self.veil.set_visible(making);
        if making && motion_welcome() {
            self.veil.add_css_class("animated");
        } else {
            self.veil.remove_css_class("animated");
        }
    }

    /// The wallpaper page shows the wallpaper, or stands in for the first one while it's made; else the empty page.
    fn show_page(&self, has_wallpaper: bool, busy: bool) {
        self.content.set_visible_child_name(if has_wallpaper || busy { "wallpaper" } else { "empty" });
        self.details.set_visible(has_wallpaper);
        if !has_wallpaper {
            self.picture_path.replace(None);
            self.picture.set_paintable(None::<&gdk::Paintable>);
            self.frame.set_ratio(16.0 / 9.0);
            self.picture.update_property(&[
                gtk::accessible::Property::Label("Your first wallpaper, being made"),
                gtk::accessible::Property::Description(""),
            ]);
        }
        self.picture.set_height_request(if has_wallpaper { -1 } else { PLACEHOLDER_HEIGHT });
    }

    fn show_current(&self) {
        let Some(app) = self.app.upgrade() else { return };
        let state = app.state();
        let Some(current) = state.current.as_ref() else {
            drop(state);
            self.show_page(false, app.is_busy());
            return;
        };
        self.show_page(true, app.is_busy());
        if current.width > 0 && current.height > 0 {
            self.frame.set_ratio(current.width as f32 / current.height as f32);
        }
        self.title.set_label(&current.concept.title);
        self.summary.set_label(&current.concept.summary);
        self.provenance.set(current);
        match current.echo_note.as_deref().filter(|note| !note.is_empty()) {
            Some(note) => {
                self.echo_note.set_label(note);
                self.echo_note.set_visible(true);
            }
            None => self.echo_note.set_visible(false),
        }
        // The picture speaks the wallpaper: describe() plus the rating in words.
        self.picture.update_property(&[
            gtk::accessible::Property::Label("The wallpaper on your desktop"),
            gtk::accessible::Property::Description(&state.current_spoken),
        ]);
        self.picture.set_alternative_text(Some(&state.current_spoken));
        self.updating.set(true);
        let (liked, disliked) = (current.rating == Rating::Liked, current.rating == Rating::Disliked);
        self.like.set_active(liked);
        self.dislike.set_active(disliked);
        // Shown, not spoken: the toggles' pressed state and the picture's description already say it.
        self.liked_badge.set_visible(liked);
        self.disliked_badge.set_visible(disliked);
        self.updating.set(false);

        let path = current.thumb_path.clone().or_else(|| current.image_path.clone());
        if *self.picture_path.borrow() != path {
            self.picture_path.replace(path.clone());
            match path {
                Some(path) => self.load_picture(path),
                None => self.picture.set_paintable(None::<&gdk::Paintable>),
            }
        }
    }

    fn load_picture(&self, path: String) {
        let picture = self.picture.downgrade();
        glib::spawn_future_local(async move {
            let texture = crate::wallpapers::texture(path).await;
            if let Some(picture) = picture.upgrade() {
                picture.set_paintable(texture.as_ref());
            }
        });
    }

    /// The widget with keyboard focus in this window, if any.
    fn focus(&self) -> Option<gtk::Widget> {
        self.scroller.root().and_then(|root| gtk::prelude::RootExt::focus(&root))
    }

    /// Whether focus is on `widget` or inside it.
    fn has_focus_in(focus: &Option<gtk::Widget>, widget: &impl IsA<gtk::Widget>) -> bool {
        focus.as_ref().is_some_and(|focus| focus == widget.upcast_ref() || focus.is_ancestor(widget))
    }

    /// The pill and the controls it stands in for. The app enables Cancel before it calls this at the start of a
    /// job, and New wallpaper now before it calls this at the end, so focus always has somewhere to go.
    fn show_progress(&self) {
        let Some(app) = self.app.upgrade() else { return };
        let busy = app.is_busy();
        let was_busy = self.busy_shown.replace(busy);
        let stage = app.state().stage.and_then(strings::stage_label);
        let focus = self.focus();
        if busy {
            if !was_busy {
                self.announcer.start_job();
            }
            // The control the person used is about to be disabled (or, before the first wallpaper, to go with the
            // empty page): focus goes to Cancel. The reload button keeps it: it's Stop now.
            let controls: [&gtk::Widget; 3] =
                [self.empty_button.upcast_ref(), self.like.upcast_ref(), self.dislike.upcast_ref()];
            let leaving = controls.iter().any(|control| Self::has_focus_in(&focus, *control));
            self.show_page(app.state().current.is_some(), true);
            self.pill.set_visible(true);
            if !was_busy && leaving && self.cancel.grab_focus() {
                self.focus_moved.set(true);
            }
            // From any view: the header's progress button shows the stage there, and this says it.
            if let Some(text) = stage {
                self.announcer.stage(text);
            }
            self.like.set_sensitive(false);
            self.dislike.set_sensitive(false);
        } else {
            self.like.set_sensitive(true);
            self.dislike.set_sensitive(true);
            let on_cancel = Self::has_focus_in(&focus, &self.pill);
            self.pill.set_visible(false);
            self.show_page(app.state().current.is_some(), false);
            // Focus on Cancel (or lost while it went) comes back to New wallpaper now, enabled again by now.
            if was_busy && (on_cancel || (self.focus().is_none() && self.focus_moved.get())) {
                self.main_action().grab_focus();
            }
            self.focus_moved.set(false);
            if was_busy {
                match app.take_outcome() {
                    Some(outcome) => self.announcer.result(&outcome),
                    None => self.announcer.end_job(),
                }
            }
        }
        self.show_veil();
        self.show_detail();
    }

    /// The pill's words, and its spinner or progress bar: a bar and time left only when the engine has numbers
    /// (ComfyUI's steps, or how long this painter took here before), never made up.
    fn show_detail(&self) {
        let Some(app) = self.app.upgrade() else { return };
        if !app.is_busy() {
            return;
        }
        let state = app.state();
        let stage = state.stage.and_then(strings::stage_label).unwrap_or(strings::MAKING);
        self.stage.set_label(stage);
        let fraction = state.detail.and_then(|detail| detail.fraction);
        let seconds_left = state.detail.and_then(|detail| detail.seconds_left);
        drop(state);
        self.spinner.update_property(&[gtk::accessible::Property::Label(stage.trim_end_matches('…'))]);
        self.spinner.set_visible(fraction.is_none());
        match fraction {
            Some(fraction) => {
                self.bar.set_fraction(f64::from(fraction.clamp(0.0, 1.0)));
                self.bar.set_visible(true);
                let name = stage.trim_end_matches('…');
                self.bar.update_property(&[
                    gtk::accessible::Property::Label(name),
                    gtk::accessible::Property::ValueText(&strings::progress_spoken(fraction, seconds_left)),
                ]);
            }
            None => self.bar.set_visible(false),
        }
        match seconds_left {
            Some(seconds) => {
                self.time_left.set_label(&strings::capitalized(&strings::time_left(seconds)));
                self.time_left.set_visible(true);
            }
            None => self.time_left.set_visible(false),
        }
    }

    fn show_status(&self) {
        let Some(app) = self.app.upgrade() else { return };
        let state = app.state();
        let settings = &state.settings;
        self.next_row.set_title(&strings::next_due_sentence(state.next_due, settings.paused, settings.cadence));
        match &state.spend {
            Some(spend) => {
                let spent = strings::money(spend.spent_microusd);
                let line = match spend.budget_cents {
                    Some(budget) => format!("{spent} of {} this month (estimated)", strings::cents(budget)),
                    None => format!("{spent} this month (estimated)"),
                };
                self.budget_row.set_title(&line);
                self.budget_row.set_visible(true);
            }
            None => self.budget_row.set_visible(false),
        }
        // The persistent window banner already explains a prospective budget block. Keep independent provider or
        // key errors here; showing the same budget failure twice would obscure the useful status lines.
        let notice = state.notice.as_ref().filter(|notice| {
            notice.fix != Some(Fix::Budget) || !state.budget.as_ref().is_some_and(|budget| budget.blocked)
        }).cloned();
        self.narrow_row.set_visible(state.narrow);
        self.background_row.set_visible(state.background_denied);
        let paused = settings.paused;
        let mood_names: Vec<(String, String)> = state.moods.iter().map(|mood| (mood.id.clone(), mood.name.clone())).collect();
        drop(state);
        // Pause keeps AutoPaper's wallpaper; only Restore my wallpaper (or another picture chosen since) shows theirs.
        let own = app.own_wallpaper_showing();
        self.own_row.set_title(if paused {
            "Your own wallpaper is showing. New wallpapers are paused, so it stays until you make one."
        } else {
            "Your own wallpaper is showing. AutoPaper's next new wallpaper covers it again."
        });
        self.own_row.set_visible(own);
        // Which of the two rows show: what happened, and the link to the fix. The text is the link itself when it
        // says what to do ("Add your OpenAI key"); a painting failure says what happened, with its link under it.
        let (row_shown, link_shown) = match &notice {
            Some(notice) => match (&notice.fix, &notice.link) {
                (Some(_), Some(_)) => (true, true),
                (Some(_), None) => (false, true),
                (None, _) => (true, false),
            },
            None => (false, false),
        };
        // A focused row about to go (the problem went away, or a new wallpaper started) hands focus on first: to
        // New wallpaper now, or to Cancel while one is being made. Never to the list around it.
        let focus = self.focus();
        let leaving = (!row_shown && Self::has_focus_in(&focus, &self.notice_row))
            || (!link_shown && Self::has_focus_in(&focus, &self.notice_link));
        if leaving {
            if app.is_busy() {
                self.cancel.grab_focus();
            } else {
                self.main_action().grab_focus();
            }
        }
        match notice {
            Some(notice) => {
                match &notice.link {
                    Some(link) => {
                        self.notice_row.set_title(&notice.text);
                        self.notice_link.set_title(link);
                        self.notice_link.set_start_icon_name(None);
                    }
                    None if notice.fix.is_some() => {
                        self.notice_link.set_title(&notice.text);
                        self.notice_link.set_start_icon_name(Some("dialog-warning-symbolic"));
                    }
                    None => self.notice_row.set_title(&notice.text),
                }
                let goes_to = match &notice.fix {
                    Some(Fix::Key(_)) => "Opens Preferences on the Keys page".to_string(),
                    Some(Fix::Budget) => "Opens Preferences on the Budget page".to_string(),
                    Some(Fix::Mood(id)) => match mood_names.iter().find(|(mood, _)| mood == id) {
                        Some((_, name)) => format!("Opens the mood “{name}”, to change its keywords"),
                        None => "Opens the mood, to change its keywords".to_string(),
                    },
                    _ => "Opens Preferences on the Providers page".to_string(),
                };
                self.notice_link.update_property(&[gtk::accessible::Property::Description(&goes_to)]);
                self.notice_link.set_tooltip_text(Some(&goes_to));
                self.notice_fix.replace(notice.fix.clone());
                self.notice_icon.set_icon_name(Some(match notice.kind {
                    NoticeKind::Error | NoticeKind::KeyProblem => "dialog-warning-symbolic",
                    NoticeKind::Revisit => "media-playlist-repeat-symbolic",
                }));
                let spoken = match &notice.link {
                    Some(link) => format!("{} {link}.", notice.text),
                    None => notice.text.clone(),
                };
                self.announcer.notice(Some(&spoken));
            }
            None => {
                self.notice_fix.replace(None);
                self.announcer.notice(None);
            }
        }
        self.notice_row.set_visible(row_shown);
        self.notice_link.set_visible(link_shown);
    }
}

/// Whether the system welcomes motion: animations on and no request to reduce it.
fn motion_welcome() -> bool {
    gtk::Settings::default().is_some_and(|settings| {
        settings.is_gtk_enable_animations() && settings.gtk_interface_reduced_motion() != gtk::ReducedMotion::Reduce
    })
}

/// Polite announcements for the Now view (WCAG 4.1.3 Status Messages), said through the window so they're heard
/// from any view while it's open.
struct Announcer {
    widget: glib::WeakRef<gtk::Widget>,
    /// Stages already said for the wallpaper being made.
    said: RefCell<Vec<String>>,
    /// When the last stage was said (monotonic µs).
    last_stage_at: Rc<Cell<i64>>,
    /// A stage waiting for the gap to pass; only the latest is said.
    pending: Rc<RefCell<Option<String>>>,
    timer: Rc<RefCell<Option<glib::SourceId>>>,
    /// The notice last said, so it's said once when it appears, not on every status refresh.
    notice: RefCell<Option<String>>,
}

impl Announcer {
    fn new(widget: &gtk::Widget) -> Self {
        Self {
            widget: widget.downgrade(),
            said: RefCell::new(Vec::new()),
            last_stage_at: Rc::new(Cell::new(i64::MIN / 2)),
            pending: Rc::new(RefCell::new(None)),
            timer: Rc::new(RefCell::new(None)),
            notice: RefCell::new(None),
        }
    }

    fn start_job(&self) {
        self.cancel_pending();
        self.said.borrow_mut().clear();
    }

    /// A stage: said once per wallpaper, and at most one every `STAGE_ANNOUNCEMENT_GAP`; stages that come
    /// sooner wait, and only the latest of them is said.
    fn stage(&self, text: &str) {
        if self.said.borrow().iter().any(|said| said == text) {
            return;
        }
        self.said.borrow_mut().push(text.to_string());
        if self.timer.borrow().is_some() {
            self.pending.replace(Some(text.to_string()));
            return;
        }
        let gap = STAGE_ANNOUNCEMENT_GAP.as_micros() as i64;
        let since = glib::monotonic_time() - self.last_stage_at.get();
        if since >= gap {
            self.last_stage_at.set(glib::monotonic_time());
            say(&self.widget, text);
            return;
        }
        self.pending.replace(Some(text.to_string()));
        let wait = Duration::from_micros((gap - since) as u64);
        let (pending, timer, widget, last) =
            (self.pending.clone(), self.timer.clone(), self.widget.clone(), self.last_stage_at.clone());
        let source = glib::timeout_add_local_once(wait, move || {
            // This source has fired: forget it without removing it again.
            timer.replace(None);
            if let Some(text) = pending.take() {
                last.set(glib::monotonic_time());
                say(&widget, &text);
            }
        });
        self.timer.replace(Some(source));
    }

    /// How the job ended: replaces any stage still waiting, and is said at once.
    fn result(&self, text: &str) {
        self.cancel_pending();
        say(&self.widget, text);
    }

    /// The job ended with nothing to say (the notice says it, or it was a quiet re-render).
    fn end_job(&self) {
        self.cancel_pending();
    }

    /// A problem or revisit reason appeared (said once), or went away.
    fn notice(&self, text: Option<&str>) {
        let changed = self.notice.borrow().as_deref() != text;
        if !changed {
            return;
        }
        self.notice.replace(text.map(str::to_string));
        if let Some(text) = text {
            self.cancel_pending();
            say(&self.widget, text);
        }
    }

    fn cancel_pending(&self) {
        self.pending.replace(None);
        if let Some(source) = self.timer.take() {
            source.remove();
        }
    }
}

/// Says `text` politely through the widget's window, if that window is on screen.
fn say(widget: &glib::WeakRef<gtk::Widget>, text: &str) {
    let Some(window) = widget.upgrade().and_then(|widget| widget.root()).and_downcast::<gtk::Window>() else { return };
    if window.is_mapped() {
        window.announce(text, gtk::AccessibleAnnouncementPriority::Medium);
    }
}

/// Like / Dislike: toggle buttons with icon and text (state exposed as pressed).
fn rating_button(icon: &str, label: &str) -> gtk::ToggleButton {
    let content = adw::ButtonContent::builder().icon_name(icon).label(label).use_underline(true).build();
    gtk::ToggleButton::builder().child(&content).css_classes(["pill"]).build()
}

fn status_row(icon: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder().use_markup(false).build();
    row.add_prefix(&crate::history::decorative_icon(icon));
    row
}
