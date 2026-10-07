//! A past wallpaper wherever it's shown outside the Now view — History's grid and gallery, a mood's recent strip and
//! the summary of every mood (docs/app-spec.md, History: "one shared menu builder"). One place builds:
//!
//! - its actions — Show on desktop, Like, Dislike, Make an echo, Show original and echoes (only when it has any:
//!   `has_echoes(id)`), Show in Files, Delete… — and the menu that offers them (`item_menu`, actions in the "item"
//!   group of `WallpaperActions`);
//! - the context menu on a widget: right click, long press, Shift+F10 or Menu (`attach_menu`);
//! - a thumbnail button: a click (or Return / Space) shows it on the desktop, the menu has the rest (`Thumbnail`);
//! - the original-and-echoes dialog, the delete confirmation, thumbnails decoded off the main thread and cached, and
//!   the line naming the models that made it (`ProvenanceLine`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use adw::prelude::*;
use autopaper_core::{Engine, Generation, Rating};
use gtk::{gdk, gio, glib};

use crate::app::App;
use crate::desktop;
use crate::runtime::blocking;
use crate::strings;

/// One wallpaper as the views show it: the generation, its spoken description (`describe` plus the rating in words),
/// and whether it has an original or echoes still in History.
#[derive(Clone, PartialEq)]
pub struct Item {
    pub generation: Generation,
    pub spoken: String,
    pub has_echoes: bool,
}

impl Item {
    /// Reads what the views show besides the generation. **Blocking** (two engine queries).
    pub fn read(engine: &Engine, generation: Generation) -> Self {
        let described = engine.describe(generation.id.clone()).unwrap_or_default();
        let spoken = strings::spoken_description(&described, generation.rating);
        let has_echoes = engine.has_echoes(generation.id.clone()).unwrap_or_else(|error| {
            tracing::warn!(%error, "couldn't tell whether a wallpaper has echoes");
            false
        });
        Self { generation, spoken, has_echoes }
    }
}

// ── Thumbnails, decoded once ────────────────────────────────────────────────────────────────────────────────────

/// Thumbnails kept decoded (a few pages of History's).
const TEXTURE_CACHE: usize = 320;

thread_local! {
    static TEXTURES: RefCell<HashMap<String, gdk::Texture>> = RefCell::new(HashMap::new());
}

/// The picture at `path` (a thumbnail, or the original when there's none), decoded off the main thread and cached for
/// every view. `None` when it can't be read (a pruned original, a file removed by hand).
pub async fn texture(path: String) -> Option<gdk::Texture> {
    if let Some(texture) = TEXTURES.with(|cache| cache.borrow().get(&path).cloned()) {
        return Some(texture);
    }
    let key = path.clone();
    let loaded = blocking(move || {
        gdk::Texture::from_filename(&path).map_err(|error| autopaper_core::AutoPaperError::Storage { detail: error.to_string() })
    })
    .await;
    match loaded {
        Ok(texture) => {
            TEXTURES.with(|cache| {
                let mut cache = cache.borrow_mut();
                if cache.len() >= TEXTURE_CACHE {
                    cache.clear();
                }
                cache.insert(key, texture.clone());
            });
            Some(texture)
        }
        Err(error) => {
            tracing::warn!(%error, "couldn't load a thumbnail");
            None
        }
    }
}

/// The small picture of a wallpaper: its thumbnail, else its original.
pub fn thumb_path(generation: &Generation) -> Option<String> {
    generation.thumb_path.clone().or_else(|| generation.image_path.clone())
}

/// The largest picture of a wallpaper there is: its original, else its thumbnail (the original may have been pruned).
pub fn large_path(generation: &Generation) -> Option<String> {
    generation
        .image_path
        .clone()
        .filter(|path| std::path::Path::new(path).is_file())
        .or_else(|| generation.thumb_path.clone())
}

