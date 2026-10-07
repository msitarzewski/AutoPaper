//! The Linux desktop around the app: where data lives, which desktop this is, the displays, and the portals that
//! set the wallpaper and let AutoPaper run in the background.
//!
//! Portal facts (docs/research/rust-linux.md): the Wallpaper portal takes a file descriptor (`SetWallpaperFile`;
//! it rejects `file://` URIs) and is always asked with `show_preview(false)` (KDE previews by default). GNOME's
//! backend ignores `set-on` and sets the light and dark backgrounds; GNOME's lock screen shows the desktop
//! background. The portal sets one picture for every monitor, so AutoPaper renders for the largest display.
//! Unsandboxed, the app registers its app ID with the portal first (`register_host_app`), or permissions and
//! autostart have no app to belong to.

use std::os::fd::AsFd;
use std::path::{Path, PathBuf};

use ashpd::WindowIdentifier;
use ashpd::desktop::background::Background;
use ashpd::desktop::wallpaper::{SetOn, WallpaperRequest};
use autopaper_core::DisplayTarget;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};

pub const APP_ID: &str = "io.github.msitarzewski.AutoPaper";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const BUILD: &str = env!("AUTOPAPER_BUILD");
/// The installed binary's name (the developer CLI is `autopaper`).
pub const BINARY: &str = "autopaper-gtk";
const MODEL_NAME: &str = "bge-small-en-v1.5";

/// GNOME (Ubuntu's session says "ubuntu:GNOME"). GNOME has no tray: AutoPaper runs as a background app and talks
/// through notifications; other desktops (KDE Plasma and the rest) get a StatusNotifierItem tray.
pub fn is_gnome() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .any(|desktop| desktop.eq_ignore_ascii_case("GNOME"))
}

/// Whether to show the tray: everywhere but GNOME. `AUTOPAPER_TRAY=1` / `=0` overrides it (to test the tray under
/// GNOME with an AppIndicator extension, or to hide it).
pub fn wants_tray() -> bool {
    match std::env::var("AUTOPAPER_TRAY").as_deref() {
        Ok("1") => true,
        Ok("0") => false,
        _ => !is_gnome(),
    }
}

pub fn is_sandboxed() -> bool {
    ashpd::is_sandboxed()
}

/// `$XDG_DATA_HOME/autopaper` (inside Flatpak, the app's own data directory).
pub fn data_dir() -> PathBuf {
    glib::user_data_dir().join("autopaper")
}

/// The embedding model: `AUTOPAPER_MODEL_DIR`, else installed next to the binary
/// (`<prefix>/share/io.github.msitarzewski.AutoPaper/models/…`, which is `/app/share/…` in Flatpak), else the
/// repository's `models/` for a developer build. When none has the files, the engine runs memory in reduced mode
/// and Preferences → Memory says so.
pub fn model_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("AUTOPAPER_MODEL_DIR") {
        return PathBuf::from(dir);
    }
    let installed = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().and_then(Path::parent).map(Path::to_path_buf))
        .map(|prefix| prefix.join("share").join(APP_ID).join("models").join(MODEL_NAME));
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models").join(MODEL_NAME);
    [installed, Some(repository)]
        .into_iter()
        .flatten()
        .find(|dir| dir.join("model.safetensors").is_file())
        .unwrap_or_else(|| PathBuf::from("/app/share").join(APP_ID).join("models").join(MODEL_NAME))
}

/// The person's language as BCP 47 ("en-US"), from the first locale GLib reports.
pub fn locale() -> String {
    glib::language_names()
        .iter()
        .map(|name| name.as_str())
        .find(|name| *name != "C" && *name != "POSIX")
        .map(|name| {
            let base = name.split(['.', '@']).next().unwrap_or(name);
            base.replace('_', "-")
        })
        .unwrap_or_else(|| "en-US".into())
}

/// "Ubuntu/26.04 AutoPaper/0.1.0" for the User-Agent (from os-release; the host's inside Flatpak).
pub fn client() -> String {
    let text = ["/run/host/os-release", "/etc/os-release", "/usr/lib/os-release"]
        .iter()
        .find_map(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_default();
    let field = |key: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(key).and_then(|rest| rest.strip_prefix('=')))
            .map(|value| value.trim_matches('"').to_string())
    };
    let os = field("NAME").unwrap_or_else(|| "Linux".into()).replace(['/', ' '], "");
    let version = field("VERSION_ID").unwrap_or_default();
    if version.is_empty() { format!("{os} AutoPaper/{VERSION}") } else { format!("{os}/{version} AutoPaper/{VERSION}") }
}

/// Every monitor, in physical pixels (logical size × scale). The connector name ("DP-1") identifies it.
pub fn displays() -> Vec<DisplayTarget> {
    let Some(display) = gdk::Display::default() else {
        return Vec::new();
    };
    let monitors = display.monitors();
    (0..monitors.n_items())
        .filter_map(|index| monitors.item(index).and_downcast::<gdk::Monitor>())
        .enumerate()
        .map(|(index, monitor)| {
            let geometry = monitor.geometry();
            let scale = monitor.scale().max(1.0);
            let id = monitor.connector().map(|c| c.to_string()).unwrap_or_else(|| format!("monitor-{index}"));
            DisplayTarget {
                id,
                width: (f64::from(geometry.width()) * scale).round().max(1.0) as u32,
                height: (f64::from(geometry.height()) * scale).round().max(1.0) as u32,
            }
        })
        .collect()
}

