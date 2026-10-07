#!/usr/bin/env bash
# Drives the "Scratch" Parallels VM (Ubuntu 26.04, GNOME 50, Wayland) for building and running the Linux app.
# Run on the Mac. The repo isn't on a shared folder, so files go over `prlctl exec` stdin as a tar stream.
#
#   scripts/linux-vm.sh sync [--full]     copy changed sources to ~/dev/autopaper in the VM (--full: everything)
#   scripts/linux-vm.sh build [args…]     cargo build -p autopaper-linux (extra args passed to cargo)
#   scripts/linux-vm.sh install [--debug] build (release) and install for michael under ~/.local (linux-install.sh)
#   scripts/linux-vm.sh run [args…]       start the installed app on the desktop (log: ~/autopaper-gtk.log)
#   scripts/linux-vm.sh stop              quit the running app
#   scripts/linux-vm.sh pull              copy apps/linux/src back from the VM (after clippy --fix there)
#   scripts/linux-vm.sh sh '<command>'    run a command as michael with the desktop session's environment
#   scripts/linux-vm.sh script <file>     run a local bash script there as michael (same environment)
#   scripts/linux-vm.sh root '<command>'  run a command as root
#   scripts/linux-vm.sh shot <name>       capture the VM screen to .scratch/shots/linux/<name>.png
#
# Why the odd plumbing (memory-bank/techContext.md): `prlctl exec` runs as root and re-joins its arguments into
# one shell string, so commands travel base64-encoded; GUI apps need the session's Wayland and D-Bus variables.
# The VM's clock may lag the Mac's, so tar extracts with fresh modification times (-m) and only changed files
# are sent: cargo then rebuilds only what changed.
set -euo pipefail

VM="${AUTOPAPER_VM:-Scratch}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
STAMP="$ROOT/.scratch/linux/last-sync"
REMOTE="/home/michael/dev/autopaper"
SESSION_ENV="XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-0 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus XDG_CURRENT_DESKTOP=ubuntu:GNOME XDG_SESSION_TYPE=wayland"
# Sources the Linux app needs (the macOS and Windows apps, the site and research stay on the Mac).
PATHS=(Cargo.toml Cargo.lock core cli uniffi-bindgen apps/linux scripts/linux-install.sh)

# Runs a bash script (from stdin of this function's argument) in the VM as root.
as_root() {
  local encoded
  encoded="$(printf '%s' "$1" | base64 | tr -d '\n')"
  prlctl exec "$VM" "echo $encoded | base64 -d | bash"
}

# Runs a command as michael in a login shell (rustup's cargo on PATH) with the desktop session's environment.
as_user() {
  local inner encoded
  inner="export $SESSION_ENV; cd $REMOTE 2>/dev/null || true; $1"
  encoded="$(printf '%s' "$inner" | base64 | tr -d '\n')"
  as_root "sudo -u michael -i bash -lc \"\$(echo $encoded | base64 -d)\""
}

sync_sources() {
  local full="${1:-}"
  mkdir -p "$(dirname "$STAMP")"
  local list existing=()
  list="$(mktemp)"
  for path in "${PATHS[@]}"; do
    [[ -e "$ROOT/$path" ]] && existing+=("$path")
  done
  (
    cd "$ROOT"
    if [[ "$full" == "--full" || ! -f "$STAMP" ]]; then
      find "${existing[@]}" -type f -not -path '*/target/*' -not -name '.DS_Store' -print
    else
      find "${existing[@]}" -type f -newer "$STAMP" -not -path '*/target/*' -not -name '.DS_Store' -print
    fi
  ) > "$list"
  local count
  count="$(wc -l < "$list" | tr -d ' ')"
  local next_stamp
  next_stamp="$(mktemp)"
  if [[ "$count" -gt 0 ]]; then
    (cd "$ROOT" && COPYFILE_DISABLE=1 tar --no-xattrs --no-mac-metadata -cf - -T "$list") |
      prlctl exec "$VM" "sudo -u michael bash -c 'mkdir -p $REMOTE && tar -xmf - -C $REMOTE'"
  fi
  # The embedding model (133 MB) goes once; the app finds it through AUTOPAPER_MODEL_DIR or the install.
  if [[ -d "$ROOT/models/bge-small-en-v1.5" ]] &&
    ! prlctl exec "$VM" "test -f $REMOTE/models/bge-small-en-v1.5/model.safetensors" >/dev/null 2>&1; then
    echo "Copying the embedding model…"
    (cd "$ROOT" && COPYFILE_DISABLE=1 tar --no-xattrs --no-mac-metadata -cf - models/bge-small-en-v1.5) |
      prlctl exec "$VM" "sudo -u michael bash -c 'mkdir -p $REMOTE && tar -xmf - -C $REMOTE'"
  fi
  mv "$next_stamp" "$STAMP"
  rm -f "$list"
  echo "Synced $count file(s) to $VM:$REMOTE"
}

command="${1:-}"
shift || true
case "$command" in
  sync) sync_sources "${1:-}" ;;
  build)
    sync_sources
    as_user "cargo build -p autopaper-linux $* 2>&1 | grep -v '^\s*Compiling\|^\s*Downloaded\|^\s*Downloading' | tail -n 200"
    ;;
  install)
    sync_sources
    as_user "bash scripts/linux-install.sh --prefix \$HOME/.local $* 2>&1 | tail -n 60"
    ;;
  run)
    as_user "pkill -x autopaper-gtk >/dev/null 2>&1; sleep 0.5; setsid \$HOME/.local/bin/autopaper-gtk $* > \$HOME/autopaper-gtk.log 2>&1 < /dev/null & sleep 2; tail -n 20 \$HOME/autopaper-gtk.log"
    ;;
  stop) as_user "pkill -x autopaper-gtk || true" ;;
  pull)
    # Brings the VM's apps/linux/src back (after `cargo clippy --fix` there). The Mac's copy is overwritten.
    prlctl exec "$VM" "tar -C $REMOTE -cf - apps/linux/src" | tar -xf - -C "$ROOT"
    touch "$STAMP"
    echo "Pulled apps/linux/src from $VM"
    ;;
  sh) as_user "$1" ;;
  script)
    # Copies a local bash script to the VM and runs it as michael (variables and quoting survive intact).
    prlctl exec "$VM" "sudo -u michael tee /home/michael/.autopaper-script.sh >/dev/null" < "${1:?script}"
    as_user "bash /home/michael/.autopaper-script.sh"
    ;;
  root) as_root "$1" ;;
  shot)
    mkdir -p "$ROOT/.scratch/shots/linux"
    prlctl capture "$VM" --file "$ROOT/.scratch/shots/linux/${1:?name}.png"
    echo "$ROOT/.scratch/shots/linux/$1.png"
    ;;
  *)
    sed -n '2,15p' "$0"
    exit 2
    ;;
esac
