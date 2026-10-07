#!/bin/bash
# Build AutoPaper for macOS and install it to /Applications (AudioPaper's scripts/install.sh, adapted).
#
#   scripts/macos-install.sh            # Release build, installed, not launched
#   scripts/macos-install.sh --open     # … and open it afterwards
#   scripts/macos-install.sh Debug      # Debug build instead
#
# Rebuilds the Rust core package first (scripts/build-xcframework.sh; seconds when the core hasn't changed).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CONFIGURATION="Release"
OPEN=0
for arg in "$@"; do
  case "$arg" in
    --open) OPEN=1 ;;
    Debug|Release) CONFIGURATION="$arg" ;;
    *) echo "usage: $0 [Debug|Release] [--open]" >&2; exit 2 ;;
  esac
done

APP_DIR="$ROOT/apps/macos"
DERIVED="$ROOT/build/macos-DerivedData"
BUILT="$DERIVED/Build/Products/$CONFIGURATION/AutoPaper.app"
TARGET="/Applications/AutoPaper.app"
LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"

"$ROOT/scripts/build-xcframework.sh"

cd "$APP_DIR"
xcodegen generate --quiet

# Sign with the Developer ID certificate releases will use, when this Mac has it: the Keychain ties saved API keys
# to the app's signature, so one signature means no password prompt per key when switching builds. Without the
# certificate (contributors), Xcode's automatic signing is used.
TEAM="$(grep -m1 'DEVELOPMENT_TEAM:' project.yml | awk '{print $2}')"
SIGNING=()
if security find-identity -v -p codesigning | grep "Developer ID Application" | grep -q "($TEAM)"; then
  SIGNING=(CODE_SIGN_STYLE=Manual CODE_SIGN_IDENTITY="Developer ID Application" PROVISIONING_PROFILE_SPECIFIER=)
fi

xcodebuild -project AutoPaper.xcodeproj -scheme AutoPaper -configuration "$CONFIGURATION" \
  -destination "generic/platform=macOS" -derivedDataPath "$DERIVED" -allowProvisioningUpdates -quiet build \
  ${SIGNING[@]+"${SIGNING[@]}"}

codesign --verify --deep --strict "$BUILT"

# Quit a running copy cleanly so it saves its state.
if pgrep -xq AutoPaper; then
  osascript -e 'tell application id "com.autopaper" to quit' || true
  for _ in {1..20}; do pgrep -xq AutoPaper || break; sleep 0.25; done
fi

rm -rf "$TARGET"
ditto "$BUILT" "$TARGET"
# ditto keeps the build's dates, and the Dock keys its icon cache on them; a fresh date makes it re-read the icon.
touch "$TARGET"
# One registered copy only, so Launch Services (and Spotlight, Shortcuts) find the installed app.
"$LSREGISTER" -u "$BUILT" 2>/dev/null || true
"$LSREGISTER" -f -R "$TARGET"

echo "Installed $CONFIGURATION build to $TARGET ($(defaults read "$TARGET/Contents/Info.plist" CFBundleShortVersionString) build $(defaults read "$TARGET/Contents/Info.plist" CFBundleVersion))."
codesign -dvv "$TARGET" 2>&1 | grep -E '^(Authority=Developer ID|Authority=Apple Development|TeamIdentifier)' | head -2
if [[ "$OPEN" == 1 ]]; then open "$TARGET"; fi