/// The display with the most pixels: the portal sets one picture for every monitor, so this is the one rendered.
pub fn largest_display() -> Option<DisplayTarget> {
    displays().into_iter().max_by_key(|display| u64::from(display.width) * u64::from(display.height))
}

/// Registers the app ID with the portals when running unsandboxed (needs the installed .desktop file).
pub async fn register_with_portals() {
    if is_sandboxed() {
        return;
    }
    let result = match ashpd::AppID::try_from(APP_ID) {
        Ok(app_id) => ashpd::register_host_app(app_id).await.map_err(|error| error.to_string()),
        Err(error) => Err(error.to_string()),
    };
    if let Err(error) = result {
        tracing::warn!(%error, "couldn't register with the portals (is the .desktop file installed?)");
    }
}

/// Why setting the wallpaper didn't work, in words for the status line.
#[derive(Debug)]
pub enum WallpaperProblem {
    /// The person said no to "Allow AutoPaper to set backgrounds?" (or dismissed it).
    Declined,
    /// No Wallpaper portal (or it failed); the detail is for the log.
    Unavailable(String),
}

/// Sets the picture at `path` as the wallpaper (and, where the desktop separates them, the lock screen) through
/// the Wallpaper portal. Polled on the GTK main loop: the window identifier belongs to GTK.
pub async fn set_wallpaper(path: &Path, lock_screen: bool, parent: Option<gtk::Window>) -> Result<(), WallpaperProblem> {
    let file = std::fs::File::open(path).map_err(|error| WallpaperProblem::Unavailable(error.to_string()))?;
    let identifier = match parent.filter(|window| window.is_visible()) {
        Some(window) => WindowIdentifier::from_native(&window).await,
        None => None,
    };
    // GNOME ignores set-on (its lock screen shows the desktop background); KDE honours it.
    let set_on = if lock_screen { SetOn::Both } else { SetOn::Background };
    let request = WallpaperRequest::default()
        .identifier(identifier)
        .set_on(set_on)
        .show_preview(false)
        .build_file(&file.as_fd())
        .await
        .map_err(|error| WallpaperProblem::Unavailable(error.to_string()))?;
    match request.response() {
        Ok(()) => Ok(()),
        Err(ashpd::Error::Response(_)) => Err(WallpaperProblem::Declined),
        Err(error) => Err(WallpaperProblem::Unavailable(error.to_string())),
    }
}

/// What the Background portal allowed.
#[derive(Debug, Clone, Copy)]
pub struct BackgroundGrant {
    pub run_in_background: bool,
    pub auto_start: bool,
}

/// Asks to keep running with no window open, and to start at login when `auto_start` (or to stop starting at
/// login). Unsandboxed apps are granted it without a dialog; Flatpak asks the person once.
pub async fn request_background(auto_start: bool, parent: Option<gtk::Window>) -> Result<BackgroundGrant, String> {
    let identifier = match parent.filter(|window| window.is_visible()) {
        Some(window) => WindowIdentifier::from_native(&window).await,
        None => None,
    };
    // Outside Flatpak the autostart entry runs this exact binary; inside, Flatpak runs it in the sandbox.
    let program = if is_sandboxed() {
        BINARY.to_string()
    } else {
        std::env::current_exe().map(|exe| exe.to_string_lossy().into_owned()).unwrap_or_else(|_| BINARY.into())
    };
    let request = Background::request()
        .identifier(identifier)
        .reason("AutoPaper makes new wallpapers on your schedule, with no window open.")
        .auto_start(auto_start)
        .command([program.as_str(), "--background"])
        .dbus_activatable(false)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let response = request.response().map_err(|error| error.to_string())?;
    Ok(BackgroundGrant { run_in_background: response.run_in_background(), auto_start: response.auto_start() })
}

/// Opens a web page or another app's link (through the OpenURI portal inside Flatpak).
pub fn open_uri(uri: &str, parent: Option<&gtk::Window>) {
    let uri = uri.to_string();
    gtk::UriLauncher::new(&uri).launch(parent, gio::Cancellable::NONE, move |result| {
        if let Err(error) = result {
            tracing::warn!(%error, uri, "couldn't open a link");
        }
    });
}

/// Whether some installed app handles `scheme://` links (e.g. Brew Browser's `brewbrowser://`).
pub fn handles_scheme(scheme: &str) -> bool {
    gio::AppInfo::default_for_uri_scheme(scheme).is_some()
}

