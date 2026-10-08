//! First run (no provider beyond Demo, and the welcome not done yet): an AdwStatusPage in the window — "Tell your
//! computer what you'd like to see." — with three steps: a few keywords (examples are shown, never saved), who
//! writes and paints (OpenAI · Google Gemini · Local · Try it without AI), and a key when one is needed; then
//! Make my first wallpaper. Skip leaves everything as it is.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use autopaper_core::{KeywordWeight, ProviderKind, ProviderSelection, Trigger, default_base_url};
use gtk::glib;

use crate::app::App;
use crate::desktop::{self, APP_ID};
use crate::runtime::blocking;
use crate::strings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Choice {
    OpenAi,
    Gemini,
    Local,
    Demo,
}

const CHOICES: [(Choice, &str, &str); 4] = [
    (Choice::OpenAi, "OpenAI", "Needs an OpenAI API key. Each wallpaper costs a little; the budget starts at $5 a month."),
    (Choice::Gemini, "Google Gemini", "Needs a Google Gemini API key. Each wallpaper costs a little; the budget starts at $5 a month."),
    (Choice::Local, "Local", "Ollama writes and ComfyUI paints, on this computer. Free and private."),
    (Choice::Demo, "Try it without AI", "Colour gradients from your keywords, to see how AutoPaper works."),
];

pub struct Welcome {
    root: adw::ToolbarView,
    app: Weak<App>,
    keywords: RefCell<Vec<String>>,
    keyword_list: gtk::ListBox,
    add_row: adw::EntryRow,
    choice: RefCell<Choice>,
    key_group: adw::PreferencesGroup,
    key_row: adw::PasswordEntryRow,
    key_link: adw::ActionRow,
    local_row: adw::ActionRow,
    make: gtk::Button,
    /// Make my first wallpaper is saving: a second press is ignored (the button keeps focus rather than going
    /// insensitive under it).
    finishing: Cell<bool>,
    toasts: adw::ToastOverlay,
    on_done: Box<dyn Fn()>,
}

impl Welcome {
    /// Shown when nothing beyond Demo is set up and the welcome hasn't been finished or skipped.
    pub fn wanted(app: &App) -> bool {
        if app.prefs.boolean("first-run-done") {
            return false;
        }
        let state = app.state();
        state.settings.text_provider.kind == ProviderKind::Demo && state.settings.image_provider.kind == ProviderKind::Demo
    }

