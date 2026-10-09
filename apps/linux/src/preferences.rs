//! Preferences (AdwPreferencesDialog): General, Providers, Keys, Memory, Budget. It reopens on the page shown
//! last. Changes save as they're made (GNOME has no Apply button); a change the engine refuses comes back with a
//! toast. About is AdwAboutDialog in the primary menu (GNOME's place for it), not a page here.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use autopaper_core::{
    AutoPaperError, InvalidInputReason, MemoryStatus, ModelInfo, ProviderJob, ProviderKind, ProviderSelection,
    ProviderUnavailableReason, Settings, StorageUsage, default_base_url, default_model, prices_as_of,
    secret_account_for,
};
use gtk::{gio, glib};

use crate::app::{App, Event};
use crate::desktop;
use crate::runtime::{blocking, spawn};
use crate::strings;

const OPENAI_KEYS: &str = "https://platform.openai.com/api-keys";
const GEMINI_KEYS: &str = "https://aistudio.google.com/apikey";
const BREW_BROWSER: &str = "https://brew-browser.zerologic.com";
const BREW_BROWSER_LOCAL_LLM: &str = "brewbrowser://bundle/local-llm";
const COMFYUI_INSTALL: &str = "https://docs.comfy.org/installation";
const CONSOLE_HELP: &str = "https://msitarzewski.github.io/AutoPaper/help.html#console";
const GEMINI_PRIVACY: &str = "A free Gemini key may let Google use what you send to improve its products; a paid project doesn't.";

/// Where a problem with a setting is fixed. The problem shows as a link (docs/app-spec.md 6a: "Add your OpenAI
/// key"), which opens Preferences on that page with the field focused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fix {
    /// The monthly limit that prevents a new wallpaper.
    Budget,
    /// A key field on the Keys page, by its keyring account (`secret_account_for`).
    Key(String),
    /// A field of the Writing ideas or Painting group on the Providers page.
    Provider(ProviderJob, ProviderField),
    /// A mood, by id: its keywords are where a keyword the writing model wouldn't follow is reworded, made a Maybe or
    /// removed. Not in Preferences: the main window shows the mood (`App::open_fix`).
    Mood(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderField {
    Provider,
    Model,
    Address,
    Workflow,
}

/// The dialog's pages; the dialog keeps them alive (and the window holds a weak reference, to go to a field).
pub struct Pages {
    _general: Rc<General>,
    providers: Rc<Providers>,
    keys: Rc<Keys>,
    _memory: Rc<Memory>,
    _budget: Rc<Budget>,
}

impl Pages {
    /// Shows the page that fixes a problem and focuses its field.
    pub fn show_fix(&self, dialog: &adw::PreferencesDialog, fix: &Fix) {
        let page = match fix {
            Fix::Key(_) => "keys",
            Fix::Budget => "budget",
            Fix::Provider(..) => "providers",
            Fix::Mood(_) => return,
        };
        dialog.set_visible_page_name(page);
        // Focus once the page has been laid out (and a key field just created for a new server exists).
        let (keys, providers, fix) = (Rc::downgrade(&self.keys), Rc::downgrade(&self.providers), fix.clone());
        glib::idle_add_local_once(move || {
            let target = match &fix {
                Fix::Key(account) => keys.upgrade().and_then(|keys| keys.focus(account)),
                Fix::Provider(job, field) => providers.upgrade().and_then(|providers| providers.focus(*job, *field)),
                Fix::Mood(_) | Fix::Budget => None,
            };
            if let Some(target) = target {
                show_fix_target(&target);
            }
        });
    }
}

/// The field a link went to: its focus ring drawn (GTK draws it only after keyboard navigation, and the link may have
/// been clicked or activated with Enter), and the row marked for a moment so it's seen at a glance.
fn show_fix_target(target: &gtk::Widget) {
    if let Some(window) = target.root().and_downcast::<gtk::Window>() {
        window.set_focus_visible(true);
    }
    target.add_css_class("fix-target");
    let weak = target.downgrade();
    glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || {
        if let Some(target) = weak.upgrade() {
            target.remove_css_class("fix-target");
        }
    });
}

/// Builds the dialog. Its pages live as long as it does.
pub fn build(app: &Rc<App>) -> (adw::PreferencesDialog, Rc<Pages>) {
    let dialog = adw::PreferencesDialog::new();
    let pages = Rc::new(Pages {
        _general: General::new(app, &dialog),
        providers: Providers::new(app, &dialog),
        keys: Keys::new(app, &dialog),
        _memory: Memory::new(app, &dialog),
        _budget: Budget::new(app, &dialog),
    });
    dialog.add(&pages._general.page);
    dialog.add(&pages.providers.page);
    dialog.add(&pages.keys.page);
    dialog.add(&pages._memory.page);
    dialog.add(&pages._budget.page);
    // Keep the pages (and their handlers) alive with the dialog. Closing it saves any key still waiting for typing
    // to pause, so a key pasted (or cleared) just before Escape is kept (or deleted).
    let weak_app = Rc::downgrade(app);
    let held = pages.clone();
    dialog.connect_closed(move |_| {
        let _ = &held;
        if let Some(app) = weak_app.upgrade() {
            app.flush_key_saves();
        }
    });
    (dialog, pages)
}

/// A refusal inside Preferences, worded for where the person already is.
fn toast_error(dialog: &adw::PreferencesDialog, _app: &App, error: &AutoPaperError) {
    dialog.add_toast(crate::window::toast(&strings::provider_problem(error)));
}

/// Saves a settings change; a refusal is toasted and the views go back to what's stored.
fn save(app: &Rc<App>, dialog: &adw::PreferencesDialog, change: impl FnOnce(&mut Settings) + 'static) {
    let app = app.clone();
    let dialog = dialog.downgrade();
    glib::spawn_future_local(async move {
        if let Err(error) = app.change_settings(change).await
            && let Some(dialog) = dialog.upgrade() {
                toast_error(&dialog, &app, &error);
            }
    });
}

fn combo(title: &str, subtitle: Option<&str>, labels: &[&str]) -> adw::ComboRow {
    let row = adw::ComboRow::builder().title(title).model(&gtk::StringList::new(labels)).build();
    if let Some(subtitle) = subtitle {
        row.set_subtitle(subtitle);
    }
    row
}

fn link_row(title: &str, uri: &'static str, shown: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(title).subtitle(shown).activatable(true).build();
    row.add_suffix(&crate::history::decorative_icon("adw-external-link-symbolic"));
    row.connect_activated(move |row| {
        let window = row.root().and_downcast::<gtk::Window>();
        desktop::open_uri(uri, window.as_ref());
    });
    row
}

/// Subscribes `update` to app events for as long as `owner` lives.
fn follow<T: 'static>(app: &Rc<App>, owner: &Rc<T>, update: impl Fn(&Rc<T>, Event) + 'static) {
    let weak = Rc::downgrade(owner);
    app.subscribe(move |event| match weak.upgrade() {
        Some(owner) => {
            update(&owner, event);
            true
        }
        None => false,
    });
}

// ── General ─────────────────────────────────────────────────────────────────────────────────────

struct General {
    page: adw::PreferencesPage,
    app: Weak<App>,
    cadence: adw::ComboRow,
    pause: adw::SwitchRow,
    replace: adw::SwitchRow,
    theme: adw::SwitchRow,
    fallback: adw::ComboRow,
    lock_screen: adw::SwitchRow,
    updating: Cell<bool>,
}

impl General {
    fn new(app: &Rc<App>, dialog: &adw::PreferencesDialog) -> Rc<Self> {
        let cadence_labels: Vec<&str> = strings::CADENCES.iter().map(|(_, label)| *label).collect();
        let cadence = combo("New wallpaper", None, &cadence_labels);
        let pause = adw::SwitchRow::builder()
            .title("Pause new wallpapers")
            .subtitle("The wallpaper showing stays. New wallpaper now still works.")
            .build();
        let replace = adw::SwitchRow::builder()
            .title("Replace wallpapers I dislike")
            .subtitle("A new one replaces it right away, within the budget.")
            .build();
        let theme = adw::SwitchRow::builder()
            .title("Match my appearance")
            .subtitle("New wallpapers suit light or dark mode, whichever your desktop is in. Your keywords still come first.")
            .build();
        let fallback_labels: Vec<&str> = strings::FALLBACKS.iter().map(|(_, label)| *label).collect();
        let fallback = combo(
            "When a new one can't be made",
            Some("When offline or a provider is down. A budget limit always keeps the current wallpaper."),
            &fallback_labels,
        );
        let making = adw::PreferencesGroup::builder().title("New wallpapers").build();
        making.add(&cadence);
        making.add(&pause);
        making.add(&replace);
        making.add(&theme);
        making.add(&fallback);

        let lock_screen = adw::SwitchRow::builder().title("Also set the lock screen").build();
        let desktop_group = adw::PreferencesGroup::builder().title("Desktop").build();
        if desktop::is_gnome() {
            // GNOME's lock screen always shows the desktop background, and the Wallpaper portal can't set it
            // separately; say so instead of offering a switch that can't work.
            let note = adw::ActionRow::builder()
                .title("Lock screen")
                .subtitle("GNOME's lock screen always shows your desktop wallpaper, so it changes too.")
                .build();
            note.add_prefix(&crate::history::decorative_icon("system-lock-screen-symbolic"));
            desktop_group.add(&note);
        } else {
            desktop_group.add(&lock_screen);
        }
        if !desktop::can_restore_wallpaper() {
            // Inside Flatpak and on desktops other than GNOME, AutoPaper can't read which wallpaper was there before
            // (docs/app-spec.md 3a): said here, rather than leaving the person to wonder where Restore my wallpaper is.
            // In Flatpak the sandbox is the reason (no portal says which wallpaper is showing, and AutoPaper's Flatpak
            // doesn't ask for the host's settings, dconf), not the desktop.
            let reason = if desktop::is_sandboxed() {
                "The Flatpak sandbox keeps your desktop's settings private, so AutoPaper can't bring back the wallpaper you \
                 had: quitting leaves its last one showing."
            } else {
                "AutoPaper can't bring back your own wallpaper on this desktop yet, so quitting leaves its last one showing."
            };
            let own = adw::ActionRow::builder().title("Your own wallpaper").subtitle(reason).build();
            own.add_prefix(&crate::history::decorative_icon("preferences-desktop-wallpaper-symbolic"));
            desktop_group.add(&own);
        }

        let login = adw::SwitchRow::builder()
            .title("Open at login")
            .subtitle("AutoPaper starts in the background, with no window.")
            .build();
        app.prefs.bind("open-at-login", &login, "active").build();
        let notify = adw::SwitchRow::builder()
            .title("Notify me about new wallpapers")
            .subtitle("With Like and Dislike, when the window isn't in front.")
            .build();
        app.prefs.bind("notify-new-wallpapers", &notify, "active").build();
        let running = adw::PreferencesGroup::builder().title("Running").build();
        running.add(&login);
        running.add(&notify);

        let page = adw::PreferencesPage::builder()
            .name("general")
            .title("General")
            .icon_name("emblem-system-symbolic")
            .build();
        page.add(&making);
        page.add(&desktop_group);
        page.add(&running);

        let this = Rc::new(Self {
            page,
            app: Rc::downgrade(app),
            cadence,
            pause,
            replace,
            theme,
            fallback,
            lock_screen,
            updating: Cell::new(false),
        });
        this.show();
        this.connect(app, dialog);
        let weak_app = Rc::downgrade(app);
        login.connect_active_notify(move |_| {
            if let Some(app) = weak_app.upgrade() {
                glib::spawn_future_local(async move { app.apply_open_at_login().await });
            }
        });
        follow(app, &this, |this, event| {
            if event == Event::Settings {
                this.show();
            }
        });
        this
    }

