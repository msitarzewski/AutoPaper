//! Compiles the app's resources (style sheet, symbolic icons, the app icon) into a GResource bundle embedded in
//! the binary, and its GSettings schema into `$OUT_DIR/schemas` so a build can run straight from `target/`
//! (installs compile the schema into the system's schema directory instead; see scripts/linux-install.sh).
//! Needs `glib-compile-resources` and `glib-compile-schemas` (Ubuntu: libglib2.0-bin; Fedora: glib2-devel).

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const SCHEMA: &str = "data/io.github.msitarzewski.AutoPaper.gschema.xml";

fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    println!("cargo::rerun-if-changed=data");
    println!("cargo::rerun-if-env-changed=AUTOPAPER_BUILD");

    let bundle = out.join("autopaper.gresource");
    run(Command::new("glib-compile-resources")
        .arg("--sourcedir=data/resources")
        .arg(format!("--target={}", bundle.display()))
        .arg("data/resources/autopaper.gresource.xml"));

    let schemas = out.join("schemas");
    fs::create_dir_all(&schemas).expect("create the schema directory");
    fs::copy(SCHEMA, schemas.join("io.github.msitarzewski.AutoPaper.gschema.xml")).expect("copy the schema");
    run(Command::new("glib-compile-schemas").arg("--strict").arg(&schemas));

    // "Version (build)" in About: packagers pass AUTOPAPER_BUILD; a developer build says "dev".
    let build = env::var("AUTOPAPER_BUILD").unwrap_or_else(|_| "dev".into());
    println!("cargo::rustc-env=AUTOPAPER_BUILD={build}");
}

fn run(command: &mut Command) {
    let program = command.get_program().to_string_lossy().into_owned();
    match command.status() {
        Ok(status) if status.success() => {}
        Ok(status) => panic!("{program} failed ({status})"),
        Err(error) => panic!("{program} is needed to build AutoPaper for Linux (libglib2.0-bin): {error}"),
    }
}
