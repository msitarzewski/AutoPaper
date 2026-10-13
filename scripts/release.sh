#!/bin/bash
# Build a signed, notarized, stapled AutoPaper release for macOS: build/release/AutoPaper-<version>.dmg (the
# download), AutoPaper-<version>.zip (the Sparkle update) and its entry in the feed, site/static/appcast.xml.
# AudioPaper's scripts/release.sh, adapted: the Rust core and the embedding model are built and bundled first, and
# there is no fanart.tv key.
#
# Run on the maintainer's Mac, which holds the "Developer ID Application" certificate for the team in
# apps/macos/project.yml and the Sparkle signing key (`generate_keys --account AutoPaper`, in the login Keychain;
# each time sign_update uses it macOS asks: choose Allow, not Always Allow, so only this script can sign an update). Needs, in the environment:
#   APPLE_NOTARY_PROFILE (a notarytool keychain profile: xcrun notarytool store-credentials …), or else
#   APPLE_ID, APPLE_PASSWORD (an app-specific password), APPLE_TEAM_ID   — for notarytool (the password is then on its command line)
#   RELEASE_NOTES_HTML (optional) — a file with an HTML fragment for Sparkle's update window
#
#   set -a; source ~/.config/brew-browser/signing.env; set +a   # or wherever your credentials live
#   scripts/release.sh
#
# Flow: the Rust core (scripts/build-xcframework.sh) → the embedding model (scripts/fetch-model.sh, checksums
#       verified) → XcodeGen → the Sparkle key matches the app's SUPublicEDKey → archive (Release) → export with
#       Developer ID (Sparkle's helpers too, hardened runtime) → verify signatures, entitlements, every executable
#       inside, Info.plist and the bundled model → notarize the app → staple
#       → disk image with an Applications link → sign → notarize → staple
#       → Sparkle update zip, signed with the EdDSA key → feed entry (scripts/appcast.py) → checksums.
# The Sparkle signature comes last, so a Keychain question there never holds up Apple's notarization.
# Afterwards: publish the GitHub release with the .dmg and .zip, THEN push the feed (it points at the zip).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_DIR="$ROOT/apps/macos"
SPEC="$APP_DIR/project.yml"
cd "$ROOT"

# notarytool credentials: a stored keychain profile keeps the password out of the process list (xcrun notarytool
# store-credentials AutoPaper-notary --apple-id … --team-id …, then APPLE_NOTARY_PROFILE=AutoPaper-notary). Without one,
# the app-specific password goes on notarytool's command line, which other processes of this user can read.
if [ -n "${APPLE_NOTARY_PROFILE:-}" ]; then
  NOTARY_AUTH=(--keychain-profile "$APPLE_NOTARY_PROFILE")