/// Shows `generation`'s picture in `picture` once it's decoded, unless `still_wanted` says the picture has moved on to
/// another wallpaper meanwhile (a recycled grid item, another mood).
pub fn show_picture(picture: &gtk::Picture, path: Option<String>, still_wanted: impl Fn() -> bool + 'static) {
    let Some(path) = path else {
        picture.set_paintable(None::<&gdk::Paintable>);
        return;
    };
    if let Some(texture) = TEXTURES.with(|cache| cache.borrow().get(&path).cloned()) {
        picture.set_paintable(Some(&texture));
        return;
    }
    picture.set_paintable(None::<&gdk::Paintable>);
    let picture = picture.downgrade();
    glib::spawn_future_local(async move {
        let texture = texture(path).await;
        if let (Some(picture), Some(texture)) = (picture.upgrade(), texture)
            && still_wanted()
        {
            picture.set_paintable(Some(&texture));
        }
    });
}

// ── Actions and their menu ──────────────────────────────────────────────────────────────────────────────────────

/// The menu every past wallpaper offers (History's items, a mood's wallpapers, the summary's), on the "item" actions.
pub fn item_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let main = gio::Menu::new();
    main.append(Some("Show on desktop"), Some("item.show"));
    menu.append_section(None, &main);
    let rating = gio::Menu::new();
    rating.append(Some("Like"), Some("item.like"));
    rating.append(Some("Dislike"), Some("item.dislike"));
    menu.append_section(None, &rating);
    let echoes = gio::Menu::new();
    echoes.append(Some("Make an echo"), Some("item.echo"));
    let lineage = gio::MenuItem::new(Some("Show original and echoes"), Some("item.lineage"));
    lineage.set_attribute_value("hidden-when", Some(&"action-disabled".to_variant()));
    echoes.append_item(&lineage);
    echoes.append(Some("Show in Files"), Some("item.open-folder"));
    menu.append_section(None, &echoes);
    let delete = gio::Menu::new();
    delete.append(Some("Delete…"), Some("item.delete"));
    menu.append_section(None, &delete);
    menu
}

/// Called before a wallpaper is deleted (History remembers where it was, so its neighbour takes keyboard focus) and
/// with whether it was.
pub type DeleteHook = Rc<dyn Fn(&Generation, Option<bool>)>;

/// The "item" actions for one wallpaper at a time (`set`). Insert `group()` on the widget whose menu uses them.
pub struct WallpaperActions {
    group: gio::SimpleActionGroup,
    app: Weak<App>,
    /// Where dialogs (Delete…, Original and echoes) are presented.
    anchor: glib::WeakRef<gtk::Widget>,
    generation: RefCell<Option<Generation>>,
    on_delete: RefCell<Option<DeleteHook>>,
}

impl WallpaperActions {
    pub fn new(app: &Rc<App>, anchor: &impl IsA<gtk::Widget>) -> Rc<Self> {
        let this = Rc::new(Self {
            group: gio::SimpleActionGroup::new(),
            app: Rc::downgrade(app),
            anchor: anchor.upcast_ref::<gtk::Widget>().downgrade(),
            generation: RefCell::new(None),
            on_delete: RefCell::new(None),
        });
        this.install();
        anchor.insert_action_group("item", Some(&this.group));
        this
    }

    /// Runs `hook` around every Delete… (History keeps keyboard focus near the wallpaper that went).
    pub fn on_delete(&self, hook: DeleteHook) {
        self.on_delete.replace(Some(hook));
    }

    pub fn generation(&self) -> Option<Generation> {
        self.generation.borrow().clone()
    }

    /// Runs one of the actions ("show", "delete", …) when it's enabled for the wallpaper now; true when it ran.
    pub fn activate(&self, name: &str) -> bool {
        match self.group.lookup_action(name) {
            Some(action) if action.is_enabled() => {
                action.activate(None);
                true
            }
            _ => false,
        }
    }

    /// The wallpaper the actions act on now, with whether it has an original or echoes to show.
    pub fn set(&self, generation: Option<&Generation>, has_echoes: bool) {
        self.generation.replace(generation.cloned());
        let Some(generation) = generation else {
            for name in ["show", "like", "dislike", "echo", "lineage", "open-folder", "delete"] {
                self.enable(name, false, None);
            }
            return;
        };
        let has_image = generation.image_path.as_deref().is_some_and(|path| std::path::Path::new(path).is_file());
        self.enable("show", has_image, None);
        self.enable("like", true, Some(generation.rating == Rating::Liked));
        self.enable("dislike", true, Some(generation.rating == Rating::Disliked));
        self.enable("echo", true, None);
        // Offered only when there's an original or an echo to show (the menu hides it while disabled).
        self.enable("lineage", has_echoes, None);
        self.enable("open-folder", generation.image_path.is_some(), None);
        self.enable("delete", true, None);
    }