    fn show(&self) {
        let Some(app) = self.app.upgrade() else { return };
        let settings = app.state().settings.clone();
        self.updating.set(true);
        if let Some(index) = strings::CADENCES.iter().position(|(cadence, _)| *cadence == settings.cadence) {
            self.cadence.set_selected(index as u32);
        }
        self.pause.set_active(settings.paused);
        self.replace.set_active(settings.replace_disliked);
        self.theme.set_active(settings.match_system_theme);
        if let Some(index) = strings::FALLBACKS.iter().position(|(fallback, _)| *fallback == settings.fallback) {
            self.fallback.set_selected(index as u32);
        }
        if !desktop::is_gnome() {
            self.lock_screen.set_active(settings.set_lock_screen);
        }
        self.updating.set(false);
    }

    fn connect(self: &Rc<Self>, app: &Rc<App>, dialog: &adw::PreferencesDialog) {
        let ctx = (Rc::downgrade(self), Rc::downgrade(app), dialog.downgrade());
        let guard = move |run: &dyn Fn(&Rc<App>, &adw::PreferencesDialog)| {
            let (Some(this), Some(app), Some(dialog)) = (ctx.0.upgrade(), ctx.1.upgrade(), ctx.2.upgrade()) else {
                return;
            };
            if !this.updating.get() {
                run(&app, &dialog);
            }
        };
        let g = guard.clone();
        self.cadence.connect_selected_notify(move |row| {
            let selected = row.selected() as usize;
            g(&|app, dialog| {
                if let Some((cadence, _)) = strings::CADENCES.get(selected) {
                    let cadence = *cadence;
                    save(app, dialog, move |settings| settings.cadence = cadence);
                }
            });
        });
        let g = guard.clone();
        self.pause.connect_active_notify(move |row| {
            let paused = row.is_active();
            g(&|app, dialog| save(app, dialog, move |settings| settings.paused = paused));
        });
        let g = guard.clone();
        self.replace.connect_active_notify(move |row| {
            let replace = row.is_active();
            g(&|app, dialog| save(app, dialog, move |settings| settings.replace_disliked = replace));
        });
        let g = guard.clone();
        self.theme.connect_active_notify(move |row| {
            let theme = row.is_active();
            g(&|app, dialog| save(app, dialog, move |settings| settings.match_system_theme = theme));
        });
        let g = guard.clone();
        self.fallback.connect_selected_notify(move |row| {
            let selected = row.selected() as usize;
            g(&|app, dialog| {
                if let Some((fallback, _)) = strings::FALLBACKS.get(selected) {
                    let fallback = *fallback;
                    save(app, dialog, move |settings| settings.fallback = fallback);
                }
            });
        });
        let g = guard;
        self.lock_screen.connect_active_notify(move |row| {
            if desktop::is_gnome() {
                return;
            }
            let lock = row.is_active();
            g(&|app, dialog| save(app, dialog, move |settings| settings.set_lock_screen = lock));
        });
    }
}

// ── Providers ───────────────────────────────────────────────────────────────────────────────────

struct Providers {
    page: adw::PreferencesPage,
    writing: Rc<ProviderGroup>,
    painting: Rc<ProviderGroup>,
}

impl Providers {
    fn new(app: &Rc<App>, dialog: &adw::PreferencesDialog) -> Rc<Self> {
        let page = adw::PreferencesPage::builder()
            .name("providers")
            .title("Providers")
            .icon_name("network-server-symbolic")
            .description("Who writes each idea and who paints it. Keys go on the Keys page.")
            .build();
        let writing = ProviderGroup::new(app, dialog, ProviderJob::Concepts);
        let painting = ProviderGroup::new(app, dialog, ProviderJob::Images);
        page.add(&writing.group);
        page.add(&painting.group);
        Rc::new(Self { page, writing, painting })
    }

    /// Focuses a field of the Writing ideas or Painting group (the link of a problem it fixes); the field focused.
    fn focus(&self, job: ProviderJob, field: ProviderField) -> Option<gtk::Widget> {
        let group = match job {
            ProviderJob::Concepts => &self.writing,
            ProviderJob::Images => &self.painting,
        };
        let target: Option<gtk::Widget> = match field {
            ProviderField::Provider => Some(group.provider.clone().upcast()),
            ProviderField::Model => Some(group.model.clone().upcast()),
            ProviderField::Address => Some(group.address.clone().upcast()),
            ProviderField::Workflow => group.workflow.clone().map(|row| row.upcast()),
        };
        target.filter(|target| target.is_visible() && target.grab_focus())
    }
}

struct ProviderGroup {
    job: ProviderJob,
    kinds: Vec<ProviderKind>,
    app: Weak<App>,
    dialog: glib::WeakRef<adw::PreferencesDialog>,
    group: adw::PreferencesGroup,
    provider: adw::ComboRow,
    model: adw::ComboRow,
    model_ids: RefCell<Vec<String>>,
    /// The provider's models from `list_models` (empty until they load, or when they can't).
    models: RefCell<Vec<ModelInfo>>,
    /// The blank choice as shown: "Default (gpt-6-luna)".
    default_label: RefCell<String>,
    address: adw::EntryRow,
    quality: Option<adw::ComboRow>,
    /// ComfyUI: AutoPaper's workflow (one per model listed) or the person's own file.
    workflow: Option<adw::ComboRow>,
    /// The person's own workflow file, with Choose another….
    workflow_file: Option<adw::ActionRow>,
    /// With their own workflow, the model is the file's: a read-only line instead of the model list.
    model_line: Option<adw::ActionRow>,
    /// Each listed model's estimate in words, by its place in the list (the model list's second lines).
    estimates: Rc<ModelEstimates>,
    /// "Add your OpenAI key" / "Check your OpenAI key": the one place a key problem is said, as the link to the
    /// key's field (docs/app-spec.md 6a). A button row: screen readers hear a button, told where it goes.
    key_link: adw::ButtonRow,
    /// A key problem with this provider: `Some(false)` missing, `Some(true)` refused. Test and model Refresh are
    /// off until it's fixed; saving the key reloads the models, which checks it again.
    key_problem: Cell<Option<bool>>,
    /// The Test row: its subtitle is this provider's one status line (what Test checks, a test result, or why the
    /// models couldn't be listed).
    test: adw::ActionRow,
    test_button: gtk::Button,
    refresh: gtk::Button,
    /// The problem or test result on the status line, kept across settings refreshes (cleared with a new provider).
    status: RefCell<Option<String>>,
    help: adw::ActionRow,
    help_button: gtk::Button,
    gemini_note: adw::ActionRow,
    updating: Cell<bool>,
    /// What the model list was loaded for (`list_key`).
    models_for: RefCell<Option<ListKey>>,
    /// The status line says why the server address was refused (it goes back to the hint once the field is edited).
    address_problem_said: Cell<bool>,
    /// A test is running: Test stays sensitive (a focused button that goes insensitive leaves focus nowhere, and
    /// Escape no longer closes the dialog), so a second press is ignored instead.
    testing: Cell<bool>,
}

/// What a model list belongs to: the provider, its server, and (ComfyUI) whether the person's own workflow is in use,
/// since AutoPaper's workflows list their models and the person's own lists its loader's.
type ListKey = (ProviderKind, Option<String>, bool);

impl ProviderGroup {
    /// The model list's key for the settings now.
    fn list_key(&self) -> Option<ListKey> {
        let app = self.app.upgrade()?;
        let selection = self.selection()?;
        let own = selection.kind == ProviderKind::ComfyUi
            && app.state().settings.comfyui_workflow.as_deref().is_some_and(|text| !text.trim().is_empty());
        Some((selection.kind, selection.base_url, own))
    }