else
  : "${APPLE_ID:?set APPLE_ID (see the header of this script)}"
  : "${APPLE_PASSWORD:?set APPLE_PASSWORD (an app-specific password)}"
  : "${APPLE_TEAM_ID:?set APPLE_TEAM_ID}"
  NOTARY_AUTH=(--apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID")
fi

setting() { grep -m1 -E "^ +$1:" "$SPEC" | cut -d: -f2- | tr -d '" '; }
TEAM="$(setting DEVELOPMENT_TEAM)"
[ -z "${APPLE_TEAM_ID:-}" ] || [ "$TEAM" = "$APPLE_TEAM_ID" ] || { echo "project.yml team ($TEAM) doesn't match APPLE_TEAM_ID"; exit 1; }
IDENTITY="$(security find-identity -v -p codesigning | grep "Developer ID Application" | grep "($TEAM)" | head -1 | awk '{print $2}')"
[ -n "$IDENTITY" ] || { echo "No Developer ID Application certificate for team $TEAM in the Keychain"; exit 1; }
VERSION="$(setting MARKETING_VERSION)"
BUILD="$(setting CURRENT_PROJECT_VERSION)"
PUBLIC_KEY="$(setting SUPublicEDKey)"
FEED_URL="$(setting SPARKLE_FEED_URL)"
BUNDLE_ID="$(setting PRODUCT_BUNDLE_IDENTIFIER)"

OUT="$ROOT/build/release"
PACKAGES="$ROOT/build/SourcePackages"
SPARKLE_BIN="$PACKAGES/artifacts/sparkle/Sparkle/bin"
MODEL_SRC="$ROOT/models/bge-small-en-v1.5"
MODEL_FILES=(config.json tokenizer.json model.safetensors)
ZIP="$OUT/AutoPaper-$VERSION.zip"
ARCHIVE="$OUT/AutoPaper.xcarchive"
EXPORT="$OUT/export"
APP="$EXPORT/AutoPaper.app"
DMG="$OUT/AutoPaper-$VERSION.dmg"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
rm -rf "$OUT" && mkdir -p "$OUT"

notarize() {
  # The result goes to a file first: read through a pipe, `grep -q` could close it before notarytool is done.
  xcrun notarytool submit "$1" "${NOTARY_AUTH[@]}" \
    --wait --output-format json > "$WORK/notary.json" || true
  local id
  id="$(sed -nE 's/.*"id" *: *"([^"]+)".*/\1/p' "$WORK/notary.json" | head -1)"
  if ! grep -q '"status" *: *"Accepted"' "$WORK/notary.json"; then
    echo "Notarization failed:"; cat "$WORK/notary.json"
    [ -n "$id" ] && xcrun notarytool log "$id" "${NOTARY_AUTH[@]}"
    exit 1
  fi
  echo "   accepted (submission $id)"
}

# Fails unless `bundle` is signed with the Developer ID certificate, with a secure timestamp (notarization needs
# both); `runtime` also requires the hardened runtime.
check_signed() {
  local bundle="$1" info
  # Captured first: with pipefail, `grep -q` closing the pipe early would read as a failure.
  info="$(codesign -dvv "$bundle" 2>&1)" || { echo "$bundle: not signed"; exit 1; }
  grep -q "Authority=Developer ID Application" <<<"$info" || { echo "$bundle: not Developer ID signed"; exit 1; }
  grep -q "^Timestamp=" <<<"$info" || { echo "$bundle: no secure timestamp"; exit 1; }
  if [ "${2:-}" = runtime ]; then
    grep -q "flags=.*runtime" <<<"$info" || { echo "$bundle: hardened runtime missing"; exit 1; }
  fi
}

plist() { /usr/libexec/PlistBuddy -c "Print :$1" "$2" 2>/dev/null; }

echo "==> AutoPaper $VERSION ($BUILD), team $TEAM"

echo "==> the Rust core"
"$ROOT/scripts/build-xcframework.sh"

echo "==> the embedding model"
"$ROOT/scripts/fetch-model.sh" "$MODEL_SRC"

echo "==> project and packages"
(cd "$APP_DIR" && xcodegen generate --quiet)
xcodebuild -project "$APP_DIR/AutoPaper.xcodeproj" -scheme AutoPaper -resolvePackageDependencies \
  -clonedSourcePackagesDirPath "$PACKAGES" -quiet
# Sparkle is what installs updates, so the commit that resolved must be the one reviewed for 2.10.0, not whatever a
# moved tag points at (project.yml pins the version; this pins the commit).
SPARKLE_COMMIT="eef1a539a373c1f1a320624b1130fc5de7b2e100"
RESOLVED="$(python3 -c 'import json,sys; print(next(p["state"]["revision"] for p in json.load(open(sys.argv[1]))["pins"] if p["identity"]=="sparkle"))' \
  "$APP_DIR/AutoPaper.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved")"
[ "$RESOLVED" = "$SPARKLE_COMMIT" ] || { echo "Sparkle resolved to $RESOLVED, not the reviewed $SPARKLE_COMMIT"; exit 1; }
[ -x "$SPARKLE_BIN/sign_update" ] || { echo "Sparkle tools missing at $SPARKLE_BIN"; exit 1; }
# The app trusts updates signed with SUPublicEDKey; the key in the Keychain must be its private half.
KEYCHAIN_KEY="$("$SPARKLE_BIN/generate_keys" --account AutoPaper -p 2>/dev/null)" \
  || { echo "No Sparkle key for AutoPaper in the Keychain (generate_keys --account AutoPaper)"; exit 1; }
[ "$KEYCHAIN_KEY" = "$PUBLIC_KEY" ] || { echo "The Keychain's Sparkle key isn't the one SUPublicEDKey in project.yml trusts"; exit 1; }

echo "==> archive (Release)"
xcodebuild -project "$APP_DIR/AutoPaper.xcodeproj" -scheme AutoPaper -configuration Release \
  -destination "generic/platform=macOS" -archivePath "$ARCHIVE" -clonedSourcePackagesDirPath "$PACKAGES" \
  -allowProvisioningUpdates -quiet archive

echo "==> export with Developer ID"
cat > "$WORK/ExportOptions.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>method</key><string>developer-id</string>
  <key>teamID</key><string>$TEAM</string>
  <key>signingStyle</key><string>automatic</string>
</dict>
</plist>
PLIST
xcodebuild -exportArchive -archivePath "$ARCHIVE" -exportPath "$EXPORT" \
  -exportOptionsPlist "$WORK/ExportOptions.plist" -allowProvisioningUpdates -quiet

echo "==> verify"
codesign --verify --deep --strict --verbose=2 "$APP"
# The app and Sparkle's helpers: Developer ID, timestamp, hardened runtime.
SPARKLE_FW="$APP/Contents/Frameworks/Sparkle.framework"
for bundle in "$APP" "$SPARKLE_FW" "$SPARKLE_FW/Versions/B/Autoupdate" "$SPARKLE_FW/Versions/B/Updater.app" \
  "$SPARKLE_FW/Versions/B/XPCServices/Installer.xpc" "$SPARKLE_FW/Versions/B/XPCServices/Downloader.xpc"; do
  [ -e "$bundle" ] || { echo "$bundle: missing"; exit 1; }
  check_signed "$bundle" runtime
done
# Every other executable inside (there should be none beyond Sparkle's; the Rust core is linked in statically).
while IFS= read -r -d '' file; do
  if file -b "$file" | grep -q "Mach-O"; then check_signed "$file"; fi
done < <(find "$APP" -type f -perm -u+x -print0)
BINARY="$APP/Contents/MacOS/AutoPaper"
archs=" $(lipo -archs "$BINARY") "
[[ "$archs" == *" arm64 "* && "$archs" == *" x86_64 "* ]] || { echo "AutoPaper isn't universal:$archs"; exit 1; }
unexpected="$(otool -L "$BINARY" | grep -E '^[[:space:]]' | awk '{print $1}' \
  | grep -v -E '^(/usr/lib/|/System/Library/|@rpath/Sparkle\.framework/)' || true)"
[ -z "$unexpected" ] || { echo "AutoPaper links libraries it doesn't carry: $unexpected"; exit 1; }
# Sparkle must check an update's signature before it unpacks it.
[ "$(plutil -extract SUVerifyUpdateBeforeExtraction raw "$APP/Contents/Info.plist" 2>/dev/null)" = true ] \
  || { echo "app: Sparkle isn't set to verify updates before extracting them"; exit 1; }
[ "$(plutil -extract SURequireSignedFeed raw "$APP/Contents/Info.plist" 2>/dev/null)" = true ] \
  || { echo "app: Sparkle isn't set to require a signed feed"; exit 1; }
# Entitlements: sandboxed, network client, Sparkle's installer channels, and never get-task-allow.
codesign -d --entitlements - --xml "$APP" > "$WORK/app.entitlements" 2>/dev/null
[ "$(plist com.apple.security.app-sandbox "$WORK/app.entitlements")" = true ] || { echo "app: not sandboxed"; exit 1; }
[ "$(plist com.apple.security.network.client "$WORK/app.entitlements")" = true ] || { echo "app: no network.client"; exit 1; }
if plist com.apple.security.get-task-allow "$WORK/app.entitlements" >/dev/null; then echo "app: carries get-task-allow"; exit 1; fi
lookups="$(plist com.apple.security.temporary-exception.mach-lookup.global-name "$WORK/app.entitlements")"
for name in "$BUNDLE_ID-spks" "$BUNDLE_ID-spki"; do
  grep -qx " *$name" <<<"$lookups" || { echo "app: mach-lookup exception $name missing"; exit 1; }
done
# Info.plist: the published feed (not a test feed), the right key, the installer launcher, the version.
INFO="$APP/Contents/Info.plist"
[ "$(plist SUFeedURL "$INFO")" = "$FEED_URL" ] && [[ "$FEED_URL" == https://* ]] || { echo "SUFeedURL isn't $FEED_URL"; exit 1; }
[ "$(plist SUPublicEDKey "$INFO")" = "$PUBLIC_KEY" ] || { echo "SUPublicEDKey mismatch"; exit 1; }
[ "$(plist SUEnableInstallerLauncherService "$INFO")" = true ] || { echo "SUEnableInstallerLauncherService missing"; exit 1; }
[ "$(plist CFBundleShortVersionString "$INFO")" = "$VERSION" ] && [ "$(plist CFBundleVersion "$INFO")" = "$BUILD" ] \
  || { echo "version mismatch"; exit 1; }
# The embedding model, byte for byte the verified copy (sealed by the app's signature like any resource).
for file in "${MODEL_FILES[@]}"; do
  cmp -s "$MODEL_SRC/$file" "$APP/Contents/Resources/Models/bge-small-en-v1.5/$file" \
    || { echo "the bundled embedding model's $file is missing or different"; exit 1; }
done
echo "   signatures, entitlements, Info.plist and the embedding model are as expected"

echo "==> notarize the app (waits for Apple)"
ditto -c -k --keepParent "$APP" "$WORK/AutoPaper.zip"
notarize "$WORK/AutoPaper.zip"
xcrun stapler staple "$APP"
spctl --assess --type execute --verbose=2 "$APP"

echo "==> disk image"
mkdir -p "$WORK/dmg"
ditto "$APP" "$WORK/dmg/AutoPaper.app"
ln -s /Applications "$WORK/dmg/Applications"
hdiutil create -volname "AutoPaper $VERSION" -srcfolder "$WORK/dmg" -ov -format UDZO "$DMG" >/dev/null
codesign --force --timestamp --sign "$IDENTITY" "$DMG"
echo "==> notarize the disk image (waits for Apple)"
notarize "$DMG"
xcrun stapler staple "$DMG"
spctl --assess --type open --context context:primary-signature --verbose=2 "$DMG"

echo "==> Sparkle update: zip, EdDSA signature, feed entry"
echo "   (macOS may ask whether sign_update may use the AutoPaper key: choose Allow, not Always Allow)"
ditto -c -k --keepParent "$APP" "$ZIP"
SIGNATURE="$("$SPARKLE_BIN/sign_update" --account AutoPaper -p "$ZIP")"
[ -n "$SIGNATURE" ] || { echo "sign_update produced no signature"; exit 1; }
python3 "$ROOT/scripts/appcast.py" "$VERSION" "$BUILD" "$ZIP" "$SIGNATURE" ${RELEASE_NOTES_HTML:+"$RELEASE_NOTES_HTML"}
# The feed itself is signed too (SURequireSignedFeed): whoever can edit the file on its host can't change the notes,
# the download address or the version without the key. Nothing may touch the file after this.
"$SPARKLE_BIN/sign_update" --account AutoPaper "$ROOT/site/static/appcast.xml" >/dev/null
"$SPARKLE_BIN/sign_update" --account AutoPaper --verify "$ROOT/site/static/appcast.xml" \
  || { echo "the signed feed doesn't verify"; exit 1; }

(cd "$OUT" && shasum -a 256 "$(basename "$DMG")" "$(basename "$ZIP")" > "AutoPaper-$VERSION.sha256")
echo
echo "==> done: $DMG and $ZIP; site/static/appcast.xml updated"
cat "$OUT/AutoPaper-$VERSION.sha256"
echo "Next: publish the GitHub release v$VERSION with both files, then commit and push the feed."