    fn enable(&self, name: &str, enabled: bool, state: Option<bool>) {
        if let Some(action) = self.group.lookup_action(name).and_downcast::<gio::SimpleAction>() {
            action.set_enabled(enabled);
            if let Some(state) = state {
                action.set_state(&state.to_variant());
            }
        }
    }

    fn install(self: &Rc<Self>) {
        type Run = Box<dyn Fn(&Rc<WallpaperActions>, &Rc<App>, Generation)>;
        let add = |name: &str, stateful: bool, run: Run| {
            let action = if stateful {
                gio::SimpleAction::new_stateful(name, None, &false.to_variant())
            } else {
                gio::SimpleAction::new(name, None)
            };
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                let Some(this) = weak.upgrade() else { return };
                let (Some(app), Some(generation)) = (this.app.upgrade(), this.generation()) else { return };
                run(&this, &app, generation);
            });
            self.group.add_action(&action);
        };
        add("show", false, Box::new(|_, app, generation| app.show_on_desktop(generation)));
        add("like", true, Box::new(|_, app, generation| app.toggle_rating(&generation, Rating::Liked)));
        add("dislike", true, Box::new(|_, app, generation| app.toggle_rating(&generation, Rating::Disliked)));
        add(
            "echo",
            false,
            Box::new(|_, app, generation| {
                app.make_echo(generation.id.clone());
                // Where its progress (and Cancel) shows.
                if let Some(window) = app.main_window() {
                    window.show_now();
                }
            }),
        );
        add(
            "lineage",
            false,
            Box::new(|this, app, generation| {
                if let Some(anchor) = this.anchor.upgrade() {
                    show_lineage(app, &anchor, &generation);
                }
            }),
        );
        add(
            "open-folder",
            false,
            Box::new(|_, app, generation| {
                if let Some(path) = generation.image_path.as_deref() {
                    desktop::show_in_files(&PathBuf::from(path), app.window_widget().as_ref());
                }
            }),
        );
        add(
            "delete",
            false,
            Box::new(|this, app, generation| {
                let Some(anchor) = this.anchor.upgrade() else { return };
                let hook = this.on_delete.borrow().clone();
                confirm_delete(app, &anchor, generation, hook);
            }),
        );
    }
}

/// The context menu on `widget`: right click, long press (touch), Shift+F10 and Menu open `menu` at the pointer or the
/// widget's start. The popover is parented to the widget and let go of as the widget goes.
pub fn attach_menu(widget: &impl IsA<gtk::Widget>, menu: &gio::Menu) -> gtk::PopoverMenu {
    let widget = widget.upcast_ref::<gtk::Widget>();
    let popover = gtk::PopoverMenu::from_model(Some(menu));
    popover.set_parent(widget);
    popover.set_has_arrow(false);
    popover.set_halign(gtk::Align::Start);
    // A widget parented with set_parent isn't removed with its parent: GTK warns about a widget finalized with
    // children left.
    let held = popover.clone();
    widget.connect_destroy(move |_| held.unparent());
    let open = {
        let popover = popover.clone();
        move |x: f64, y: f64| {
            popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            popover.popup();
        }
    };
    let click = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
    let open_click = open.clone();
    click.connect_pressed(move |gesture, _, x, y| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        open_click(x, y);
    });
    widget.add_controller(click);
    let press = gtk::GestureLongPress::new();
    press.set_touch_only(true);
    let open_press = open.clone();
    press.connect_pressed(move |gesture, x, y| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        open_press(x, y);
    });
    widget.add_controller(press);
    let shortcuts = gtk::ShortcutController::new();
    shortcuts.set_scope(gtk::ShortcutScope::Local);
    let target = widget.downgrade();
    let open_keys = gtk::CallbackAction::new(move |_, _| {
        if let Some(widget) = target.upgrade() {
            open(f64::from(widget.width()) / 4.0, f64::from(widget.height()) / 2.0);
        }
        glib::Propagation::Stop
    });
    for keys in ["<Shift>F10", "Menu"] {
        if let Some(trigger) = gtk::ShortcutTrigger::parse_string(keys) {
            shortcuts.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(open_keys.clone())));
        }
    }
    widget.add_controller(shortcuts);
    popover
}