    fn new(app: &Rc<App>, dialog: &adw::PreferencesDialog, job: ProviderJob) -> Rc<Self> {
        let painting = job == ProviderJob::Images;
        let kinds: Vec<ProviderKind> = [
            ProviderKind::OpenAi,
            ProviderKind::Google,
            ProviderKind::Ollama,
            ProviderKind::OpenAiCompatible,
            ProviderKind::ComfyUi,
            ProviderKind::Demo,
        ]
        .into_iter()
        .filter(|kind| if painting { kind.makes_images() } else { kind.writes_concepts() })
        .collect();
        let labels: Vec<&str> = kinds
            .iter()
            .map(|kind| match kind {
                ProviderKind::OpenAi => "OpenAI",
                ProviderKind::Google => "Google Gemini",
                ProviderKind::Ollama => "Ollama",
                ProviderKind::OpenAiCompatible => "OpenAI-compatible",
                ProviderKind::ComfyUi => "ComfyUI",
                ProviderKind::Demo => "Demo (no AI)",
            })
            .collect();
        let title = if painting { "Painting" } else { "Writing ideas" };
        let group = adw::PreferencesGroup::builder()
            .title(title)
            .description(if painting {
                "Paints the idea as an image."
            } else {
                "Turns your keywords into an idea for a wallpaper."
            })
            .build();
        let provider = combo("Provider", None, &labels);
        provider.update_property(&[gtk::accessible::Property::Description(title)]);
        let model = combo("Model", None, &["Default"]);
        // The list shows each model's usual time on this computer under its name, once there is one.
        let estimates = Rc::new(ModelEstimates::default());
        model.set_list_factory(Some(&model_list_factory(estimates.clone())));
        let refresh = gtk::Button::builder()
            .icon_name("view-refresh-symbolic")
            .valign(gtk::Align::Center)
            .tooltip_text("Refresh the list of models")
            .css_classes(["flat"])
            .build();
        refresh.update_property(&[gtk::accessible::Property::Label("Refresh the list of models")]);
        model.add_suffix(&refresh);
        let address = adw::EntryRow::builder().title("Server address").show_apply_button(true).build();
        let quality = painting.then(|| {
            let labels: Vec<&str> = strings::QUALITIES.iter().map(|(_, label)| *label).collect();
            combo("Painting quality", Some("High costs more with hosted providers."), &labels)
        });
        let (workflow, workflow_file, model_line) = if painting {
            let workflow = combo("Workflow", None, &[WORKFLOW_AUTOPAPER, WORKFLOW_OWN]);
            let file = adw::ActionRow::builder().title("Workflow file").use_markup(false).subtitle_selectable(true).build();
            let choose = gtk::Button::builder().label("Choose another…").valign(gtk::Align::Center).build();
            choose.update_property(&[gtk::accessible::Property::Description("Choose another workflow file")]);
            file.add_suffix(&choose);
            let weak_dialog = dialog.downgrade();
            let weak_app = Rc::downgrade(app);
            choose.connect_clicked(move |_| {
                if let (Some(app), Some(dialog)) = (weak_app.upgrade(), weak_dialog.upgrade()) {
                    choose_workflow(app, dialog);
                }
            });
            let line = adw::ActionRow::builder().title("Model").use_markup(false).subtitle_selectable(true).build();
            (Some(workflow), Some(file), Some(line))
        } else {
            (None, None, None)
        };
        // Its subtitle names the server's address and the provider's answer: plain text, never markup.
        let test = adw::ActionRow::builder().title("Test").use_markup(false).build();
        let test_button = gtk::Button::builder().label("Test").valign(gtk::Align::Center).build();
        test_button.update_property(&[gtk::accessible::Property::Description(&format!("Tests the {} provider", title.to_lowercase()))]);
        test.add_suffix(&test_button);
        let help = adw::ActionRow::builder().title("Setting up local models").build();
        let help_button = gtk::Button::builder().valign(gtk::Align::Center).build();
        help.add_suffix(&help_button);
        let gemini_note = adw::ActionRow::builder().title("Free and paid keys").subtitle(GEMINI_PRIVACY).build();
        gemini_note.add_prefix(&crate::history::decorative_icon("dialog-information-symbolic"));
        // A button row with GNOME's arrow for "go there": activating it opens the key's field on the Keys page.
        let key_link = adw::ButtonRow::builder()
            .use_markup(false)
            .start_icon_name("dialog-warning-symbolic")
            .end_icon_name("go-next-symbolic")
            .visible(false)
            .build();
        key_link.update_property(&[gtk::accessible::Property::Description("Opens the key's field on the Keys page")]);

        group.add(&provider);
        group.add(&key_link);
        if let Some(workflow) = &workflow {
            group.add(workflow);
        }
        if let Some(file) = &workflow_file {
            group.add(file);
        }
        group.add(&model);
        if let Some(line) = &model_line {
            group.add(line);
        }
        group.add(&address);
        if let Some(quality) = &quality {
            group.add(quality);
        }
        group.add(&test);
        group.add(&help);
        group.add(&gemini_note);

        let this = Rc::new(Self {
            job,
            kinds,
            app: Rc::downgrade(app),
            dialog: dialog.downgrade(),
            group,
            provider,
            model,
            model_ids: RefCell::new(vec![String::new()]),
            models: RefCell::new(Vec::new()),
            default_label: RefCell::new("Default".into()),
            address,
            quality,
            workflow,
            workflow_file,
            model_line,
            estimates,
            key_link,
            key_problem: Cell::new(None),
            test,
            test_button,
            refresh,
            status: RefCell::new(None),
            help,
            help_button,
            gemini_note,
            updating: Cell::new(false),
            models_for: RefCell::new(None),
            address_problem_said: Cell::new(false),
            testing: Cell::new(false),
        });
        this.show();
        this.connect();
        follow(app, &this, |this, event| match event {
            Event::Settings => this.show(),
            // A key was saved or deleted: the view checks it again by listing the models.
            Event::Keys if this.selection().is_some_and(|selection| takes_key(selection.kind)) => this.load_models(false),
            // A wallpaper was made: what it took is part of the estimates now.
            Event::History => this.load_estimates(),
            _ => {}
        });
        this
    }

    /// Each listed model's usual time here (`estimate`, learned on this computer, `None` until a call like it has
    /// finished): under its name in the list, and the chosen one's as the Model row's subtitle.
    fn load_estimates(self: &Rc<Self>) {
        let (Some(app), Some(selection)) = (self.app.upgrade(), self.selection()) else { return };
        let Some(engine) = app.engine() else { return };
        if selection.kind == ProviderKind::Demo {
            return;
        }
        let job = self.job;
        let ids = self.model_ids.borrow().clone();
        let weak = Rc::downgrade(self);
        let asked = selection.clone();
        let asked_key = self.list_key();
        let asked_ids = ids.clone();
        glib::spawn_future_local(async move {
            // The estimates follow the saved settings too (a ComfyUI workflow's steps): after any change being saved.
            let _ = app.serial(|_| Ok(())).await;
            let estimates = blocking(move || {
                Ok(asked_ids
                    .iter()
                    .map(|id| {
                        let selection = ProviderSelection { model: id.clone(), ..asked.clone() };
                        engine.estimate(selection, job, 0, 0).ok().flatten()
                    })
                    .collect::<Vec<Option<u32>>>())
            })
            .await;
            let (Some(this), Ok(estimates)) = (weak.upgrade(), estimates) else { return };
            let Some(current) = this.selection() else { return };
            if this.list_key() != asked_key || ids != *this.model_ids.borrow() {
                return; // The list changed meanwhile; its own estimates are on their way.
            }
            let lines: Vec<String> = estimates
                .iter()
                .map(|seconds| seconds.map(|seconds| strings::estimate_line(job, current.kind, seconds)).unwrap_or_default())
                .collect();
            let chosen = this.model_ids.borrow().iter().position(|id| *id == current.model.trim()).unwrap_or(0);
            this.model.set_subtitle(lines.get(chosen).map(String::as_str).unwrap_or(""));
            this.estimates.set(lines);
        });
    }

    /// The keyring account of this provider's key (`None` for providers without one, and for an OpenAI-compatible
    /// server without a valid address yet).
    fn key_account(&self) -> Option<String> {
        self.selection().and_then(secret_account_for)
    }

    /// Shows a key problem as the link to its field, or clears it. Test and Refresh can't work until it's fixed, so
    /// they're off meanwhile; when one of them has focus, focus moves to the link first (never nowhere).
    fn set_key_problem(&self, problem: Option<bool>) {
        let kind = self.selection().map(|selection| selection.kind);
        let problem = problem.filter(|_| self.key_account().is_some());
        self.key_problem.set(problem);
        if let (Some(refused), Some(kind)) = (problem, kind) {
            self.key_link.set_title(&strings::key_link(kind, refused));
            self.key_link.set_visible(true);
            let focus = self.group.root().and_then(|root| gtk::prelude::RootExt::focus(&root));
            if [&self.test_button, &self.refresh].iter().any(|button| focus.as_ref() == Some(button.upcast_ref())) {
                self.key_link.grab_focus();
            }
        } else {
            self.key_link.set_visible(false);
        }
        self.test_button.set_sensitive(problem.is_none());
        self.refresh.set_sensitive(problem.is_none());
    }

    /// The status line: a problem or test result, else what Test checks.
    fn set_status(&self, text: Option<String>) {
        let shown = match (&text, self.selection()) {
            (Some(text), _) => text.clone(),
            (None, Some(selection)) => test_hint(&selection),
            (None, None) => String::new(),
        };
        self.status.replace(text);
        self.test.set_subtitle(&shown);
    }

    fn selection(&self) -> Option<ProviderSelection> {
        let app = self.app.upgrade()?;
        let settings = &app.state().settings;
        Some(match self.job {
            ProviderJob::Concepts => settings.text_provider.clone(),
            ProviderJob::Images => settings.image_provider.clone(),
        })
    }

    fn show(self: &Rc<Self>) {
        let Some(app) = self.app.upgrade() else { return };
        let Some(selection) = self.selection() else { return };
        let settings = app.state().settings.clone();
        let kind = selection.kind;
        self.updating.set(true);
        if let Some(index) = self.kinds.iter().position(|k| *k == kind) {
            self.provider.set_selected(index as u32);
        }
        self.provider.set_subtitle(match kind {
            ProviderKind::Demo if self.job == ProviderJob::Images => "Colour gradients, no AI: for trying AutoPaper.",
            ProviderKind::Demo => "Simple ideas from your keywords, no AI: for trying AutoPaper.",
            ProviderKind::OpenAi | ProviderKind::Google => "Hosted: needs a key, costs a little per wallpaper.",
            ProviderKind::Ollama | ProviderKind::ComfyUi => "Runs on this computer: free and private.",
            ProviderKind::OpenAiCompatible => "Your own server, such as LM Studio or LocalAI.",
        });
        let own = settings.comfyui_workflow.as_deref().is_some_and(|text| !text.trim().is_empty());
        let comfy = kind == ProviderKind::ComfyUi;
        // With their own ComfyUI workflow the model is the file's: a read-only line instead of the list.
        self.model.set_visible(kind != ProviderKind::Demo && !(comfy && own));
        let uses_address = matches!(kind, ProviderKind::Ollama | ProviderKind::OpenAiCompatible | ProviderKind::ComfyUi);
        self.address.set_visible(uses_address);
        let address = selection.base_url.clone().unwrap_or_default();
        // An address the engine refused stays in the field, marked, until the person corrects it.
        if self.address.text() != address && !self.address.has_css_class("error") {
            self.address.set_text(&address);
        }
        if let Some(text) = self.address.delegate().and_downcast::<gtk::Text>() {
            let placeholder = default_base_url(kind).unwrap_or("https://…");
            text.set_placeholder_text(Some(placeholder));
        }
        self.address.update_property(&[gtk::accessible::Property::Description(&match default_base_url(kind) {
            Some(default) => format!("Leave empty for {default}"),
            None => "The server's address, starting with https:// or http://".to_string(),
        })]);
        if let Some(quality) = &self.quality {
            quality.set_visible(kind != ProviderKind::Demo && kind != ProviderKind::ComfyUi);
            if let Some(index) = strings::QUALITIES.iter().position(|(q, _)| *q == settings.image_quality) {
                quality.set_selected(index as u32);
            }
        }
        if let (Some(workflow), Some(file), Some(line)) = (&self.workflow, &self.workflow_file, &self.model_line) {
            workflow.set_visible(comfy);
            workflow.set_selected(if own { 1 } else { 0 });
            workflow.set_subtitle(if own {
                "AutoPaper fills in its {{prompt}}, {{width}}, {{height}} and {{seed}}."
            } else {
                "Tested with each model listed, each with its own encoder and settings."
            });
            file.set_visible(comfy && own);
            line.set_visible(comfy && own);
            if comfy && own {
                let name = app.prefs.string("comfyui-workflow-name");
                file.set_subtitle(if name.is_empty() { "Your own workflow" } else { name.as_str() });
                let reading = settings.comfyui_workflow.as_deref().map(read_workflow);
                line.set_subtitle(&own_model_line(reading.as_ref()));
            }
        }
        self.test.set_visible(kind != ProviderKind::Demo);
        let status = self.status.borrow().clone();
        self.test.set_subtitle(&status.unwrap_or_else(|| test_hint(&selection)));
        self.gemini_note.set_visible(kind == ProviderKind::Google);
        match kind {
            ProviderKind::Ollama => {
                self.help.set_visible(true);
                self.help.set_subtitle(
                    "Not installed? Brew Browser can set it up: its Local LLMs bundle installs Ollama. Models are \
                     separate downloads.",
                );
                self.help_button.set_label(if desktop::handles_scheme("brewbrowser") {
                    "Open in Brew Browser"
                } else {
                    "Get Brew Browser…"
                });
            }
            ProviderKind::ComfyUi => {
                self.help.set_visible(true);
                self.help.set_subtitle("Not installed? ComfyUI's own guide covers Linux. Models are separate downloads.");
                self.help_button.set_label("Installation guide");
            }
            _ => self.help.set_visible(false),
        }
        self.updating.set(false);
        let label = strings::default_model_label(kind, self.job, own);
        let key = self.list_key();
        if kind == ProviderKind::Demo {
            // Demo has no models, key or server: nothing from the provider before it stays.
            self.models_for.replace(None);
            self.set_key_problem(None);
            self.set_status(None);
        } else if *self.models_for.borrow() != key {
            self.models_for.replace(key);
            // Another provider, server or ComfyUI workflow: its own status, and no key problem until its models say so.
            self.set_status(None);
            self.set_key_problem(None);
            // The models listed for the one before don't belong here (AutoPaper's workflows list theirs, the person's
            // own its loader's): the default alone until its own list loads.
            self.default_label.replace(label);
            self.fill_models(&[], &selection.model);
            self.load_models(false);
        } else {
            self.select_model(&selection.model);
            // Another model was chosen: its own estimate.
            let chosen = self.model_ids.borrow().iter().position(|id| *id == selection.model.trim()).unwrap_or(0);
            self.model.set_subtitle(&self.estimates.line(chosen as u32));
        }
    }