/// Opens the folder holding `path` in the file manager, with the file selected where it can.
pub fn show_in_files(path: &Path, parent: Option<&gtk::Window>) {
    let file = gio::File::for_path(path);
    gtk::FileLauncher::new(Some(&file)).open_containing_folder(parent, gio::Cancellable::NONE, |result| {
        if let Err(error) = result {
            tracing::warn!(%error, "couldn't show a file in Files");
        }
    });
}

// ── The person's own wallpaper (docs/app-spec.md 3a) ───────────────────────────────────────────────────────────

/// GNOME's background settings: the person's own wallpaper is recorded from them before AutoPaper first sets one, and
/// put back when AutoPaper quits or the person chooses Restore my wallpaper (pausing keeps AutoPaper's showing). Only
/// on GNOME, outside Flatpak: the Wallpaper portal can set a picture but not say which one is showing, a sandbox can't
/// read the host's settings, and other desktops keep their wallpaper elsewhere (GNOME's keys may be installed there but
/// do nothing).
const BACKGROUND_SCHEMA: &str = "org.gnome.desktop.background";
const BACKGROUND_KEYS: [&str; 6] =
    ["picture-uri", "picture-uri-dark", "picture-options", "primary-color", "secondary-color", "color-shading-type"];
/// The keys naming the picture (light and dark style): GNOME's Wallpaper portal points both at its copy.
const PICTURE_KEYS: [&str; 2] = ["picture-uri", "picture-uri-dark"];

/// GNOME's background settings, where they say what the desktop shows. The app keeps one to hear changes
/// (`changed::picture-uri`); `None` where the person's own wallpaper can't be recorded and put back.
pub fn gnome_background() -> Option<gio::Settings> {
    if is_sandboxed() || !is_gnome() {
        return None;
    }
    let schema = gio::SettingsSchemaSource::default()?.lookup(BACKGROUND_SCHEMA, true)?;
    BACKGROUND_KEYS.iter().all(|key| schema.has_key(key)).then(|| gio::Settings::new(BACKGROUND_SCHEMA))
}

/// Where GNOME's Wallpaper portal keeps the picture it was last given (any app's, AutoPaper's too).
pub fn portal_copy() -> PathBuf {
    glib::user_config_dir().join("background")
}

/// The Wallpaper portal's copy as GNOME's picture keys name it once a picture has been set through the portal.
pub fn portal_uri() -> String {
    gio::File::for_path(portal_copy()).uri().to_string()
}

/// The picture keys (light and dark style) as they are now, by name; `None` where they can't be read.
pub fn picture_settings() -> Option<Vec<(String, String)>> {
    let settings = gnome_background()?;
    Some(PICTURE_KEYS.iter().map(|key| (key.to_string(), settings.string(key).to_string())).collect())
}

/// The portal's copy as it is now (modification time, size, inode), noted when AutoPaper sets a wallpaper: a copy
/// that has changed since is another app's picture, set through the portal under the same name.
pub fn portal_copy_identity() -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(portal_copy()).ok()?;
    Some(format!("{}.{:09}:{}:{}", meta.mtime(), meta.mtime_nsec(), meta.size(), meta.ino()))
}

/// Whether the person's own wallpaper can be recorded and put back on this desktop.
pub fn can_restore_wallpaper() -> bool {
    gnome_background().is_some()
}

/// GNOME's background settings as (key, value) pairs.
pub type BackgroundSettings = Vec<(String, String)>;

/// The person's wallpaper settings now, and the picture that needs keeping first: when it's the Wallpaper portal's
/// own copy (`~/.config/background`), every portal app overwrites that file (AutoPaper too), so the caller copies it
/// to `kept` (`keep_own_picture`), which the settings name instead.
pub fn own_wallpaper(kept: &Path) -> Option<(BackgroundSettings, Option<PathBuf>)> {
    let settings = gnome_background()?;
    let portal_copy = portal_copy();
    let portal_uri = gio::File::for_path(&portal_copy).uri();
    let kept_uri = gio::File::for_path(kept).uri().to_string();
    let mut needs_copy = None;
    let saved = BACKGROUND_KEYS
        .iter()
        .map(|key| {
            let mut value = settings.string(key).to_string();
            if key.starts_with("picture-uri") && value == portal_uri.as_str() {
                needs_copy = Some(portal_copy.clone());
                value = kept_uri.clone();
            }
            (key.to_string(), value)
        })
        .collect();
    Some((saved, needs_copy))
}

/// Copies the portal's picture to `kept` (off the main thread: it's file I/O).
pub fn keep_own_picture(from: &Path, kept: &Path) -> std::io::Result<()> {
    if let Some(dir) = kept.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::copy(from, kept).map(|_| ())
}

/// Puts the recorded settings back (only GNOME's background keys); false when this desktop can't.
pub fn put_back_own_wallpaper(saved: &[(String, String)]) -> bool {
    let Some(settings) = gnome_background() else { return false };
    for (key, value) in saved.iter().filter(|(key, _)| BACKGROUND_KEYS.contains(&key.as_str())) {
        if let Err(error) = settings.set_string(key, value) {
            tracing::warn!(%error, key, "couldn't put a background setting back");
        }
    }
    gio::Settings::sync();
    true
}