/// Opens the picture in the person's image viewer (Space in the gallery, like a quick look).
pub fn preview(app: &App, generation: &Generation) {
    let Some(path) = large_path(generation) else { return };
    let file = gio::File::for_path(path);
    let parent = app.window_widget();
    gtk::FileLauncher::new(Some(&file)).launch(parent.as_ref(), gio::Cancellable::NONE, |result| {
        if let Err(error) = result {
            tracing::warn!(%error, "couldn't open a wallpaper in the image viewer");
        }
    });
}

// ── A thumbnail that shows itself on the desktop ────────────────────────────────────────────────────────────────

/// A wallpaper's thumbnail as a button: a click, Return or Space shows it on the desktop (docs/app-spec.md: thumbnails
/// outside History); right click, long press, Shift+F10 or Menu opens History's menu. It speaks its title, with its
/// description, and says what a click does in its tooltip.
pub struct Thumbnail {
    button: gtk::Button,
    picture: gtk::Picture,
    actions: Rc<WallpaperActions>,
    id: Rc<RefCell<Option<String>>>,
}

impl Thumbnail {
    pub fn new(app: &Rc<App>, width: i32, height: i32) -> Rc<Self> {
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Cover)
            .width_request(width)
            .height_request(height)
            .can_shrink(true)
            .css_classes(["history-thumb"])
            .overflow(gtk::Overflow::Hidden)
            .accessible_role(gtk::AccessibleRole::Presentation)
            .build();
        // A picture asks for its image's own size; clamps hold it to the thumbnail's.
        let tall = adw::Clamp::builder().orientation(gtk::Orientation::Vertical).maximum_size(height).child(&picture).build();
        let wide = adw::Clamp::builder().maximum_size(width).tightening_threshold(width).child(&tall).build();
        let button = gtk::Button::builder().child(&wide).css_classes(["flat", "thumb-button"]).build();
        let actions = WallpaperActions::new(app, &button);
        attach_menu(&button, &item_menu());
        // The button's own handler holds the actions (not the button: no cycle), so they live exactly as long as it
        // does, whether or not anyone keeps this `Thumbnail`.
        let held = actions.clone();
        button.connect_clicked(move |_| {
            held.activate("show");
        });
        Rc::new(Self { button, picture, actions, id: Rc::new(RefCell::new(None)) })
    }

    pub fn widget(&self) -> &gtk::Button {
        &self.button
    }

    pub fn set(&self, item: &Item) {
        let generation = &item.generation;
        self.actions.set(Some(generation), item.has_echoes);
        let title = &generation.concept.title;
        self.button.set_tooltip_text(Some(&format!("{title}\nShow on desktop")));
        self.button.update_property(&[
            gtk::accessible::Property::Label(title),
            gtk::accessible::Property::Description(&item.spoken),
        ]);
        // A pruned original can't be shown on the desktop; the thumbnail stays, as in History.
        self.button.set_sensitive(true);
        self.id.replace(Some(generation.id.clone()));
        let id = self.id.clone();
        let wanted = generation.id.clone();
        show_picture(&self.picture, thumb_path(generation), move || id.borrow().as_deref() == Some(wanted.as_str()));
    }
}