    fn select_model(&self, model: &str) {
        let ids = self.model_ids.borrow();
        let index = ids.iter().position(|id| id == model.trim()).unwrap_or(0);
        self.updating.set(true);
        self.model.set_selected(index as u32);
        self.updating.set(false);
    }

    /// Fills the model list from the provider (`list_models`). Without a key or a running server it keeps
    /// the blank choice, "Default (…)" (plus the chosen model), and says why.
    fn load_models(self: &Rc<Self>, announce: bool) {
        let (Some(app), Some(selection)) = (self.app.upgrade(), self.selection()) else { return };
        let Some(engine) = app.engine() else { return };
        let job = self.job;
        let weak = Rc::downgrade(self);
        let asked_for = self.list_key();
        let asked = selection.clone();
        glib::spawn_future_local(async move {
            // After any settings change still being saved: ComfyUI lists the models of the workflow in the saved
            // settings (AutoPaper's, or the person's own), and the Workflow choice that asked for this list may not have
            // landed yet.
            let _ = app.serial(|_| Ok(())).await;
            let models = spawn(async move { engine.list_models(asked, job).await }).await;
            let Some(this) = weak.upgrade() else { return };
            let Some(current) = this.selection() else { return };
            if this.list_key() != asked_for {
                return; // Another provider, server or workflow was chosen meanwhile; its own list is on its way.
            }
            // A problem is said once, on the status line (or as the key link), never also on the model row.
            match models {
                Ok(models) => {
                    this.fill_models(&models, &current.model);
                    this.load_estimates();
                    this.set_key_problem(None);
                    // A refused address the person is correcting stays said until they edit it.
                    if this.status.borrow().is_some() && !this.address_problem_said.get() {
                        this.set_status(None);
                    }
                    if announce {
                        this.model.announce(
                            &format!("{} models", models.len()),
                            gtk::AccessibleAnnouncementPriority::Medium,
                        );
                    }
                }
                Err(error) => {
                    this.fill_models(&[], &current.model);
                    this.load_estimates();
                    tracing::info!(problem = %strings::provider_problem(&error), "couldn't list the models");
                    match key_problem(&error) {
                        Some(refused) => {
                            this.set_status(None);
                            this.set_key_problem(Some(refused));
                        }
                        None => {
                            this.set_key_problem(None);
                            if !this.address_problem_said.get() {
                                this.set_status(Some(strings::provider_problem(&error)));
                            }
                        }
                    }
                }
            }
        });
    }

    fn fill_models(&self, models: &[ModelInfo], current: &str) {
        self.models.replace(models.to_vec());
        // Lines by place in the old list don't fit the new one: they come back with `load_estimates`.
        self.estimates.set(Vec::new());
        let mut ids = vec![String::new()];
        // The blank choice names the default in words once the provider has listed it ("Default (Z-Image Turbo)"
        // rather than its file name).
        let default_id = self.selection().map(|selection| default_model(selection.kind, self.job)).unwrap_or_default();
        let named_default = models
            .iter()
            .find(|model| !default_id.is_empty() && model.id == default_id && !model.display_name.is_empty())
            .map(|model| format!("Default ({})", model.display_name));
        let own_workflow = self
            .app
            .upgrade()
            .is_some_and(|app| app.state().settings.comfyui_workflow.as_deref().is_some_and(|text| !text.trim().is_empty()));
        let mut labels = vec![named_default.filter(|_| !own_workflow).unwrap_or_else(|| self.default_label.borrow().clone())];
        for model in models {
            ids.push(model.id.clone());
            labels.push(if model.display_name.is_empty() { model.id.clone() } else { model.display_name.clone() });
        }
        let current = current.trim();
        if !current.is_empty() && !ids.iter().any(|id| id == current) {
            ids.push(current.to_string());
            labels.push(current.to_string());
        }
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        self.updating.set(true);
        self.model.set_model(Some(&gtk::StringList::new(&refs)));
        self.updating.set(false);
        self.model_ids.replace(ids);
        self.select_model(current);
    }

    fn connect(self: &Rc<Self>) {
        let (refresh, test_button) = (&self.refresh, &self.test_button);
        let weak = Rc::downgrade(self);
        self.key_link.connect_activated(move |_| {
            let Some(this) = weak.upgrade() else { return };
            if let (Some(app), Some(account)) = (this.app.upgrade(), this.key_account()) {
                app.open_fix(Fix::Key(account));
            }
        });
        let weak = Rc::downgrade(self);
        self.provider.connect_selected_notify(move |row| {
            let Some(this) = weak.upgrade() else { return };
            if this.updating.get() {
                return;
            }
            let Some(kind) = this.kinds.get(row.selected() as usize).copied() else { return };
            let job = this.job;
            this.clear_address_problem();
            // Trying another provider for a moment doesn't lose what was set for this one: its server address and
            // model are kept, and the provider chosen gets back its own.
            let Some(app) = this.app.upgrade() else { return };
            if let Some(leaving) = this.selection() {
                remember_selection(&app, job, &leaving);
            }
            let selection = recalled_selection(&app, job, kind);
            this.change(move |settings| match job {
                ProviderJob::Concepts => settings.text_provider = selection,
                ProviderJob::Images => settings.image_provider = selection,
            });
        });
        let weak = Rc::downgrade(self);
        self.model.connect_selected_notify(move |row| {
            let Some(this) = weak.upgrade() else { return };
            if this.updating.get() {
                return;
            }
            let Some(model) = this.model_ids.borrow().get(row.selected() as usize).cloned() else { return };
            let job = this.job;
            this.change(move |settings| match job {
                ProviderJob::Concepts => settings.text_provider.model = model,
                ProviderJob::Images => settings.image_provider.model = model,
            });
        });
        let weak = Rc::downgrade(self);
        self.address.connect_apply(move |row| {
            let Some(this) = weak.upgrade() else { return };
            let text = row.text().trim().to_string();
            let base_url = (!text.is_empty()).then_some(text);
            this.save_address(base_url);
        });
        let weak = Rc::downgrade(self);
        self.address.connect_changed(move |_| {
            if let Some(this) = weak.upgrade()
                && !this.updating.get()
            {
                this.clear_address_problem();
            }
        });
        if let Some(workflow) = &self.workflow {
            let weak = Rc::downgrade(self);
            workflow.connect_selected_notify(move |row| {
                let Some(this) = weak.upgrade() else { return };
                if this.updating.get() {
                    return;
                }
                let (Some(app), Some(dialog)) = (this.app.upgrade(), this.dialog.upgrade()) else { return };
                if row.selected() == 0 {
                    save(&app, &dialog, |settings| settings.comfyui_workflow = None);
                    return;
                }
                // Their own: the file used before (kept while AutoPaper's was chosen), else choose one now.
                let kept = app.prefs.string("comfyui-workflow-text").to_string();
                match (kept.trim().is_empty(), read_workflow(&kept)) {
                    (false, WorkflowReading::Model(_)) => save(&app, &dialog, move |settings| settings.comfyui_workflow = Some(kept)),
                    _ => choose_workflow(app, dialog),
                }
            });
        }
        if let Some(quality) = &self.quality {
            let weak = Rc::downgrade(self);
            quality.connect_selected_notify(move |row| {
                let Some(this) = weak.upgrade() else { return };
                if this.updating.get() {
                    return;
                }
                if let Some((quality, _)) = strings::QUALITIES.get(row.selected() as usize) {
                    let quality = *quality;
                    this.change(move |settings| settings.image_quality = quality);
                }
            });
        }
        let weak = Rc::downgrade(self);
        refresh.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.load_models(true);
            }
        });
        let weak = Rc::downgrade(self);
        test_button.connect_clicked(move |button| {
            if let Some(this) = weak.upgrade() {
                this.run_test(button);
            }
        });
        let weak = Rc::downgrade(self);
        self.help_button.connect_clicked(move |button| {
            let Some(this) = weak.upgrade() else { return };
            let window = button.root().and_downcast::<gtk::Window>();
            match this.selection().map(|selection| selection.kind) {
                Some(ProviderKind::Ollama) if desktop::handles_scheme("brewbrowser") => {
                    desktop::open_uri(BREW_BROWSER_LOCAL_LLM, window.as_ref())
                }
                Some(ProviderKind::Ollama) => desktop::open_uri(BREW_BROWSER, window.as_ref()),
                Some(ProviderKind::ComfyUi) => desktop::open_uri(COMFYUI_INSTALL, window.as_ref()),
                _ => {}
            }
        });
    }

    fn change(self: &Rc<Self>, change: impl FnOnce(&mut Settings) + 'static) {
        let (Some(app), Some(dialog)) = (self.app.upgrade(), self.dialog.upgrade()) else { return };
        save(&app, &dialog, change);
    }

    /// Saves the server address; one the engine refuses (`InvalidInput` with an address reason) stays in the
    /// field, marked, with the reason in words.
    fn save_address(self: &Rc<Self>, base_url: Option<String>) {
        let Some(app) = self.app.upgrade() else { return };
        let job = self.job;
        let weak = Rc::downgrade(self);
        let typed = base_url.clone().unwrap_or_default();
        glib::spawn_future_local(async move {
            let result = app
                .change_settings(move |settings| match job {
                    ProviderJob::Concepts => settings.text_provider.base_url = base_url,
                    ProviderJob::Images => settings.image_provider.base_url = base_url,
                })
                .await;
            let Some(this) = weak.upgrade() else { return };
            let Some(dialog) = this.dialog.upgrade() else { return };
            match result {
                Ok(()) => this.clear_address_problem(),
                Err(error) if strings::is_address_problem(&error) => {
                    // The refusal put the stored address back in the field; give the person theirs to correct. Why it
                    // was refused goes on the status line, where it stays until the field is edited (a toast would cut
                    // it short and go).
                    this.updating.set(true);
                    this.address.set_text(&typed);
                    this.updating.set(false);
                    this.mark_address_problem();
                    let reason = strings::provider_problem(&error);
                    this.set_status(Some(reason.clone()));
                    this.address_problem_said.set(true);
                    this.address.announce(&reason, gtk::AccessibleAnnouncementPriority::Medium);
                }
                Err(error) => toast_error(&dialog, &app, &error),
            }
        });
    }

    /// Marks the address field as the problem: red, and "invalid" to screen readers.
    fn mark_address_problem(&self) {
        self.address.add_css_class("error");
        if let Some(text) = self.address.delegate() {
            text.update_state(&[gtk::accessible::State::Invalid(gtk::AccessibleInvalidState::True)]);
        }
    }

    fn clear_address_problem(&self) {
        if self.address_problem_said.replace(false) {
            self.set_status(None);
        }
        if !self.address.has_css_class("error") {
            return;
        }
        self.address.remove_css_class("error");
        if let Some(text) = self.address.delegate() {
            text.update_state(&[gtk::accessible::State::Invalid(gtk::AccessibleInvalidState::False)]);
        }
    }

    fn run_test(self: &Rc<Self>, _button: &gtk::Button) {
        if self.testing.get() {
            return;
        }
        let (Some(app), Some(selection)) = (self.app.upgrade(), self.selection()) else { return };
        let Some(engine) = app.engine() else { return };
        self.testing.set(true);
        self.test.set_subtitle("Testing…");
        self.help_button.remove_css_class("suggested-action");
        let job = self.job;
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let kind = selection.kind;
            let result = spawn(async move { engine.test_provider(selection, job).await }).await;
            let Some(this) = weak.upgrade() else { return };
            this.testing.set(false);
            // A key problem is said once, as the link to the key (not as a Test error too).
            if let Some(refused) = result.as_ref().err().and_then(key_problem) {
                this.set_status(None);
                this.set_key_problem(Some(refused));
                let link = strings::key_link(kind, refused);
                this.test.announce(&format!("{link}."), gtk::AccessibleAnnouncementPriority::Medium);
                return;
            }
            let text = match &result {
                Ok(()) => format!("{} works.", strings::provider_name(kind)),
                Err(error) => strings::provider_problem(error),
            };
            if result.is_ok() {
                this.set_key_problem(None);
            }
            this.set_status(Some(text.clone()));
            this.test.announce(&text, gtk::AccessibleAnnouncementPriority::Medium);
            match &result {
                // Nothing answers at the address: point at the setup help.
                Err(AutoPaperError::ProviderUnavailable { reason: ProviderUnavailableReason::NotRunning, .. })
                    if kind.is_local() =>
                {
                    this.help_button.add_css_class("suggested-action")
                }
                // The address itself is the problem: mark the field (the Test row already says why, until it's edited).
                Err(error) if strings::is_address_problem(error) => {
                    this.mark_address_problem();
                    this.address_problem_said.set(true);
                }
                _ => {}
            }
        });
    }
}

