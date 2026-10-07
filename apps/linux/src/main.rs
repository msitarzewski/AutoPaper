//! AutoPaper for Linux: GTK 4 + libadwaita, following the GNOME HIG, on the shared Rust core.
//!
//! Binary `autopaper-gtk` (`autopaper` is the developer CLI). App ID `io.github.msitarzewski.AutoPaper`. What
//! the app does is docs/app-spec.md; how the core works is memory-bank/systemPatterns.md.

mod app;
mod desktop;
mod history;
mod keywords;
mod moods;
mod now;
mod preferences;
mod runtime;
mod secrets;
mod strings;
mod summary;
#[cfg(feature = "tray")]
mod tray;
mod wallpapers;
mod welcome;
mod window;

use gtk::{gio, glib};

fn main() -> glib::ExitCode {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,zbus=error"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    if let Err(error) = runtime::init() {
        eprintln!("AutoPaper couldn't start its engine runtime: {error}");
        return glib::ExitCode::FAILURE;
    }
    // Portal futures (ashpd on tokio) are polled on the main loop; the runtime's workers drive their I/O.
    let _runtime = runtime::runtime().enter();

    if let Err(error) = gio::resources_register_include!("autopaper.gresource") {
        eprintln!("AutoPaper's resources are missing: {error}");
        return glib::ExitCode::FAILURE;
    }
    glib::set_application_name("AutoPaper");

    match app::App::new() {
        Ok(app) => app.run(),
        Err(problem) => {
            eprintln!("AutoPaper couldn't start: {problem}");
            glib::ExitCode::FAILURE
        }
    }
}