    pub fn new(app: &Rc<App>, on_done: impl Fn() + 'static) -> Rc<Self> {
        // Step 1: keywords.
        let add_row = adw::EntryRow::builder().title("Add a keyword").show_apply_button(true).build();
        let keyword_list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list-separate"])
            .build();
        keyword_list.update_property(&[gtk::accessible::Property::Label("Keywords")]);
        keyword_list.append(&add_row);
        let step1 = adw::PreferencesGroup::builder()
            .title("1. Add a few keywords")
            .description("For example: rain, ruins, peaceful, night, blue. Each one is a Must; you can change that later.")
            .build();
        step1.add(&keyword_list);

        // Step 2: who writes and paints.
        let choices = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        choices.update_property(&[gtk::accessible::Property::Label("Who writes and paints")]);
        let mut first: Option<gtk::CheckButton> = None;
        let mut buttons = Vec::new();
        for (choice, title, subtitle) in CHOICES {
            let check = gtk::CheckButton::builder().valign(gtk::Align::Center).build();
            if let Some(first) = &first {
                check.set_group(Some(first));
            } else {
                first = Some(check.clone());
            }
            let row = adw::ActionRow::builder()
                .title(title)
                .subtitle(subtitle)
                .activatable_widget(&check)
                .build();
            row.add_prefix(&check);
            choices.append(&row);
            buttons.push((choice, check));
        }
        let step2 = adw::PreferencesGroup::builder().title("2. Choose who writes and paints").build();
        step2.add(&choices);

        // Step 3: a key (OpenAI / Gemini), or local setup help.
        let key_row = adw::PasswordEntryRow::builder().title("OpenAI API key").build();
        let key_link = adw::ActionRow::builder().title("Get a key").activatable(true).build();
        key_link.add_suffix(&crate::history::decorative_icon("adw-external-link-symbolic"));
        // The addresses are the core's defaults (`default_base_url`), never copies of them.
        let local_subtitle = format!(
            "AutoPaper uses Ollama at {} and ComfyUI at {}; change them in Preferences. Not installed? Brew Browser \
             can set up Ollama (its Local LLMs bundle).",
            default_base_url(ProviderKind::Ollama).unwrap_or_default(),
            default_base_url(ProviderKind::ComfyUi).unwrap_or_default(),
        );
        let local_row = adw::ActionRow::builder()
            .title("Ollama and ComfyUI")
            .subtitle(glib::markup_escape_text(&local_subtitle).as_str())
            .activatable(true)
            .build();
        local_row.add_suffix(&crate::history::decorative_icon("adw-external-link-symbolic"));
        let key_group = adw::PreferencesGroup::builder().title("3. Add your key").build();
        key_group.add(&key_row);
        key_group.add(&key_link);
        key_group.add(&local_row);

        // Finishing (or skipping) the welcome asks to start AutoPaper at login: said here, and the person's choice.
        let login = adw::SwitchRow::builder()
            .title("Open at login")
            .subtitle("AutoPaper starts in the background so it can change your wallpaper on schedule. You can change this in Preferences.")
            .build();
        app.prefs.bind("open-at-login", &login, "active").build();
        let login_group = adw::PreferencesGroup::new();
        login_group.add(&login);

        let make = gtk::Button::builder()
            .label("_Make my first wallpaper")
            .use_underline(true)
            .css_classes(["pill", "suggested-action"])
            .halign(gtk::Align::Center)
            .sensitive(false)
            .build();
        let skip = gtk::Button::builder().label("_Skip").use_underline(true).css_classes(["flat"]).halign(gtk::Align::Center).build();
        let buttons_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(12)
            .build();
        buttons_box.append(&make);
        buttons_box.append(&skip);

        let steps = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(24).build();
        steps.append(&step1);
        steps.append(&step2);
        steps.append(&key_group);
        steps.append(&login_group);
        steps.append(&buttons_box);
        let clamp = adw::Clamp::builder().maximum_size(560).child(&steps).build();
        let status = adw::StatusPage::builder()
            .icon_name(APP_ID)
            .title("Tell your computer what you'd like to see.")
            .description("AutoPaper keeps making new wallpapers from a few words, remembers everything it has made, and learns what you like.")
            .child(&clamp)
            .build();
        let scrolled = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&status).build();
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&scrolled));
        let root = adw::ToolbarView::new();
        root.add_top_bar(&adw::HeaderBar::builder().title_widget(&adw::WindowTitle::new("Welcome to AutoPaper", "")).build());
        root.set_content(Some(&toasts));

        let this = Rc::new(Self {
            root,
            app: Rc::downgrade(app),
            keywords: RefCell::new(Vec::new()),
            keyword_list,
            add_row,
            choice: RefCell::new(Choice::OpenAi),
            key_group,
            key_row,
            key_link,
            local_row,
            make,
            finishing: Cell::new(false),
            toasts,
            on_done: Box::new(on_done),
        });
        for (choice, check) in &buttons {
            let choice = *choice;
            let weak = Rc::downgrade(&this);
            check.connect_toggled(move |check| {
                if let (true, Some(this)) = (check.is_active(), weak.upgrade()) {
                    this.choice.replace(choice);
                    this.show_choice();
                }
            });
        }
        if let Some((_, check)) = buttons.first() {
            check.set_active(true);
        }
        this.connect(&skip);
        this.show_choice();
        this
    }

    pub fn widget(&self) -> &adw::ToolbarView {
        &self.root
    }

    fn connect(self: &Rc<Self>, skip: &gtk::Button) {
        let weak = Rc::downgrade(self);
        self.add_row.connect_apply(move |row| {
            if let Some(this) = weak.upgrade() {
                this.add_keyword(row.text().to_string());
            }
        });
        let weak = Rc::downgrade(self);
        self.key_row.connect_changed(move |_| {
            if let Some(this) = weak.upgrade() {
                this.update_make();
            }
        });
        let weak = Rc::downgrade(self);
        self.key_link.connect_activated(move |row| {
            let Some(this) = weak.upgrade() else { return };
            let uri = match *this.choice.borrow() {
                Choice::Gemini => "https://aistudio.google.com/apikey",
                _ => "https://platform.openai.com/api-keys",
            };
            desktop::open_uri(uri, row.root().and_downcast::<gtk::Window>().as_ref());
        });
        self.local_row.connect_activated(|row| {
            let uri = if desktop::handles_scheme("brewbrowser") {
                "brewbrowser://bundle/local-llm"
            } else {
                "https://brew-browser.zerologic.com"
            };
            desktop::open_uri(uri, row.root().and_downcast::<gtk::Window>().as_ref());
        });
        let weak = Rc::downgrade(self);
        self.make.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.finish();
            }
        });
        let weak = Rc::downgrade(self);
        skip.connect_clicked(move |_| {
            let Some(this) = weak.upgrade() else { return };
            let Some(app) = this.app.upgrade() else { return };
            let _ = app.prefs.set_boolean("first-run-done", true);
            (this.on_done)();
            // Skipped: AutoPaper runs as set up so far (Demo until a provider is chosen in Preferences).
            glib::spawn_future_local(async move {
                app.apply_open_at_login().await;
                app.check_schedule();
            });
        });
    }

    fn add_keyword(self: &Rc<Self>, text: String) {
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            return;
        }
        if self.keywords.borrow().iter().any(|existing| existing.to_lowercase() == text.to_lowercase()) {
            self.toasts.add_toast(crate::window::toast(&format!("“{text}” is already in the list.")));
            return;
        }
        crate::keywords::clear_entry(&self.add_row);
        let row = adw::ActionRow::builder().title(glib::markup_escape_text(&text).as_str()).build();
        let remove = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .valign(gtk::Align::Center)
            .tooltip_text(format!("Remove {text}"))
            .css_classes(["flat"])
            .build();
        remove.update_property(&[gtk::accessible::Property::Label(&format!("Remove {text}"))]);
        row.add_suffix(&remove);
        self.keyword_list.append(&row);
        self.keywords.borrow_mut().push(text.clone());
        let weak = Rc::downgrade(self);
        let row_weak = row.downgrade();
        remove.connect_clicked(move |_| {
            let (Some(this), Some(row)) = (weak.upgrade(), row_weak.upgrade()) else { return };
            this.keyword_list.remove(&row);
            this.keywords.borrow_mut().retain(|existing| *existing != text);
            this.add_row.grab_focus();
            this.update_make();
        });
        self.root.announce(&format!("Added {}", self.keywords.borrow().last().cloned().unwrap_or_default()), gtk::AccessibleAnnouncementPriority::Medium);
        self.update_make();
    }

    fn show_choice(&self) {
        let choice = *self.choice.borrow();
        let needs_key = matches!(choice, Choice::OpenAi | Choice::Gemini);
        self.key_group.set_visible(choice != Choice::Demo);
        self.key_group.set_title(if needs_key { "3. Add your key" } else { "3. Check your local setup" });
        self.key_row.set_visible(needs_key);
        self.key_link.set_visible(needs_key);
        self.local_row.set_visible(choice == Choice::Local);
        self.key_row.set_title(if choice == Choice::Gemini { "Google Gemini API key" } else { "OpenAI API key" });
        self.key_link.set_subtitle(if choice == Choice::Gemini {
            "aistudio.google.com/apikey"
        } else {
            "platform.openai.com/api-keys"
        });
        self.update_make();
    }

    fn update_make(&self) {
        let choice = *self.choice.borrow();
        let has_key = !self.key_row.text().trim().is_empty();
        let ready = !self.keywords.borrow().is_empty() && (has_key || matches!(choice, Choice::Local | Choice::Demo));
        self.make.set_sensitive(ready);
    }

    /// Saves the keywords, the providers and the key, then makes the first wallpaper.
    fn finish(self: &Rc<Self>) {
        if self.finishing.get() {
            return;
        }
        let Some(app) = self.app.upgrade() else { return };
        let Some(engine) = app.engine() else { return };
        let choice = *self.choice.borrow();
        let keywords = self.keywords.borrow().clone();
        let key = self.key_row.text().trim().to_string();
        self.finishing.set(true);
        let this = self.clone();
        glib::spawn_future_local(async move {
            let added = blocking(move || {
                for keyword in keywords {
                    engine.add_keyword(keyword, KeywordWeight::Must)?;
                }
                Ok(())
            })
            .await;
            if let Err(error) = added {
                this.fail(&app, &error);
                return;
            }
            let account = match choice {
                Choice::OpenAi => Some("openai.api_key"),
                Choice::Gemini => Some("google.api_key"),
                Choice::Local | Choice::Demo => None,
            };
            if let Some(account) = account
                && let Err(problem) = app.save_key(account.into(), key).await {
                    tracing::warn!(problem, "couldn't save the key");
                    this.toasts.add_toast(crate::window::toast("Couldn't save the key to the keyring. Is it unlocked?"));
                    this.finishing.set(false);
                    return;
                }
            let (text, image) = match choice {
                Choice::OpenAi => (ProviderKind::OpenAi, ProviderKind::OpenAi),
                Choice::Gemini => (ProviderKind::Google, ProviderKind::Google),
                Choice::Local => (ProviderKind::Ollama, ProviderKind::ComfyUi),
                Choice::Demo => (ProviderKind::Demo, ProviderKind::Demo),
            };
            let changed = app
                .change_settings(move |settings| {
                    settings.text_provider = ProviderSelection { kind: text, model: String::new(), base_url: None };
                    settings.image_provider = ProviderSelection { kind: image, model: String::new(), base_url: None };
                })
                .await;
            if let Err(error) = changed {
                this.fail(&app, &error);
                return;
            }
            let _ = app.prefs.set_boolean("first-run-done", true);
            app.keywords_changed();
            (this.on_done)();
            app.new_wallpaper(Trigger::Manual);
            // Starting at login is the point of a wallpaper agent; the portal may ask (inside Flatpak).
            app.apply_open_at_login().await;
        });
    }

    fn fail(&self, app: &Rc<App>, error: &autopaper_core::AutoPaperError) {
        self.toasts.add_toast(crate::window::toast(&strings::error_sentence(error, app.state().settings.fallback)));
        self.finishing.set(false);
    }
}