/// Whether an error is a key problem, and if so whether the key was refused (`true`) or is missing (`false`).
fn key_problem(error: &AutoPaperError) -> Option<bool> {
    match error {
        AutoPaperError::MissingKey { .. } => Some(false),
        AutoPaperError::InvalidKey { .. } => Some(true),
        _ => None,
    }
}

/// Where a provider's last model and server address are kept while another is chosen ("images.ComfyUi.model").
fn recent_key(job: ProviderJob, kind: ProviderKind, field: &str) -> String {
    let job = match job {
        ProviderJob::Concepts => "writing",
        ProviderJob::Images => "painting",
    };
    format!("{job}.{kind:?}.{field}")
}

/// Keeps `selection`'s model and server address (the `recent-providers` setting) for when its provider is chosen again.
fn remember_selection(app: &App, job: ProviderJob, selection: &ProviderSelection) {
    let mut recent = app.prefs.value("recent-providers").get::<std::collections::HashMap<String, String>>().unwrap_or_default();
    recent.insert(recent_key(job, selection.kind, "model"), selection.model.trim().to_string());
    recent.insert(recent_key(job, selection.kind, "address"), selection.base_url.as_deref().unwrap_or("").trim().to_string());
    if let Err(error) = app.prefs.set_value("recent-providers", &recent.to_variant()) {
        tracing::warn!(%error, "couldn't keep the provider's model and address");
    }
}

/// `kind` as it was last chosen for `job`: its model and server address, or its defaults.
fn recalled_selection(app: &App, job: ProviderJob, kind: ProviderKind) -> ProviderSelection {
    let recent = app.prefs.value("recent-providers").get::<std::collections::HashMap<String, String>>().unwrap_or_default();
    let field = |name: &str| recent.get(&recent_key(job, kind, name)).map(|value| value.trim().to_string()).unwrap_or_default();
    let address = field("address");
    ProviderSelection { kind, model: field("model"), base_url: (!address.is_empty()).then_some(address) }
}

/// Providers that take a key (hosted ones always; an OpenAI-compatible server when it asks for one).
fn takes_key(kind: ProviderKind) -> bool {
    matches!(kind, ProviderKind::OpenAi | ProviderKind::Google | ProviderKind::OpenAiCompatible)
}

/// What Test checks, naming the address (so the default shows even while the address field is empty).
fn test_hint(selection: &ProviderSelection) -> String {
    let address = selection
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .or_else(|| default_base_url(selection.kind));
    match (selection.kind, address) {
        (ProviderKind::OpenAi | ProviderKind::Google, _) => "Checks your key with a free call.".into(),
        (kind, Some(address)) => format!("Checks {} at {address} with a free call.", strings::provider_in_sentence(kind)),
        (_, None) => "Enter the server's address, then test it.".into(),
    }
}

const WORKFLOW_AUTOPAPER: &str = "AutoPaper's";
/// It opens a file chooser (when there's no file of theirs kept from before): GNOME's ellipsis for "asks for more".
const WORKFLOW_OWN: &str = "Your own…";

/// A ComfyUI workflow file of the person's own, chosen with the file chooser: one in API format with {{prompt}} where
/// the scene goes. A file that can't work is refused now, with the reason, rather than at the next wallpaper. Used,
/// it's kept (with its name) so it's one choice away after switching back to AutoPaper's. Cancelled, nothing changes
/// (the Workflow row goes back to what's in use).
fn choose_workflow(app: Rc<App>, dialog: adw::PreferencesDialog) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("ComfyUI workflows (JSON)"));
    filter.add_mime_type("application/json");
    filter.add_suffix("json");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let chooser = gtk::FileDialog::builder()
        .title("Choose a ComfyUI workflow")
        .accept_label("Use")
        .filters(&filters)
        .default_filter(&filter)
        .modal(true)
        .build();
    let window = dialog.root().and_downcast::<gtk::Window>();
    glib::spawn_future_local(async move {
        let Ok(file) = chooser.open_future(window.as_ref()).await else {
            // Cancelled: show what's in use again.
            app.emit(Event::Settings);
            return;
        };
        let name = file.basename().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let contents = match file.load_contents_future().await {
            Ok((bytes, _)) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(error) => {
                tracing::warn!(%error, "couldn't read a workflow file");
                app.emit(Event::Settings);
                refuse_workflow(app, dialog, &name, "AutoPaper couldn't read the file.".into());
                return;
            }
        };
        if let WorkflowReading::Problem(reason) = read_workflow(&contents) {
            app.emit(Event::Settings);
            refuse_workflow(app, dialog, &name, strings::invalid_input(reason, strings::Context::Preferences));
            return;
        }
        let _ = app.prefs.set_string("comfyui-workflow-name", &name);
        let _ = app.prefs.set_string("comfyui-workflow-text", &contents);
        match app.change_settings(move |settings| settings.comfyui_workflow = Some(contents)).await {
            Ok(()) => {}
            Err(AutoPaperError::InvalidInput { reason, .. })
                if matches!(
                    reason,
                    InvalidInputReason::WorkflowNeedsPrompt
                        | InvalidInputReason::WorkflowNotApiFormat
                        | InvalidInputReason::WorkflowInvalid
                ) =>
            {
                refuse_workflow(app, dialog, &name, strings::invalid_input(reason, strings::Context::Preferences));
            }
            Err(error) => toast_error(&dialog, &app, &error),
        }
    });
}

/// A workflow file that can't be used, and why, in an alert: the reason is a sentence or two about what to do in
/// ComfyUI, which a toast would cut short and take away. Choose another… opens the file chooser again; Close leaves
/// the Workflow row on what's in use.
fn refuse_workflow(app: Rc<App>, dialog: adw::PreferencesDialog, name: &str, reason: String) {
    let heading = if name.is_empty() { "Can't use this workflow".to_string() } else { format!("Can't use “{name}”") };
    let alert = adw::AlertDialog::new(Some(&heading), Some(&reason));
    alert.add_responses(&[("close", "_Close"), ("choose", "_Choose another…")]);
    alert.set_response_appearance("choose", adw::ResponseAppearance::Suggested);
    alert.set_default_response(Some("choose"));
    alert.set_close_response("close");
    glib::spawn_future_local(async move {
        if alert.choose_future(Some(&dialog)).await == "choose" {
            choose_workflow(app, dialog);
        }
    });
}

/// What a person's own ComfyUI workflow is, read as the core reads it when it paints (`fill_placeholders`,
/// `api_graph` and `loader` in core/src/providers/comfyui.rs).
#[derive(Debug, Clone, PartialEq, Eq)]
enum WorkflowReading {
    /// Usable; the model file its first loader loads (`ckpt_name`, else `unet_name`, by node id), if any.
    Model(Option<String>),
    /// Can't be used, and why (`WorkflowNeedsPrompt`, `WorkflowNotApiFormat`, `WorkflowInvalid`).
    Problem(InvalidInputReason),
}

fn read_workflow(text: &str) -> WorkflowReading {
    use serde_json::Value;
    if !text.contains("{{prompt}}") {
        return WorkflowReading::Problem(InvalidInputReason::WorkflowNeedsPrompt);
    }
    let mut filled = text.to_string();
    for name in ["width", "height", "seed"] {
        let placeholder = format!("{{{{{name}}}}}");
        filled = filled.replace(&format!("\"{placeholder}\""), "1").replace(&placeholder, "1");
    }
    let filled = filled.replace("{{prompt}}", "");
    let Ok(value) = serde_json::from_str::<Value>(&filled) else {
        return WorkflowReading::Problem(InvalidInputReason::WorkflowInvalid);
    };
    let is_node = |value: &Value| {
        value.get("class_type").is_some_and(Value::is_string) && value.get("inputs").is_none_or(Value::is_object)
    };
    let not_api = WorkflowReading::Problem(InvalidInputReason::WorkflowNotApiFormat);
    let Value::Object(mut root) = value else { return not_api };
    if root.get("nodes").is_some_and(Value::is_array) {
        return not_api;
    }
    if let Some(Value::Object(inner)) = root.get("prompt")
        && !root.values().all(is_node)
    {
        root = inner.clone();
    }
    if root.is_empty() || !root.values().all(is_node) {
        return not_api;
    }
    let mut ids: Vec<&String> = root.keys().collect();
    ids.sort_by_key(|id| (id.parse::<u64>().unwrap_or(u64::MAX), id.as_str()));
    let model = ["ckpt_name", "unet_name"].iter().find_map(|input| {
        ids.iter().find_map(|id| root.get(*id)?.get("inputs")?.get(*input)?.as_str().map(str::to_string))
    });
    WorkflowReading::Model(model)
}

