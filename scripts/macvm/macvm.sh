#!/usr/bin/env bash
# Drives a headless macOS 26 "Tahoe" VM (tart) for GUI-testing the macOS app without touching this Mac's desktop:
# no window, no sound, no clipboard sharing on the host; screenshots, wallpaper changes and accessibility audits
# happen inside the VM. Notes and caveats: memory-bank/techContext.md, "macOS test VM".
#
#   scripts/macvm/macvm.sh setup                  pull the image, create autopaper-mac26 + autopaper-mac26-clean
#   scripts/macvm/macvm.sh start                  boot headless, share the repo read-only, wait for SSH
#   scripts/macvm/macvm.sh stop                   shut the VM down (it stays ready to start again)
#   scripts/macvm/macvm.sh reset                  recreate autopaper-mac26-run from autopaper-mac26-clean
#   scripts/macvm/macvm.sh status                 VMs, sizes, the running VM's IP
#   scripts/macvm/macvm.sh ip                     the VM's IP address
#   scripts/macvm/macvm.sh ssh ['<command>']      SSH as admin (interactive without a command)
#   scripts/macvm/macvm.sh gui '<command>'        run a bash command as admin in the logged-in GUI (Aqua) session
#   scripts/macvm/macvm.sh tool <args…>           vmtool in the GUI session: windows [owner] | wallpaper get |
#                                                 wallpaper set <file-in-vm> | ax <bundle-id|name|pid> [depth] |
#                                                 press <bundle-id|name|pid> <button title>
#   scripts/macvm/macvm.sh shot <name>            whole VM screen → .scratch/shots/macvm/<name>.png
#   scripts/macvm/macvm.sh winshot <owner> <name> one window of <owner> (e.g. AutoPaper) → …/<name>.png
#   scripts/macvm/macvm.sh install-app [App.app]  copy the app (default: the Debug build) to /Applications in the VM
#   scripts/macvm/macvm.sh launch-app             open it in the GUI session; report signature/launch rejections
#   scripts/macvm/macvm.sh quit-app               quit it
#
# AUTOPAPER_MACVM picks the VM (default autopaper-mac26); for a throwaway run:
#   scripts/macvm/macvm.sh reset && AUTOPAPER_MACVM=autopaper-mac26-run scripts/macvm/macvm.sh start
set -euo pipefail

IMAGE="ghcr.io/cirruslabs/macos-tahoe-base:latest"
BASE_VM="autopaper-mac26"
CLEAN_VM="autopaper-mac26-clean"
RUN_VM="autopaper-mac26-run"
VM="${AUTOPAPER_MACVM:-$BASE_VM}"
CPUS=6
MEMORY_MB=16384
# "px": 2560×1600 pixels at 1x. Without a unit tart means points and sizes the VM for the host's Retina scale
# (5120×3200 px screenshots).
DISPLAY_SIZE="2560x1600px"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SCRATCH="$ROOT/.scratch/macvm"
SHOTS="$ROOT/.scratch/shots/macvm"
KEY="$SCRATCH/id_ed25519"
# Only scripts/macvm is shared (read-only): never the whole repo, which holds .env with API keys.
SHARE="/Volumes/My Shared Files/macvm"
APP_DEFAULT="$ROOT/build/macos-DerivedData/Build/Products/Debug/AutoPaper.app"
VMTOOL="/Users/admin/macvm/vmtool"
SSH_OPTS=(-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=5
  -o ServerAliveInterval=15)
mkdir -p "$SCRATCH" "$SHOTS"