/// A row of thumbnails (a mood's latest wallpapers): a flow box that wraps, so a narrow pane shows fewer per line.
pub fn thumbnail_strip(app: &Rc<App>, items: &[Item], width: i32, height: i32, label: &str) -> gtk::FlowBox {
    let strip = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .halign(gtk::Align::Start)
        .min_children_per_line(1)
        .max_children_per_line(8)
        .column_spacing(6)
        .row_spacing(6)
        .build();
    strip.update_property(&[gtk::accessible::Property::Label(label)]);
    for item in items {
        let thumbnail = Thumbnail::new(app, width, height);
        thumbnail.set(item);
        // The flow box child isn't a stop of its own: the button inside is.
        let child = gtk::FlowBoxChild::builder().child(thumbnail.widget()).focusable(false).build();
        strip.append(&child);
    }
    strip
}

// ── Original and echoes ─────────────────────────────────────────────────────────────────────────────────────────

/// "Show original and echoes": the wallpaper's lineage in a dialog, each with Show on desktop.
pub fn show_lineage(app: &Rc<App>, anchor: &gtk::Widget, generation: &Generation) {
    let Some(engine) = app.engine() else { return };
    let id = generation.id.clone();
    let (app, anchor) = (app.clone(), anchor.downgrade());
    glib::spawn_future_local(async move {
        let lineage = blocking(move || {
            let lineage = engine.lineage(id)?;
            Ok(lineage.into_iter().map(|generation| Item::read(&engine, generation)).collect::<Vec<_>>())
        })
        .await;
        match lineage {
            Ok(items) => {
                if let Some(anchor) = anchor.upgrade() {
                    present_lineage(&app, &anchor, items);
                }
            }
            Err(error) => app.toast(&strings::error_sentence(&error, app.state().settings.fallback)),
        }
    });
}

fn present_lineage(app: &Rc<App>, anchor: &gtk::Widget, items: Vec<Item>) {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .valign(gtk::Align::Start)
        .build();
    list.update_property(&[gtk::accessible::Property::Label("Original and echoes")]);
    let dialog = adw::Dialog::builder().title("Original and echoes").content_width(560).build();
    let lone = items.len() == 1;
    for item in items {
        let generation = item.generation.clone();
        let role = if generation.echo_of.is_none() { "Original" } else { "Echo" };
        let subtitle = match generation.echo_note.as_deref().filter(|note| !note.is_empty()) {
            Some(note) => format!("{} · {note}", strings::day_label(generation.created_at)),
            None => format!("{role} · {}", strings::day_label(generation.created_at)),
        };
        let row = adw::ActionRow::builder()
            .title(&generation.concept.title)
            .subtitle(&subtitle)
            .use_markup(false)
            .subtitle_lines(3)
            .build();
        row.update_property(&[gtk::accessible::Property::Description(&item.spoken)]);
        // A small fixed-size thumbnail (the row already speaks the title and description).
        let picture = gtk::Image::builder()
            .pixel_size(96)
            .valign(gtk::Align::Center)
            .css_classes(["history-thumb"])
            .overflow(gtk::Overflow::Hidden)
            .accessible_role(gtk::AccessibleRole::Presentation)
            .build();
        if let Some(path) = generation.thumb_path.clone() {
            let image = picture.downgrade();
            glib::spawn_future_local(async move {
                if let (Some(texture), Some(image)) = (texture(path).await, image.upgrade()) {
                    image.set_paintable(Some(&texture));
                }
            });
        }
        row.add_prefix(&picture);
        let show = gtk::Button::builder()
            .label("Show on desktop")
            .valign(gtk::Align::Center)
            .sensitive(generation.image_path.is_some())
            .build();
        // GTK names a labelled button by its text; the description says which wallpaper.
        show.update_property(&[gtk::accessible::Property::Description(&generation.concept.title)]);
        let app_weak = Rc::downgrade(app);
        let dialog_weak = dialog.downgrade();
        show.connect_clicked(move |_| {
            if let Some(app) = app_weak.upgrade() {
                app.show_on_desktop(generation.clone());
            }
            if let Some(dialog) = dialog_weak.upgrade() {
                dialog.close();
            }
        });
        row.add_suffix(&show);
        list.append(&row);
    }
    let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).build();
    column.append(&list);
    if lone {
        let note = gtk::Label::builder()
            .label("No echoes of it yet. An idea can come back once its quiet period has passed, or now with Make an echo.")
            .wrap(true)
            .xalign(0.0)
            .css_classes(["secondary-text"])
            .build();
        column.append(&note);
    }
    let clamp = adw::Clamp::builder().child(&column).margin_top(12).margin_bottom(24).margin_start(12).margin_end(12).build();
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .child(&clamp)
        .build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&scrolled));
    dialog.set_child(Some(&toolbar));
    dialog.present(Some(anchor));
}