/// The read-only Model line for a person's own workflow: "sd_xl_base_1.0 (from your workflow)".
fn own_model_line(reading: Option<&WorkflowReading>) -> String {
    match reading {
        Some(WorkflowReading::Model(Some(file))) => {
            let name = [".safetensors", ".ckpt", ".gguf", ".pt", ".pth", ".bin"]
                .iter()
                .find_map(|extension| file.strip_suffix(extension))
                .unwrap_or(file);
            format!("{name} (from your workflow)")
        }
        _ => "Set in your workflow".into(),
    }
}

/// Each listed model's estimate line, by its place in the model list, and the list items showing them: new
/// estimates (after a wallpaper, or once the models load) update the items already bound, which GTK doesn't bind again.
#[derive(Default)]
struct ModelEstimates {
    lines: RefCell<Vec<String>>,
    bound: RefCell<Vec<(glib::WeakRef<gtk::ListItem>, glib::WeakRef<gtk::Label>)>>,
}

impl ModelEstimates {
    fn line(&self, position: u32) -> String {
        self.lines.borrow().get(position as usize).cloned().unwrap_or_default()
    }

    fn set(&self, lines: Vec<String>) {
        self.lines.replace(lines);
        self.bound.borrow_mut().retain(|(item, label)| item.upgrade().is_some() && label.upgrade().is_some());
        for (item, label) in self.bound.borrow().iter() {
            if let (Some(item), Some(label)) = (item.upgrade(), label.upgrade()) {
                show_estimate(&label, &self.line(item.position()));
            }
        }
    }
}

fn show_estimate(label: &gtk::Label, line: &str) {
    label.set_label(line);
    label.set_visible(!line.is_empty());
}

/// The model list's items: the model's name, under it its usual time here when there is one, and the checkmark on
/// the chosen one (as libadwaita's own combo row items have).
fn model_list_factory(estimates: Rc<ModelEstimates>) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else { return };
        let name = gtk::Label::builder().xalign(0.0).build();
        let estimate = gtk::Label::builder().xalign(0.0).css_classes(["secondary-text"]).visible(false).build();
        let lines = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).build();
        lines.append(&name);
        lines.append(&estimate);
        let check = gtk::Image::builder()
            .icon_name("object-select-symbolic")
            .accessible_role(gtk::AccessibleRole::Presentation)
            .opacity(0.0)
            .build();
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.append(&lines);
        row.append(&check);
        item.set_child(Some(&row));
        let check = check.downgrade();
        item.connect_selected_notify(move |item| {
            if let Some(check) = check.upgrade() {
                check.set_opacity(if item.is_selected() { 1.0 } else { 0.0 });
            }
        });
    });
    let shared = estimates.clone();
    factory.connect_bind(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else { return };
        let Some(row) = item.child() else { return };
        let lines = row.first_child();
        let name = lines.as_ref().and_then(|lines| lines.first_child()).and_downcast::<gtk::Label>();
        let estimate = lines.as_ref().and_then(|lines| lines.last_child()).and_downcast::<gtk::Label>();
        let (Some(name), Some(estimate)) = (name, estimate) else { return };
        let text = item.item().and_downcast::<gtk::StringObject>().map(|object| object.string().to_string()).unwrap_or_default();
        name.set_label(&text);
        show_estimate(&estimate, &shared.line(item.position()));
        if let Some(check) = row.last_child() {
            check.set_opacity(if item.is_selected() { 1.0 } else { 0.0 });
        }
        shared.bound.borrow_mut().push((item.downgrade(), estimate.downgrade()));
    });
    factory.connect_unbind(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else { return };
        estimates.bound.borrow_mut().retain(|(bound, _)| bound.upgrade().is_some_and(|bound| bound != *item));
    });
    factory
}

// ── Keys ────────────────────────────────────────────────────────────────────────────────────────

struct Keys {
    page: adw::PreferencesPage,
    app: Weak<App>,
    compatible: adw::PreferencesGroup,
    compatible_fields: RefCell<Vec<(String, Rc<KeyField>, adw::PasswordEntryRow)>>,
    compatible_empty: adw::ActionRow,
    _fields: Vec<Rc<KeyField>>,
}

/// A key field that saves as you type (after a short pause), like AudioPaper's Keychain fields. The pause is
/// the app's (`App::schedule_key_save`), so closing Preferences right after typing still saves.
struct KeyField {
    account: String,
    row: adw::PasswordEntryRow,
    app: Weak<App>,
    dialog: glib::WeakRef<adw::PreferencesDialog>,
    loading: Cell<bool>,
}

impl KeyField {
    fn new(app: &Rc<App>, dialog: &adw::PreferencesDialog, account: String, title: &str) -> Rc<Self> {
        // The title can name a server's address: plain text, never markup.
        let row = adw::PasswordEntryRow::builder().title(title).use_markup(false).build();
        let field = Rc::new(Self {
            account,
            row,
            app: Rc::downgrade(app),
            dialog: dialog.downgrade(),
            loading: Cell::new(true),
        });
        let weak = Rc::downgrade(&field);
        field.row.connect_changed(move |_| {
            if let Some(field) = weak.upgrade() {
                field.schedule_save();
            }
        });
        field.load();
        field
    }

    fn load(self: &Rc<Self>) {
        let Some(app) = self.app.upgrade() else { return };
        let account = self.account.clone();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let stored = app.read_key(account).await;
            let Some(field) = weak.upgrade() else { return };
            match stored {
                Ok(value) => field.row.set_text(value.as_deref().unwrap_or("")),
                Err(problem) => {
                    tracing::warn!(problem, "couldn't read a key");
                    if let Some(dialog) = field.dialog.upgrade() {
                        dialog.add_toast(crate::window::toast("Couldn't read your keys from the keyring."));
                    }
                }
            }
            field.loading.set(false);
        });
    }

    fn schedule_save(self: &Rc<Self>) {
        if self.loading.get() {
            return;
        }
        let (Some(app), Some(dialog)) = (self.app.upgrade(), self.dialog.upgrade()) else { return };
        app.schedule_key_save(self.account.clone(), self.row.text().to_string(), &dialog);
    }
}

impl Keys {
    fn new(app: &Rc<App>, dialog: &adw::PreferencesDialog) -> Rc<Self> {
        let page = adw::PreferencesPage::builder()
            .name("keys")
            .title("Keys")
            .icon_name("dialog-password-symbolic")
            .description("Keys are kept in your keyring and sent only to their own service.")
            .build();
        let openai_field = KeyField::new(app, dialog, "openai.api_key".into(), "OpenAI API key");
        let openai = adw::PreferencesGroup::builder().title("OpenAI").build();
        openai.add(&openai_field.row);
        openai.add(&link_row("Get a key", OPENAI_KEYS, "platform.openai.com/api-keys"));
        let gemini_field = KeyField::new(app, dialog, "google.api_key".into(), "Google Gemini API key");
        let gemini = adw::PreferencesGroup::builder().title("Google Gemini").description(GEMINI_PRIVACY).build();
        gemini.add(&gemini_field.row);
        gemini.add(&link_row("Get a key", GEMINI_KEYS, "aistudio.google.com/apikey"));
        let compatible = adw::PreferencesGroup::builder()
            .title("OpenAI-compatible server")
            .description("A key belongs to its server's address, so a new address never gets an old key.")
            .build();
        let compatible_empty = adw::ActionRow::builder()
            .title("No OpenAI-compatible server is chosen")
            .subtitle("Choose one in Providers; its key goes here.")
            .build();
        compatible.add(&compatible_empty);
        page.add(&openai);
        page.add(&gemini);
        page.add(&compatible);
        let this = Rc::new(Self {
            page,
            app: Rc::downgrade(app),
            compatible,
            compatible_fields: RefCell::new(Vec::new()),
            compatible_empty,
            _fields: vec![openai_field, gemini_field],
        });
        this.show_compatible(dialog);
        let dialog_weak = dialog.downgrade();
        follow(app, &this, move |this, event| {
            if event == Event::Settings
                && let Some(dialog) = dialog_weak.upgrade() {
                    this.show_compatible(&dialog);
                }
        });
        this
    }

    /// Focuses the field of a key (the link of a missing or refused key goes here); the field focused.
    fn focus(&self, account: &str) -> Option<gtk::Widget> {
        let fixed = self._fields.iter().find(|field| field.account == account).map(|field| field.row.clone());
        let compatible = || {
            self.compatible_fields.borrow().iter().find(|(held, _, _)| held == account).map(|(_, _, row)| row.clone())
        };
        fixed.or_else(compatible).filter(|row| row.grab_focus()).map(|row| row.upcast())
    }

    /// One key field per OpenAI-compatible server in use ("Key for http://192.168.1.20:1234").
    fn show_compatible(&self, dialog: &adw::PreferencesDialog) {
        let Some(app) = self.app.upgrade() else { return };
        let settings = app.state().settings.clone();
        let mut accounts: Vec<(String, String)> = Vec::new();
        for selection in [&settings.text_provider, &settings.image_provider] {
            if selection.kind != ProviderKind::OpenAiCompatible {
                continue;
            }
            if let Some(account) = secret_account_for(selection.clone())
                && !accounts.iter().any(|(existing, _)| *existing == account) {
                    let address = selection.base_url.clone().unwrap_or_default();
                    accounts.push((account, address));
                }
        }
        let same = {
            let fields = self.compatible_fields.borrow();
            fields.len() == accounts.len() && fields.iter().zip(&accounts).all(|((a, _, _), (b, _))| a == b)
        };
        if same {
            return;
        }
        for (_, _, row) in self.compatible_fields.take() {
            self.compatible.remove(&row);
        }
        let mut fields = Vec::new();
        for (account, address) in accounts {
            let field = KeyField::new(&app, dialog, account.clone(), &format!("Key for {}", address.trim()));
            self.compatible.add(&field.row);
            let row = field.row.clone();
            fields.push((account, field, row));
        }
        self.compatible_empty.set_visible(fields.is_empty());
        self.compatible_fields.replace(fields);
    }
}

// ── Memory ──────────────────────────────────────────────────────────────────────────────────────

struct Memory {
    page: adw::PreferencesPage,
    app: Weak<App>,
    /// What AutoPaper has learned from ratings (taste is shared by every mood).
    learned: adw::PreferencesGroup,
    liked_row: adw::ActionRow,
    disliked_row: adw::ActionRow,
    reset: gtk::Button,
    quiet: adw::ComboRow,
    echoes: adw::ComboRow,
    status: adw::ActionRow,
    usage: adw::ActionRow,
    limit: adw::ComboRow,
    limit_values: RefCell<Vec<u32>>,
    updating: Cell<bool>,
}