die() { echo "macvm: $*" >&2; exit 1; }
exists() { tart list --quiet 2>/dev/null | grep -qx "$1"; }
running() { tart list --format json 2>/dev/null | python3 -c 'import json,sys; n=sys.argv[1]
sys.exit(0 if any(v["Name"]==n and v.get("Running") for v in json.load(sys.stdin)) else 1)' "$1"; }
vm_ip() { tart ip "$VM" --wait "${1:-0}"; }

# SSH with the project key. Cirrus images ship with admin/admin; the first connection installs the key using
# that password (through SSH_ASKPASS, so nothing is typed). SSH_EXTRA adds options (e.g. -t).
ssh_vm() {
  local ip
  # tart ip still answers (the last DHCP lease) after the VM stops, so ask tart whether it runs.
  running "$VM" || die "$VM isn't running (scripts/macvm/macvm.sh start)"
  ip="$(vm_ip)" || die "no IP for $VM"
  ssh "${SSH_OPTS[@]}" ${SSH_EXTRA:-} -i "$KEY" -o IdentitiesOnly=yes -o BatchMode=yes "admin@$ip" "$@"
}
install_key() {
  [[ -f "$KEY" ]] || ssh-keygen -q -t ed25519 -N '' -C "autopaper-macvm" -f "$KEY"
  if ssh_vm true 2>/dev/null; then return; fi
  local ip askpass="$SCRATCH/askpass.sh"
  ip="$(vm_ip)"
  printf '#!/bin/sh\necho "${MACVM_PASSWORD:-admin}"\n' > "$askpass" && chmod 700 "$askpass"
  SSH_ASKPASS="$askpass" SSH_ASKPASS_REQUIRE=force \
    ssh "${SSH_OPTS[@]}" -o PreferredAuthentications=password -o PubkeyAuthentication=no "admin@$ip" \
    "mkdir -p ~/.ssh && chmod 700 ~/.ssh && cat >> ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys" \
    < "$KEY.pub"
  ssh_vm true || die "key login still fails"
}

# Runs a bash script as admin inside the GUI session: `launchctl asuser` (root) joins the user's Aqua bootstrap,
# `sudo -u admin` drops back to the user. The script travels base64-encoded, so quoting survives SSH.
gui() {
  local encoded
  encoded="$(printf 'export PATH=/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin\n%s\n' "$1" |
    base64 | tr -d '\n')"
  ssh_vm "sudo launchctl asuser \$(id -u admin) sudo -u admin /bin/bash -c \"\$(echo $encoded | base64 -d)\""
}

# The guest's virtiofs keeps stale entries for files the Mac replaced by rename (editors, Xcode rebuilds): the old
# size and date, then "No such file" on open. Remounting the share drops them. Run before reading from the share.
# Never forced: if something in the VM has the share open, it's left as it is (with a warning).
refresh_share() {
  ssh_vm "M='/Volumes/My Shared Files'; if mount | grep -q \" on \$M \"; then sudo umount \"\$M\" 2>/dev/null ||
      echo 'macvm: share in use, not refreshed (files the Mac replaced may look stale)' >&2; fi
    for _ in 1 2 3 4 5 6 7 8 9 10; do mount | grep -q \" on \$M \" && break; sleep 0.3; done
    mount | grep -q \" on \$M \" || { sudo mkdir -p \"\$M\" && sudo mount_virtiofs com.apple.virtio-fs.automount \"\$M\"; }
    test -d '$SHARE' || { echo 'macvm share not mounted' >&2; exit 1; }"
}

