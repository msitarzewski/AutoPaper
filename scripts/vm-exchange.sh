#!/usr/bin/env bash
# Hands the repository to the Windows VM without sharing the Mac's home folder with it.
#
# The "Windows 11" VM builds and signs AutoPaper next to a live Azure signing session, and builds third-party crates and
# packages. So it sees only one folder of the Mac, ~/vm-exchange (shared as \\Mac\Exchange), not the home folder:
#
#   scripts/vm-exchange.sh push        copy the source (no .git, .env files, build output or the other platforms' apps)
#                                      to ~/vm-exchange/src, and the VM scripts in .scratch/windows to ~/vm-exchange/scripts
#   scripts/vm-exchange.sh clean       empty ~/vm-exchange
#
# In the VM:  robocopy \\Mac\Exchange\src C:\Users\michael\dev\autopaper /E /XD bin obj target Generated ...
#             powershell -File scripts\windows-release.ps1 -Version X -CopyTo \\Mac\Exchange\out
# Then here:  scripts/publish-windows.sh X --from ~/vm-exchange/out/build/windows/X
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
EXCHANGE="${AUTOPAPER_VM_EXCHANGE:-$HOME/vm-exchange}"

case "${1:-}" in
  push)
    mkdir -p "$EXCHANGE/src" "$EXCHANGE/scripts" "$EXCHANGE/out"
    rsync -a --delete \
      --exclude '.git' --exclude '.scratch' --exclude '.agents' --exclude '.codex' --exclude '.aws' \
      --exclude '.env' --exclude '.env.*' --exclude 'target' --exclude 'build' --exclude 'release' \
      --exclude '_site' --exclude 'bin' --exclude 'obj' --exclude 'AppX' --exclude 'AppPackages' \
      --exclude 'apps/macos' --exclude 'apps/linux' --exclude 'apps/windows/Generated' \
      "$ROOT/" "$EXCHANGE/src/"
    if [ -d "$ROOT/.scratch/windows" ]; then
      rsync -a --delete --include '*.ps1' --exclude '*' "$ROOT/.scratch/windows/" "$EXCHANGE/scripts/"
    fi
    echo "Pushed the source to $EXCHANGE/src (VM: \\\\Mac\\Exchange\\src)"
    ;;
  clean)
    rm -rf "$EXCHANGE"/src "$EXCHANGE"/scripts "$EXCHANGE"/out
    echo "Emptied $EXCHANGE"
    ;;
  *)
    sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'
    exit 1
    ;;
esac