impl Memory {
    fn new(app: &Rc<App>, dialog: &adw::PreferencesDialog) -> Rc<Self> {
        let quiet_labels: Vec<&str> = strings::QUIET_PERIODS.iter().map(|(_, label)| *label).collect();
        let quiet = combo("Quiet period", Some("How long before a similar idea may appear again."), &quiet_labels);
        let echo_labels: Vec<&str> = strings::ECHOES.iter().map(|(_, label)| *label).collect();
        let echoes = combo(
            "Echoes",
            Some("How often an old idea, past its quiet period, comes back in a new form."),
            &echo_labels,
        );
        let status = adw::ActionRow::builder().title("Memory").use_markup(false).subtitle_selectable(true).build();
        let memory = adw::PreferencesGroup::builder()
            .title("Memory")
            .description("AutoPaper remembers every idea it has made, even after the image is deleted, so it doesn't repeat itself.")
            .build();
        memory.add(&quiet);
        memory.add(&echoes);
        memory.add(&status);

        // Taste: what ratings taught it, shared by every mood. The lists are the providers' own words: plain text.
        let liked_row = adw::ActionRow::builder().title("Liked").use_markup(false).build();
        let disliked_row = adw::ActionRow::builder().title("Disliked").use_markup(false).build();
        let reset = gtk::Button::builder().label("Reset…").valign(gtk::Align::Center).css_classes(["flat"]).build();
        // Its visible text names it ("Reset…"); the description says what it resets.
        reset.update_property(&[gtk::accessible::Property::Description("Forget what AutoPaper has learned")]);
        let learned = adw::PreferencesGroup::builder()
            .title("What AutoPaper has learned")
            .header_suffix(&reset)
            .build();
        learned.add(&liked_row);
        learned.add(&disliked_row);

        let usage = adw::ActionRow::builder().title("Images on disk").subtitle("…").build();
        let limit = combo("Keep up to", Some("Older images go first; liked ones and their memory stay."), &[]);
        let clear = adw::ButtonRow::builder().title("Clear history…").build();
        // Added, not set: the builder's css_classes would replace libadwaita's own button-row classes.
        clear.add_css_class("destructive-action");
        let storage = adw::PreferencesGroup::builder().title("Storage").build();
        storage.add(&usage);
        storage.add(&limit);
        storage.add(&clear);

        let console = adw::PreferencesGroup::builder()
            .title("Console history")
            .description("The Console records generation attempts on this device, including failed and blocked runs. It keeps the newest 200 runs for up to 30 days; active runs are kept until they finish.")
            .build();
        let privacy = adw::ActionRow::builder()
            .title("Local run details")
            .subtitle("Prompts, models, requests, responses and retries stay on this device. Credentials and image data are omitted. Copying or exporting is your choice.")
            .subtitle_selectable(true)
            .use_markup(false)
            .build();
        let earlier = adw::ActionRow::builder()
            .title("Earlier wallpapers")
            .subtitle("Requests from before Console recording began cannot be reconstructed. Earlier runs may have no model timings. Their previously saved details remain in History.")
            .subtitle_selectable(true)
            .use_markup(false)
            .build();
        console.add(&privacy);
        console.add(&earlier);
        console.add(&link_row("Console help", CONSOLE_HELP, "How run outcomes, charts and exports work"));

        let page = adw::PreferencesPage::builder()
            .name("memory")
            .title("Memory")
            .icon_name("document-open-recent-symbolic")
            .build();
        page.add(&learned);
        page.add(&memory);
        page.add(&storage);
        page.add(&console);
        let this = Rc::new(Self {
            page,
            app: Rc::downgrade(app),
            learned,
            liked_row,
            disliked_row,
            reset: reset.clone(),
            quiet,
            echoes,
            status,
            usage,
            limit,
            limit_values: RefCell::new(Vec::new()),
            updating: Cell::new(false),
        });
        this.show();
        this.load_status();
        this.load_taste();
        this.connect(app, dialog, &clear, &reset);
        follow(app, &this, |this, event| match event {
            Event::Settings => this.show(),
            Event::History => this.load_status(),
            Event::Taste => this.load_taste(),
            _ => {}
        });
        this
    }

    /// The features ratings taught AutoPaper to prefer and avoid (`taste_summary`).
    fn load_taste(&self) {
        let Some(engine) = self.app.upgrade().and_then(|app| app.engine()) else { return };
        let (liked_row, disliked_row, learned, reset) =
            (self.liked_row.clone(), self.disliked_row.clone(), self.learned.clone(), self.reset.clone());
        glib::spawn_future_local(async move {
            match blocking(move || engine.taste_summary()).await {
                Ok(summary) => {
                    let list = |items: &[String]| if items.is_empty() { "Nothing yet".to_string() } else { items.join(", ") };
                    liked_row.set_subtitle(&list(&summary.liked));
                    disliked_row.set_subtitle(&list(&summary.disliked));
                    learned.set_description(Some(&match summary.ratings {
                        0 => "Like or dislike a few wallpapers and AutoPaper learns what you enjoy, in every mood.".to_string(),
                        1 => "From 1 rating, in every mood.".to_string(),
                        n => format!("From {n} ratings, in every mood."),
                    }));
                    // Taste counts exist from the first rating, before any feature shows in the lists.
                    reset.set_sensitive(summary.ratings > 0);
                }
                Err(error) => tracing::warn!(%error, "couldn't read what AutoPaper has learned"),
            }
        });
    }

    fn show(&self) {
        let Some(app) = self.app.upgrade() else { return };
        let settings = app.state().settings.clone();
        self.updating.set(true);
        if let Some(index) = strings::QUIET_PERIODS.iter().position(|(q, _)| *q == settings.quiet_period) {
            self.quiet.set_selected(index as u32);
        }
        if let Some(index) = strings::ECHOES.iter().position(|(e, _)| *e == settings.echoes) {
            self.echoes.set_selected(index as u32);
        }
        let mut values: Vec<u32> = strings::STORAGE_LIMITS.iter().map(|(mb, _)| *mb).collect();
        let mut labels: Vec<String> = strings::STORAGE_LIMITS.iter().map(|(_, label)| label.to_string()).collect();
        if !values.contains(&settings.storage_limit_mb) {
            values.push(settings.storage_limit_mb);
            labels.push(strings::size(u64::from(settings.storage_limit_mb) * 1_000_000));
        }
        if *self.limit_values.borrow() != values {
            let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            self.limit.set_model(Some(&gtk::StringList::new(&refs)));
            self.limit_values.replace(values.clone());
        }
        if let Some(index) = values.iter().position(|mb| *mb == settings.storage_limit_mb) {
            self.limit.set_selected(index as u32);
        }
        self.updating.set(false);
    }

    fn load_status(&self) {
        let Some(engine) = self.app.upgrade().and_then(|app| app.engine()) else { return };
        let status = self.status.clone();
        let usage = self.usage.clone();
        glib::spawn_future_local(async move {
            let read: Result<(MemoryStatus, StorageUsage), AutoPaperError> =
                blocking(move || Ok((engine.memory_status(), engine.storage_usage()?))).await;
            match read {
                Ok((memory, storage)) => {
                    if memory.reduced {
                        status.set_title("Reduced memory");
                        status.set_subtitle(
                            "The embedding model isn't installed, so AutoPaper only notices ideas that are nearly word for word the same.",
                        );
                        if let Some(problem) = memory.problem {
                            status.set_tooltip_text(Some(&problem));
                        }
                    } else {
                        status.set_title("Full memory");
                        status.set_subtitle(&format!("Notices similar ideas with {}.", memory.embedding_model));
                    }
                    let images = match storage.images_on_disk {
                        1 => "1 image".to_string(),
                        n => format!("{n} images"),
                    };
                    let remembered = match storage.generations {
                        1 => "1 idea remembered".to_string(),
                        n => format!("{n} ideas remembered"),
                    };
                    usage.set_subtitle(&format!("{images}, {} · {remembered}", strings::size(storage.image_bytes)));
                }
                Err(error) => tracing::warn!(%error, "couldn't read memory and storage"),
            }
        });
    }

    fn connect(self: &Rc<Self>, app: &Rc<App>, dialog: &adw::PreferencesDialog, clear: &adw::ButtonRow, reset: &gtk::Button) {
        let ctx = (Rc::downgrade(self), Rc::downgrade(app), dialog.downgrade());
        let quiet_ctx = ctx.clone();
        self.quiet.connect_selected_notify(move |row| {
            let (Some(this), Some(app), Some(dialog)) = (quiet_ctx.0.upgrade(), quiet_ctx.1.upgrade(), quiet_ctx.2.upgrade()) else { return };
            if this.updating.get() {
                return;
            }
            if let Some((quiet, _)) = strings::QUIET_PERIODS.get(row.selected() as usize) {
                let quiet = *quiet;
                save(&app, &dialog, move |settings| settings.quiet_period = quiet);
            }
        });
        let echo_ctx = ctx.clone();
        self.echoes.connect_selected_notify(move |row| {
            let (Some(this), Some(app), Some(dialog)) = (echo_ctx.0.upgrade(), echo_ctx.1.upgrade(), echo_ctx.2.upgrade()) else { return };
            if this.updating.get() {
                return;
            }
            if let Some((echoes, _)) = strings::ECHOES.get(row.selected() as usize) {
                let echoes = *echoes;
                save(&app, &dialog, move |settings| settings.echoes = echoes);
            }
        });
        let limit_ctx = ctx.clone();
        self.limit.connect_selected_notify(move |row| {
            let (Some(this), Some(app), Some(dialog)) = (limit_ctx.0.upgrade(), limit_ctx.1.upgrade(), limit_ctx.2.upgrade()) else { return };
            if this.updating.get() {
                return;
            }
            if let Some(mb) = this.limit_values.borrow().get(row.selected() as usize).copied() {
                let this = this.clone();
                let app_for_prune = app.clone();
                glib::spawn_future_local(async move {
                    match app_for_prune.change_settings(move |settings| settings.storage_limit_mb = mb).await {
                        Ok(()) => {
                            // A lower limit applies now, not after the next wallpaper.
                            if let Some(engine) = app_for_prune.engine()
                                && let Err(error) = blocking(move || engine.prune()).await {
                                    tracing::warn!(%error, "couldn't prune to the new limit");
                                }
                            this.load_status();
                        }
                        Err(error) => toast_error(&dialog, &app, &error),
                    }
                });
            }
        });
        let clear_ctx = ctx.clone();
        clear.connect_activated(move |row| {
            let (Some(this), Some(app), Some(dialog)) = (clear_ctx.0.upgrade(), clear_ctx.1.upgrade(), clear_ctx.2.upgrade()) else { return };
            let alert = adw::AlertDialog::new(
                Some("Clear history?"),
                Some(
                    "Every image AutoPaper made is deleted. With Keep memory, it still remembers each idea, so it won't \
                     repeat itself. Forget everything also resets what it learned from your ratings.",
                ),
            );
            alert.add_responses(&[("cancel", "_Cancel"), ("forget", "_Forget everything"), ("keep", "_Keep memory")]);
            alert.set_response_appearance("forget", adw::ResponseAppearance::Destructive);
            alert.set_response_appearance("keep", adw::ResponseAppearance::Suggested);
            alert.set_default_response(Some("keep"));
            alert.set_close_response("cancel");
            let row = row.clone();
            glib::spawn_future_local(async move {
                let response = alert.choose_future(Some(&row)).await;
                let keep_memory = match response.as_str() {
                    "keep" => true,
                    "forget" => false,
                    _ => return,
                };
                match app.clear_history(keep_memory).await {
                    Ok(()) => dialog.add_toast(crate::window::toast("History cleared")),
                    Err(error) => toast_error(&dialog, &app, &error),
                }
                this.load_status();
            });
        });
        let reset_ctx = ctx;
        reset.connect_clicked(move |row| {
            let (Some(app), Some(dialog)) = (reset_ctx.1.upgrade(), reset_ctx.2.upgrade()) else { return };
            let alert = adw::AlertDialog::new(
                Some("Reset what AutoPaper has learned?"),
                Some("It forgets which features you liked and disliked. Your ratings stay on each wallpaper."),
            );
            alert.add_responses(&[("cancel", "_Cancel"), ("reset", "_Reset")]);
            alert.set_response_appearance("reset", adw::ResponseAppearance::Destructive);
            alert.set_default_response(Some("cancel"));
            alert.set_close_response("cancel");
            let row = row.clone();
            glib::spawn_future_local(async move {
                if alert.choose_future(Some(&row)).await == "reset" {
                    match app.reset_taste().await {
                        Ok(()) => dialog.add_toast(crate::window::toast("AutoPaper has forgotten what it learned.")),
                        Err(error) => toast_error(&dialog, &app, &error),
                    }
                }
            });
        });
    }
}