// ── Delete… ─────────────────────────────────────────────────────────────────────────────────────────────────────

/// Asks before deleting a wallpaper, then deletes it (a toast says so, or why it couldn't). `hook` runs before the
/// delete (`None`) and after it (`Some(deleted)`).
pub fn confirm_delete(app: &Rc<App>, anchor: &gtk::Widget, generation: Generation, hook: Option<DeleteHook>) {
    let dialog = adw::AlertDialog::new(
        Some(&format!("Delete “{}”?", generation.concept.title)),
        Some("Its image is deleted and AutoPaper forgets it, so it may make something like it again."),
    );
    dialog.add_responses(&[("cancel", "_Cancel"), ("delete", "_Delete")]);
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    let (app, anchor) = (app.clone(), anchor.clone());
    glib::spawn_future_local(async move {
        if dialog.choose_future(Some(&anchor)).await != "delete" {
            return;
        }
        if let Some(hook) = &hook {
            hook(&generation, None);
        }
        let result = app.delete(generation.id.clone()).await;
        if let Some(hook) = &hook {
            hook(&generation, Some(result.is_ok()));
        }
        match result {
            Ok(()) => app.toast("Deleted"),
            Err(error) => app.toast(&strings::error_sentence(&error, app.state().settings.fallback)),
        }
    });
}

// ── Which models made it ────────────────────────────────────────────────────────────────────────────────────────

/// "Written by … · Painted by … · 3840×2160 · about $0.04" (docs/app-spec.md: Now and History's details), the painter a
/// link that opens Preferences → Providers, where painting is chosen. Screen readers hear the whole line as the
/// group's name, then the link.
pub struct ProvenanceLine {
    root: adw::WrapBox,
    written: gtk::Label,
    painted: gtk::LinkButton,
    tail: gtk::Label,
}

impl ProvenanceLine {
    pub fn new(app: &Rc<App>) -> Self {
        // The group's name says the whole line; its pieces are presentation (else the first is read again after it).
        let label = || {
            gtk::Label::builder()
                .xalign(0.0)
                .wrap(true)
                .selectable(false)
                .css_classes(["secondary-text", "caption"])
                .accessible_role(gtk::AccessibleRole::Presentation)
                .build()
        };
        let written = label();
        let tail = label();
        let painted = gtk::LinkButton::builder()
            .uri("autopaper:preferences/providers")
            .tooltip_text("Choose who paints in Preferences → Providers")
            .build();
        // Added to its own "link" class (setting the list would drop it, and the link would look like a button).
        painted.add_css_class("provenance-link");
        painted.add_css_class("caption");
        painted.update_property(&[gtk::accessible::Property::Description("Opens Preferences on the Providers page")]);
        let weak = Rc::downgrade(app);
        painted.connect_activate_link(move |_| {
            if let Some(app) = weak.upgrade() {
                app.open_preferences(Some("providers"));
            }
            glib::Propagation::Stop
        });
        let root = adw::WrapBox::builder().child_spacing(4).line_spacing(0).accessible_role(gtk::AccessibleRole::Group).build();
        root.append(&written);
        root.append(&painted);
        root.append(&tail);
        Self { root, written, painted, tail }
    }

    pub fn widget(&self) -> &adw::WrapBox {
        &self.root
    }

    pub fn set(&self, generation: &Generation) {
        let line = strings::Provenance::of(generation);
        self.written.set_label(&format!("{} ·", line.written));
        self.painted.set_label(&line.painted);
        let tail = line.tail();
        let tail = tail.trim_start_matches(' ');
        self.tail.set_label(tail);
        self.tail.set_visible(!tail.is_empty());
        self.root.update_property(&[gtk::accessible::Property::Label(&line.spoken())]);
        self.root.set_tooltip_text(Some(&line.text()));
    }
}
