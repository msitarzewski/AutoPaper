#!/usr/bin/env bash
# Builds AutoPaper for Linux and installs it so the desktop knows it: the binary (autopaper-gtk), its .desktop
# file and D-Bus service (notifications' buttons and the Background portal find the app by its ID), icons,
# AppStream metainfo, the GSettings schema, and the embedding model when the repo has it (scripts/fetch-model.sh).
#
#   scripts/linux-install.sh [--prefix DIR] [--debug]     default prefix: ~/.local (no root needed)
#   scripts/linux-install.sh --uninstall [--prefix DIR]
#
# Needs Rust (rustup), GTK 4.22+ and libadwaita 1.9+ development files (Ubuntu 26.04: libgtk-4-dev
# libadwaita-1-dev; Fedora 44: gtk4-devel libadwaita-devel), and libglib2.0-bin for the schema and resources.
# Flatpak packaging is separate (phase 4).
set -euo pipefail

APP_ID="io.github.msitarzewski.AutoPaper"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DATA="$ROOT/apps/linux/data"
PREFIX="$HOME/.local"
PROFILE="release"
UNINSTALL=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --prefix) PREFIX="$2"; shift 2 ;;
    --debug) PROFILE="debug"; shift ;;
    --uninstall) UNINSTALL=1; shift ;;
    *) sed -n '2,10p' "$0"; exit 2 ;;
  esac
done
SHARE="$PREFIX/share"

if [[ "$UNINSTALL" == 1 ]]; then
  rm -f "$PREFIX/bin/autopaper-gtk" "$SHARE/applications/$APP_ID.desktop" "$SHARE/dbus-1/services/$APP_ID.service" \
    "$SHARE/icons/hicolor/scalable/apps/$APP_ID.svg" "$SHARE/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg" \
    "$SHARE/metainfo/$APP_ID.metainfo.xml" "$SHARE/glib-2.0/schemas/$APP_ID.gschema.xml"
  rm -rf "${SHARE:?}/$APP_ID"
  glib-compile-schemas "$SHARE/glib-2.0/schemas" 2>/dev/null || true
  echo "Removed AutoPaper from $PREFIX (your wallpapers and memory in ~/.local/share/autopaper stay)."
  exit 0
fi

cd "$ROOT"
if [[ "$PROFILE" == "release" ]]; then
  cargo build --release -p autopaper-linux
else
  cargo build -p autopaper-linux
fi
TARGET="${CARGO_TARGET_DIR:-$ROOT/target}/$PROFILE"

install -Dm755 "$TARGET/autopaper-gtk" "$PREFIX/bin/autopaper-gtk"

# A user install names the binary by its full path (~/.local/bin may not be on the session's PATH).
desktop_file="$(mktemp)"
service_file="$(mktemp)"
trap 'rm -f "$desktop_file" "$service_file"' EXIT
if [[ "$PREFIX" == "/usr" ]]; then
  cp "$DATA/$APP_ID.desktop" "$desktop_file"
else
  sed "s|^Exec=autopaper-gtk|Exec=$PREFIX/bin/autopaper-gtk|" "$DATA/$APP_ID.desktop" > "$desktop_file"
fi
sed "s|@bindir@|$PREFIX/bin|" "$DATA/$APP_ID.service.in" > "$service_file"
install -Dm644 "$desktop_file" "$SHARE/applications/$APP_ID.desktop"
install -Dm644 "$service_file" "$SHARE/dbus-1/services/$APP_ID.service"
install -Dm644 "$DATA/icons/hicolor/scalable/apps/$APP_ID.svg" "$SHARE/icons/hicolor/scalable/apps/$APP_ID.svg"
install -Dm644 "$DATA/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg" \
  "$SHARE/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg"
install -Dm644 "$DATA/$APP_ID.metainfo.xml" "$SHARE/metainfo/$APP_ID.metainfo.xml"
install -Dm644 "$DATA/$APP_ID.gschema.xml" "$SHARE/glib-2.0/schemas/$APP_ID.gschema.xml"
glib-compile-schemas "$SHARE/glib-2.0/schemas"

MODEL="$ROOT/models/bge-small-en-v1.5"
if [[ -f "$MODEL/model.safetensors" ]]; then
  mkdir -p "$SHARE/$APP_ID/models"
  rm -rf "$SHARE/$APP_ID/models/bge-small-en-v1.5"
  cp -R "$MODEL" "$SHARE/$APP_ID/models/"
else
  echo "Note: no embedding model in $MODEL (scripts/fetch-model.sh); memory will run in reduced mode."
fi

command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t -f "$SHARE/icons/hicolor" || true
command -v update-desktop-database >/dev/null && update-desktop-database -q "$SHARE/applications" || true
command -v desktop-file-validate >/dev/null && desktop-file-validate "$SHARE/applications/$APP_ID.desktop"
command -v appstreamcli >/dev/null && { appstreamcli validate --no-net --explain "$SHARE/metainfo/$APP_ID.metainfo.xml" || true; }
echo "Installed AutoPaper ($PROFILE) into $PREFIX."