// ── Budget ──────────────────────────────────────────────────────────────────────────────────────

/// Where "Custom" starts when there was no limit: the core's default budget ($5).
fn custom_start_cents() -> u32 {
    Settings::default().monthly_budget_cents.unwrap_or(500)
}

struct Budget {
    page: adw::PreferencesPage,
    app: Weak<App>,
    budget: adw::ComboRow,
    custom: adw::SpinRow,
    spent: adw::ActionRow,
    per_image: adw::ActionRow,
    monthly: adw::ActionRow,
    updating: Cell<bool>,
    /// "Custom" was chosen: it stays chosen, with its amount field, even while that amount equals a preset
    /// (Custom at $5 is still Custom). Choosing a preset ends it.
    custom_mode: Cell<bool>,
}

impl Budget {
    fn new(app: &Rc<App>, dialog: &adw::PreferencesDialog) -> Rc<Self> {
        let mut labels: Vec<&str> = strings::BUDGETS.iter().map(|(_, label)| *label).collect();
        labels.push("Custom");
        let budget = combo("Monthly budget", Some("A cap on estimated spend with hosted providers."), &labels);
        let custom = adw::SpinRow::builder()
            .title("Custom budget")
            .subtitle("US dollars a month")
            .adjustment(&gtk::Adjustment::new(5.0, 0.5, 1000.0, 0.5, 5.0, 0.0))
            .digits(2)
            .visible(false)
            .build();
        let limit = adw::PreferencesGroup::builder().title("Budget").build();
        limit.add(&budget);
        limit.add(&custom);

        let spent = adw::ActionRow::builder().title("Spent this month").subtitle_selectable(true).build();
        let per_image = adw::ActionRow::builder().title("Each wallpaper").subtitle_selectable(true).build();
        let monthly = adw::ActionRow::builder().title("A month at this pace").subtitle_selectable(true).build();
        let estimates = adw::PreferencesGroup::builder()
            .title("Estimates")
            .description(format!(
                "Prices as of {}. Local providers cost nothing. Your provider's bill is what counts.",
                strings::iso_date(&prices_as_of())
            ))
            .build();
        estimates.add(&spent);
        estimates.add(&per_image);
        estimates.add(&monthly);

        let page = adw::PreferencesPage::builder()
            .name("budget")
            .title("Budget")
            .icon_name("x-office-spreadsheet-symbolic")
            .build();
        page.add(&limit);
        page.add(&estimates);
        let this = Rc::new(Self {
            page,
            app: Rc::downgrade(app),
            budget,
            custom,
            spent,
            per_image,
            monthly,
            updating: Cell::new(false),
            custom_mode: Cell::new(false),
        });
        this.show();
        this.connect(app, dialog);
        follow(app, &this, |this, event| {
            if matches!(event, Event::Settings | Event::Status) {
                this.show();
            }
        });
        this
    }

    fn show(&self) {
        let Some(app) = self.app.upgrade() else { return };
        let state = app.state();
        let cents = state.settings.monthly_budget_cents;
        self.updating.set(true);
        let preset = strings::BUDGETS.iter().position(|(value, _)| *value == cents);
        match preset.filter(|_| !self.custom_mode.get()) {
            Some(index) => {
                self.budget.set_selected(index as u32);
                self.custom.set_visible(false);
            }
            None => {
                self.budget.set_selected(strings::BUDGETS.len() as u32);
                self.custom.set_visible(true);
                self.custom.set_value(f64::from(cents.unwrap_or_else(custom_start_cents)) / 100.0);
            }
        }
        self.updating.set(false);
        if let Some(spend) = &state.spend {
            let spent = strings::money(spend.spent_microusd);
            self.spent.set_subtitle(&match spend.budget_cents {
                Some(budget) => format!("{spent} of {} (estimated)", strings::cents(budget)),
                None => format!("{spent} (estimated)"),
            });
            self.per_image.set_subtitle(&if spend.per_image_microusd == 0 {
                "Free with these providers".to_string()
            } else {
                format!("About {}", strings::money(spend.per_image_microusd))
            });
            self.monthly.set_subtitle(&if spend.monthly_estimate_microusd == 0 {
                "Free".to_string()
            } else {
                format!(
                    "About {} {}",
                    strings::money(spend.monthly_estimate_microusd),
                    strings::cadence_pace(state.settings.cadence)
                )
            });
        }
    }

    fn connect(self: &Rc<Self>, app: &Rc<App>, dialog: &adw::PreferencesDialog) {
        let ctx = (Rc::downgrade(self), Rc::downgrade(app), dialog.downgrade());
        let combo_ctx = ctx.clone();
        self.budget.connect_selected_notify(move |row| {
            let (Some(this), Some(app), Some(dialog)) = (combo_ctx.0.upgrade(), combo_ctx.1.upgrade(), combo_ctx.2.upgrade()) else { return };
            if this.updating.get() {
                return;
            }
            match strings::BUDGETS.get(row.selected() as usize) {
                Some((cents, _)) => {
                    this.custom_mode.set(false);
                    this.custom.set_visible(false);
                    let cents = *cents;
                    save(&app, &dialog, move |settings| settings.monthly_budget_cents = cents);
                }
                None => {
                    // Custom: the amount field starts at the current budget ($5 from No limit, saved so the field
                    // and the budget agree) and saves its own changes; focus goes to it, to type the amount.
                    this.custom_mode.set(true);
                    let current = app.state().settings.monthly_budget_cents;
                    this.updating.set(true);
                    this.custom.set_value(f64::from(current.unwrap_or_else(custom_start_cents)) / 100.0);
                    this.updating.set(false);
                    this.custom.set_visible(true);
                    if current.is_none() {
                        save(&app, &dialog, |settings| settings.monthly_budget_cents = Some(custom_start_cents()));
                    }
                    let custom = this.custom.downgrade();
                    // After the drop-down's popover has closed and handed focus back to its row.
                    glib::idle_add_local_once(move || {
                        if let Some(custom) = custom.upgrade() {
                            custom.grab_focus();
                        }
                    });
                }
            }
        });
        let spin_ctx = ctx;
        self.custom.connect_value_notify(move |row| {
            let (Some(this), Some(app), Some(dialog)) = (spin_ctx.0.upgrade(), spin_ctx.1.upgrade(), spin_ctx.2.upgrade()) else { return };
            if this.updating.get() || !row.is_visible() {
                return;
            }
            let cents = (row.value() * 100.0).round() as u32;
            save(&app, &dialog, move |settings| settings.monthly_budget_cents = Some(cents));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_workflows_are_read_as_the_core_reads_them() {
        let api = r#"{"3": {"class_type": "KSampler", "inputs": {"seed": "{{seed}}", "width": {{width}}}},
            "4": {"class_type": "CheckpointLoaderSimple", "inputs": {"ckpt_name": "sd_xl_base_1.0.safetensors"}},
            "6": {"class_type": "CLIPTextEncode", "inputs": {"text": "{{prompt}}"}}}"#;
        assert_eq!(read_workflow(api), WorkflowReading::Model(Some("sd_xl_base_1.0.safetensors".into())));
        assert_eq!(own_model_line(Some(&read_workflow(api))), "sd_xl_base_1.0 (from your workflow)");
        // A whole /prompt body is accepted too, and the first loader by node number wins (ckpt_name before unet_name).
        let body = r#"{"prompt": {"12": {"class_type": "UNETLoader", "inputs": {"unet_name": "qwen.gguf"}},
            "2": {"class_type": "UNETLoader", "inputs": {"unet_name": "z_image.safetensors"}},
            "5": {"class_type": "CLIPTextEncode", "inputs": {"text": "{{prompt}}"}}}, "client_id": "x"}"#;
        assert_eq!(read_workflow(body), WorkflowReading::Model(Some("z_image.safetensors".into())));
        let no_loader = r#"{"1": {"class_type": "CLIPTextEncode", "inputs": {"text": "{{prompt}}"}}}"#;
        assert_eq!(read_workflow(no_loader), WorkflowReading::Model(None));
        assert_eq!(own_model_line(Some(&read_workflow(no_loader))), "Set in your workflow");
    }

    #[test]
    fn workflows_that_cant_work_are_refused_with_their_reason() {
        let no_prompt = r#"{"1": {"class_type": "CLIPTextEncode", "inputs": {"text": "a cat"}}}"#;
        assert_eq!(read_workflow(no_prompt), WorkflowReading::Problem(InvalidInputReason::WorkflowNeedsPrompt));
        let editor = r#"{"nodes": [{"id": 1, "type": "CLIPTextEncode", "widgets_values": ["{{prompt}}"]}], "links": []}"#;
        assert_eq!(read_workflow(editor), WorkflowReading::Problem(InvalidInputReason::WorkflowNotApiFormat));
        let broken = r#"{"1": {"class_type": "CLIPTextEncode", "inputs": {"text": "{{prompt}}"}"#;
        assert_eq!(read_workflow(broken), WorkflowReading::Problem(InvalidInputReason::WorkflowInvalid));
        let not_nodes = r#"{"text": "{{prompt}}"}"#;
        assert_eq!(read_workflow(not_nodes), WorkflowReading::Problem(InvalidInputReason::WorkflowNotApiFormat));
    }
}