# Compiles scripts/macvm/vmtool.swift inside the VM (from the read-only share) when it's missing or older.
# "quick": only make sure a vmtool exists (no share refresh), for screenshots.
ensure_vmtool() {
  if [[ "${1:-}" == quick ]] && ssh_vm "test -x $VMTOOL"; then return; fi
  refresh_share
  ssh_vm "src='$SHARE/vmtool.swift'; test -f \"\$src\" || { echo 'macvm share not mounted' >&2; exit 1; }
    if [ ! -x $VMTOOL ] || [ \"\$src\" -nt $VMTOOL ]; then mkdir -p \$(dirname $VMTOOL) &&
      xcrun swiftc -swift-version 5 -O \"\$src\" -o $VMTOOL; fi"
}

# macOS 26 shows "com.apple.sshd-session is requesting to bypass the system private window picker…" in the VM after
# the first capture from SSH, and again every 30 days (replayd's ScreenCaptureApprovals.plist). The capture itself
# works. Setup pushes the next notice to 2100; replayd only rereads the file when restarted (it ignores SIGTERM).
quiet_capture_notice() {
  ssh_vm 'P="$HOME/Library/Group Containers/group.com.apple.replayd/ScreenCaptureApprovals.plist"
    sudo kill -9 $(pgrep -x replayd) 2>/dev/null; mkdir -p "$(dirname "$P")"
    /usr/bin/python3 -c "
import plistlib, datetime, os, sys
p = sys.argv[1]
d = plistlib.load(open(p, \"rb\")) if os.path.exists(p) else {}
e = d.setdefault(\"/usr/libexec/sshd-keygen-wrapper\", {})
e[\"kScreenCaptureApprovalLastAlerted\"] = datetime.datetime(2099, 12, 1, tzinfo=datetime.timezone.utc)
e[\"kScreenCapturePrivacyHintDate\"] = datetime.datetime(2100, 1, 1, tzinfo=datetime.timezone.utc)
e.setdefault(\"kScreenCapturePrivacyHintPolicy\", 2592000)
plistlib.dump(d, open(p, \"wb\"))" "$P"'
}

# If that notice is up anyway, press its Allow button (through Accessibility). Succeeds only when it dismissed one.
dismiss_capture_notice() {
  gui "$VMTOOL ax UserNotificationCenter 3 2>/dev/null | grep -q 'private window picker' &&
    $VMTOOL press UserNotificationCenter Allow >/dev/null"
}

wait_for_ssh() {
  local ip
  ip="$(vm_ip 180)" || die "no IP for $VM after 3 minutes (log: $SCRATCH/$VM.log)"
  for _ in $(seq 1 60); do
    if ssh "${SSH_OPTS[@]}" -o BatchMode=yes -o PreferredAuthentications=none "admin@$ip" true 2>&1 |
      grep -q -i -E 'permission denied|authenticat'; then break; fi
    sleep 2
  done
  install_key
  # The auto-login session is ready once Finder and the Dock are up.
  for _ in $(seq 1 60); do
    ssh_vm "pgrep -qx Dock && pgrep -qx Finder" && break
    sleep 2
  done
}

start() {
  exists "$VM" || die "no VM named $VM (scripts/macvm/macvm.sh setup)"
  if running "$VM"; then echo "$VM is already running at $(vm_ip)"; return; fi
  # --no-graphics: no window on the Mac. --no-audio / --no-clipboard: the VM can't play sound on the Mac or read and
  # write its clipboard. Only scripts/macvm is shared, read-only, at "$SHARE" (never the repo: .env holds API keys).
  nohup tart run "$VM" --no-graphics --no-audio --no-clipboard --dir="macvm:$ROOT/scripts/macvm:ro" \
    > "$SCRATCH/$VM.log" 2>&1 < /dev/null &
  disown
  wait_for_ssh
  echo "$VM running at $(vm_ip) (ssh: scripts/macvm/macvm.sh ssh)"
}

stop() {
  running "$VM" || { echo "$VM isn't running"; return; }
  # Shut macOS down cleanly over SSH (tart stop's power-button request can leave a dialog waiting in the guest);
  # tart stop is the fallback after 90 s.
  ssh_vm "sudo shutdown -h now" >/dev/null 2>&1 || true
  for _ in $(seq 1 45); do running "$VM" || break; sleep 2; done
  if running "$VM"; then tart stop "$VM" --timeout 30 >/dev/null 2>&1 || true; fi
  echo "$VM stopped"
}

setup() {
  if ! tart list --source oci --quiet 2>/dev/null | grep -qx "$IMAGE"; then
    tart pull "$IMAGE" --concurrency 3
  fi
  if ! exists "$BASE_VM"; then
    tart clone "$IMAGE" "$BASE_VM"
  fi
  tart set "$BASE_VM" --cpu "$CPUS" --memory "$MEMORY_MB" --display "$DISPLAY_SIZE"
  VM="$BASE_VM" start
  # Inside the VM only: no background update downloads or installs (several GB each; macOS 26 keeps the small
  # catalog check on whatever is written), no Spotlight indexing.
  VM="$BASE_VM" ssh_vm 'for key in AutomaticDownload AutomaticallyInstallMacOSUpdates CriticalUpdateInstall \
      ConfigDataInstall; do sudo defaults write /Library/Preferences/com.apple.SoftwareUpdate $key -bool false; done
    sudo defaults write /Library/Preferences/com.apple.commerce AutoUpdate -bool false
    sudo mdutil -a -i off >/dev/null 2>&1 || true'
  # The image was saved with a Terminal window open, and login reopens it: turn off window restoration and close it,
  # so screenshots show only the desktop and the app under test.
  VM="$BASE_VM" gui 'defaults write com.apple.loginwindow TALLogoutSavesState -bool false
    defaults write com.apple.loginwindow LoginwindowLaunchesRelaunchApps -bool false
    defaults write NSGlobalDomain NSQuitAlwaysKeepsWindows -bool false
    for f in ~/Library/Preferences/ByHost/com.apple.loginwindow.*.plist; do
      [ -f "$f" ] && /usr/libexec/PlistBuddy -c "Delete :TALAppsToRelaunchAtLogin" "$f" 2>/dev/null; done
    pkill -x Terminal; sleep 1; rm -rf ~/Library/Saved\ Application\ State/com.apple.Terminal.savedState; true'
  VM="$BASE_VM" quiet_capture_notice
  VM="$BASE_VM" ensure_vmtool
  VM="$BASE_VM" stop
  if exists "$CLEAN_VM"; then tart delete "$CLEAN_VM"; fi
  tart clone "$BASE_VM" "$CLEAN_VM"
  echo "Ready: $BASE_VM (working VM) and $CLEAN_VM (never booted; reset clones it to $RUN_VM)"
}

reset() {
  exists "$CLEAN_VM" || die "no $CLEAN_VM (scripts/macvm/macvm.sh setup)"
  if running "$RUN_VM"; then VM="$RUN_VM" stop; fi
  if exists "$RUN_VM"; then tart delete "$RUN_VM"; fi
  tart clone "$CLEAN_VM" "$RUN_VM"
  echo "$RUN_VM recreated from $CLEAN_VM (start it: AUTOPAPER_MACVM=$RUN_VM scripts/macvm/macvm.sh start)"
}

shot() {
  local name="${1:?name}" remote="/tmp/macvm-shot.png"
  ensure_vmtool quick
  dismiss_capture_notice || true
  gui "screencapture -x -t png $remote"
  # The privacy notice appears just after a capture; if it did, dismiss it and capture again.
  sleep 1
  if dismiss_capture_notice; then sleep 1; gui "screencapture -x -t png $remote"; fi
  ssh_vm "cat $remote && rm -f $remote" > "$SHOTS/$name.png"
  echo "$SHOTS/$name.png"
}

winshot() {
  local owner="${1:?owner}" name="${2:?name}" id remote="/tmp/macvm-window.png"
  ensure_vmtool quick
  # The owner's first normal (layer 0) window; -o drops the shadow.
  id="$(gui "$VMTOOL windows $(printf '%q' "$owner")" | awk -F'\t' '$4 == "layer=0" { print $1; exit }')"
  [[ -n "$id" ]] || die "no on-screen window for $owner"
  gui "screencapture -x -o -t png -l $id $remote"
  sleep 1
  if dismiss_capture_notice; then sleep 1; gui "screencapture -x -o -t png -l $id $remote"; fi
  ssh_vm "cat $remote && rm -f $remote" > "$SHOTS/$name.png"
  echo "$SHOTS/$name.png (window $id)"
}

install_app() {
  local app="${1:-$APP_DEFAULT}"
  app="$(cd "$(dirname "$app")" && pwd)/$(basename "$app")"
  [[ -d "$app" ]] || die "no app at $app (build it on the Mac first)"
  # Copied over SSH (the repo isn't shared into the VM); tar keeps the bundle and its signature intact.
  COPYFILE_DISABLE=1 tar -C "$(dirname "$app")" -cf - "$(basename "$app")" |
    ssh_vm "rm -rf /Applications/AutoPaper.app && tar -xf - -C /tmp && mv '/tmp/$(basename "$app")' /Applications/AutoPaper.app"
  ssh_vm "codesign --verify --deep --strict /Applications/AutoPaper.app && echo 'signature: valid' &&
    codesign -dvv /Applications/AutoPaper.app 2>&1 | grep -E '^(Authority=Apple Dev|TeamIdentifier)'"
}

launch_app() {
  local since
  since="$(ssh_vm "date '+%Y-%m-%d %H:%M:%S'")"
  gui "open /Applications/AutoPaper.app"
  sleep 6
  if ssh_vm "pgrep -x AutoPaper >/dev/null"; then
    echo "AutoPaper is running (pid $(ssh_vm 'pgrep -x AutoPaper'))"
  else
    echo "AutoPaper is NOT running"
  fi
  # Code-signing / Gatekeeper / sandbox rejections show up from these processes.
  ssh_vm "log show --style compact --start '$since' --predicate '(process == \"amfid\" OR process == \"syspolicyd\" OR
    process == \"kernel\" OR process == \"taskgated-helper\" OR process == \"launchservicesd\") AND
    (eventMessage CONTAINS[c] \"AutoPaper\" OR eventMessage CONTAINS \"com.autopaper\")' 2>/dev/null |
    grep -v -E '^(Timestamp|Filtering)' | tail -n 20" || true
}

quit_app() {
  # SIGTERM ends it (an Apple Event "quit" would need an Automation permission prompt in the VM).
  ssh_vm "pkill -x AutoPaper && sleep 1; pgrep -x AutoPaper >/dev/null && echo 'still running' || echo 'AutoPaper quit'"
}

status() {
  tart list
  if running "$VM"; then echo "$VM: running at $(vm_ip)"; else echo "$VM: stopped"; fi
  du -sh "$HOME/.tart/cache" "$HOME/.tart/vms"/* 2>/dev/null || true
}

command="${1:-}"
shift || true
case "$command" in
  setup) setup ;;
  start) start ;;
  stop) stop ;;
  reset) reset ;;
  status) status ;;
  ip) vm_ip ;;
  ssh) if [[ $# -gt 0 ]]; then ssh_vm "$@"; else SSH_EXTRA=-t ssh_vm; fi ;;
  gui) gui "${1:?command}" ;;
  tool) ensure_vmtool && gui "$VMTOOL $(printf '%q ' "$@")" ;;
  shot) shot "$@" ;;
  winshot) winshot "$@" ;;
  install-app) install_app "$@" ;;
  launch-app) launch_app ;;
  quit-app) quit_app ;;
  *) sed -n '2,25p' "$0"; exit 2 ;;
esac
