#!/usr/bin/env bash
# Publishes a Windows release of AutoPaper on its update server: the .msixbundle first, then the App Installer feed
# that points to it, then checks both over HTTPS as Windows will fetch them.
#
#   scripts/publish-windows.sh 0.1.0              # build/windows/0.1.0/ → https://msitarzewski.com/app-updates/autopaper/
#   scripts/publish-windows.sh 0.1.0 --dry-run    # says what it would do
#   scripts/publish-windows.sh 0.1.0 --from DIR   # the release from DIR instead of build/windows/0.1.0/
#
# Run on the Mac, after scripts/windows-release.ps1 -Version <v> -CopyTo \\Mac\Home\Clean\autopaper (in the Windows VM)
# has put the release in build/windows/<v>/: AutoPaper_<v>_x64_arm64.msixbundle and the feed (AutoPaper.appinstaller, or
# a test feed). Where they go comes from the feed itself: its own Uri and its MainBundle Uri must both be under
# https://msitarzewski.com/app-updates/autopaper/, which is /srv/www/msitarzewski.com/app-updates/autopaper/ on pipx
# (Caddy; set up by scripts/pipx-app-updates.sh). So a test feed (another name, another bundle folder, made with
# windows-release.ps1 -FeedName/-BundleFolder) goes to its own place, and a development build's feed is refused at the
# real feed's address.
#
# Follows ~/Clean/pipx/DEPLOYING.md §1: runs as michael over `ssh pipx`, no sudo, touches only
# /srv/www/msitarzewski.com/app-updates/autopaper/. Files land 0640 and keep the folder's group (www-static, from its
# setgid bit: the group is never set here), so Caddy can read them. The folders exist already (pipx-app-updates.sh); a
# missing bundle folder (a test one) is made with mkdir, which inherits the setgid group.
#
# Order matters: the bundle is uploaded (rsync writes a temporary file and renames it, so no one ever downloads half a
# bundle) and its SHA-256 checked on the server before the feed that names it goes up; the feed is uploaded last. The
# bundle's name is versioned, so a new release never replaces a file someone is downloading. Bundles of older releases
# are kept: the newest --keep (default 3) in the folder stay, and the one the feed names is never removed; --keep 0
# keeps them all.
set -euo pipefail

BASE_URL=https://msitarzewski.com/app-updates/autopaper/
REMOTE_BASE=/srv/www/msitarzewski.com/app-updates/autopaper/
HOST=pipx
KEEP=3
DRY_RUN=0

usage() { sed -n '2,7p' "$0" | sed 's/^# \{0,1\}//'; exit "${1:-0}"; }

VERSION=""
FROM=""
while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run) DRY_RUN=1 ;;
        --keep) KEEP="${2:?--keep needs a number}"; shift ;;
        --keep=*) KEEP="${1#--keep=}" ;;
        --from) FROM="${2:?--from needs a folder}"; shift ;;
        --from=*) FROM="${1#--from=}" ;;
        -h|--help) usage 0 ;;
        -*) echo "unknown option $1" >&2; usage 1 ;;
        *) [ -z "$VERSION" ] || { echo "one version only" >&2; exit 1; }; VERSION="$1" ;;
    esac
    shift
done
[[ "$VERSION" =~ ^[0-9]{1,5}\.[0-9]{1,5}\.[0-9]{1,5}$ ]] || { echo "usage: $0 <version, e.g. 0.1.0> [--keep N] [--from DIR] [--dry-run]" >&2; exit 1; }
[[ "$KEEP" =~ ^[0-9]+$ ]] || { echo "--keep needs a number" >&2; exit 1; }

REPO=$(cd "$(dirname "$0")/.." && pwd)
RELEASE="${FROM:-$REPO/build/windows/$VERSION}"
[ -d "$RELEASE" ] || { printf 'no %s: run scripts/windows-release.ps1 -Version %s -CopyTo \\\\Mac\\Home\\Clean\\autopaper in the VM first\n' "$RELEASE" "$VERSION" >&2; exit 1; }

step() { printf '\n==> %s\n' "$*"; }
run() { if [ "$DRY_RUN" = 1 ]; then printf '    would run: %s\n' "$*"; else "$@"; fi; }
remote() { ssh -o BatchMode=yes "$HOST" "$@"; }
sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }

# ── The feed and what it names ───────────────────────────────────────────────────────────────

feeds=("$RELEASE"/*.appinstaller)
[ -f "${feeds[0]}" ] || { echo "no .appinstaller in $RELEASE" >&2; exit 1; }
[ ${#feeds[@]} -eq 1 ] || { echo "more than one .appinstaller in $RELEASE: ${feeds[*]}" >&2; exit 1; }
FEED=${feeds[0]}

attribute() { # attribute <element> <name>: the attribute's value in the feed (single-line elements, as written)
    sed -n "s/.*<$1 [^>]*$2=\"\([^\"]*\)\".*/\1/p" "$FEED" | head -1
}
feed_uri=$(attribute AppInstaller Uri)
feed_version=$(attribute AppInstaller Version)
bundle_uri=$(attribute MainBundle Uri)
[ "$feed_version" = "$VERSION.0" ] || { echo "$FEED is for version $feed_version, not $VERSION.0" >&2; exit 1; }
for uri in "$feed_uri" "$bundle_uri"; do
    case "$uri" in
        "$BASE_URL"?*) ;;
        *) echo "$FEED names $uri, which isn't under $BASE_URL" >&2; exit 1 ;;
    esac
    [[ "$uri" =~ ^[A-Za-z0-9:/._-]+$ ]] || { echo "unexpected characters in $uri" >&2; exit 1; }
    case "$uri" in *..*) echo "no .. in $uri" >&2; exit 1 ;; esac
done
feed_path=${feed_uri#"$BASE_URL"}        # AutoPaper.appinstaller
bundle_path=${bundle_uri#"$BASE_URL"}    # windows/AutoPaper_0.1.0_x64_arm64.msixbundle
[[ "$feed_path" != */* ]] || { echo "the feed must sit at $BASE_URL itself, not in a folder ($feed_uri)" >&2; exit 1; }
[[ "$bundle_path" =~ ^[A-Za-z0-9._-]+/AutoPaper_${VERSION//./\\.}_x64_arm64\.msixbundle$ ]] \
    || { echo "the bundle must be <folder>/AutoPaper_${VERSION}_x64_arm64.msixbundle under $BASE_URL ($bundle_uri)" >&2; exit 1; }
bundle_folder=${bundle_path%/*}
BUNDLE="$RELEASE/${bundle_path##*/}"
[ -f "$BUNDLE" ] || { echo "the feed names ${bundle_path##*/}, which isn't in $RELEASE" >&2; exit 1; }
if grep -q 'DEVELOPMENT BUILD' "$FEED" && [ "$feed_path" = AutoPaper.appinstaller ]; then
    echo "$FEED is a development build: it can't go to the real feed ($feed_uri). Test builds use their own feed name" >&2
    echo "(windows-release.ps1 -DevCert -FeedName test.appinstaller -BundleFolder windows-test)." >&2
    exit 1
fi
bundle_sha=$(sha256 "$BUNDLE")
bundle_size=$(stat -f %z "$BUNDLE")
feed_sha=$(sha256 "$FEED")

echo "Release $VERSION"
echo "  bundle  $BUNDLE ($bundle_size bytes, SHA-256 $bundle_sha)"
echo "          → $bundle_uri"
echo "  feed    $FEED"
echo "          → $feed_uri"

# ── The server ───────────────────────────────────────────────────────────────────────────────

step "Checking pipx (ssh $HOST, $REMOTE_BASE)"
remote "test \"\$(id -un)\" = michael && test -d '$REMOTE_BASE' && test -w '$REMOTE_BASE'" \
    || { echo "can't write $REMOTE_BASE on $HOST as michael (has scripts/pipx-app-updates.sh run?)" >&2; exit 1; }
if ! remote "test -d '$REMOTE_BASE$bundle_folder'"; then
    # A new folder (a test one): mkdir in the setgid base folder gives it the base's group (www-static) and setgid.
    run remote "umask 027 && mkdir '$REMOTE_BASE$bundle_folder' && stat -c '    made %n %A %U:%G' '$REMOTE_BASE$bundle_folder'"
fi

# The files go up 0640. The Mac's rsync (openrsync) ignores --chmod, so it gets copies with that mode (APFS clones:
# no extra space) and --perms; a GNU rsync 3 also gets --chmod=D2750,F0640. Neither sets the group.
staging=$(mktemp -d "${TMPDIR:-/tmp}/autopaper-publish.XXXXXX")
trap 'rm -rf "$staging"' EXIT
cp -c "$BUNDLE" "$staging/" 2>/dev/null || cp "$BUNDLE" "$staging/"
cp "$FEED" "$staging/"
chmod 0640 "$staging"/*
rsync_options=(-t --perms)
if rsync --version 2>/dev/null | grep -q '^rsync  *version 3'; then rsync_options+=("--chmod=D2750,F0640"); fi

step "Uploading the bundle to $REMOTE_BASE$bundle_path"
if [ "$DRY_RUN" = 0 ] && remote "test -f '$REMOTE_BASE$bundle_path'"; then
    existing=$(remote "sha256sum '$REMOTE_BASE$bundle_path' | cut -d' ' -f1")
    [ "$existing" = "$bundle_sha" ] || { echo "a different $bundle_path is already published: a release is never replaced (bump the version)" >&2; exit 1; }
    echo "    already there, the same file"
else
    run rsync "${rsync_options[@]}" "$staging/${BUNDLE##*/}" "$HOST:$REMOTE_BASE$bundle_folder/"
fi
if [ "$DRY_RUN" = 0 ]; then
    remote_sha=$(remote "sha256sum '$REMOTE_BASE$bundle_path' | cut -d' ' -f1")
    [ "$remote_sha" = "$bundle_sha" ] || { echo "the uploaded bundle's SHA-256 ($remote_sha) isn't $bundle_sha: the feed wasn't changed" >&2; exit 1; }
    echo "    SHA-256 on the server matches"
fi

step "Uploading the feed to $REMOTE_BASE$feed_path (last)"
run rsync "${rsync_options[@]}" "$staging/${FEED##*/}" "$HOST:$REMOTE_BASE$feed_path"
run remote "stat -c '    %n %a %U:%G %s bytes' '$REMOTE_BASE$feed_path' '$REMOTE_BASE$bundle_path'"

# ── As Windows sees it ───────────────────────────────────────────────────────────────────────

if [ "$DRY_RUN" = 0 ]; then
    step "Checking over HTTPS"
    header() { tr -d '\r' | grep -i "^$1:" | tail -1 | sed 's/^[^:]*: *//'; }
    feed_headers=$(curl -sS -I "$feed_uri")
    feed_status=$(printf '%s\n' "$feed_headers" | head -1 | awk '{print $2}')
    feed_type=$(printf '%s\n' "$feed_headers" | header content-type)
    feed_cache=$(printf '%s\n' "$feed_headers" | header cache-control)
    served_sha=$(curl -sS "$feed_uri" | shasum -a 256 | cut -d' ' -f1)
    echo "    $feed_uri → $feed_status, $feed_type, Cache-Control: ${feed_cache:-none}"
    bundle_headers=$(curl -sS -I "$bundle_uri")
    bundle_status=$(printf '%s\n' "$bundle_headers" | head -1 | awk '{print $2}')
    bundle_type=$(printf '%s\n' "$bundle_headers" | header content-type)
    bundle_length=$(printf '%s\n' "$bundle_headers" | header content-length)
    range_status=$(curl -sS -o /dev/null -w '%{http_code}' -r 0-1023 "$bundle_uri")
    echo "    $bundle_uri → $bundle_status, $bundle_type, $bundle_length bytes; a range request → $range_status"
    problems=()
    [ "$feed_status" = 200 ] || problems+=("the feed answers $feed_status")
    [ "$served_sha" = "$feed_sha" ] || problems+=("the feed served isn't the one uploaded (a cache?)")
    [ "$feed_type" = application/appinstaller ] || problems+=("the feed's type is $feed_type, not application/appinstaller")
    [[ "$feed_cache" == *no-cache* ]] || problems+=("the feed has no Cache-Control: no-cache")
    [ "$bundle_status" = 200 ] || problems+=("the bundle answers $bundle_status")
    [ "$bundle_type" = application/msixbundle ] || problems+=("the bundle's type is $bundle_type, not application/msixbundle")
    [ "$bundle_length" = "$bundle_size" ] || problems+=("the bundle's Content-Length is $bundle_length, not $bundle_size")
    [ "$range_status" = 206 ] || problems+=("a range request answers $range_status, not 206 (App Installer downloads in ranges)")
    if [ ${#problems[@]} -gt 0 ]; then
        printf '    PROBLEM: %s\n' "${problems[@]}"
        echo "Published, but Windows may not take it as it is (scripts/pipx-app-updates.sh sets the types and no-cache)." >&2
        exit 1
    fi
fi

# ── Older bundles ────────────────────────────────────────────────────────────────────────────

if [ "$KEEP" -gt 0 ]; then
    step "Keeping the newest $KEEP bundles in $REMOTE_BASE$bundle_folder"
    # Sorted by version on the server (GNU sort -V); everything but the newest KEEP.
    old=$(remote "cd '$REMOTE_BASE$bundle_folder' 2>/dev/null && ls -1 | grep -E '^AutoPaper_[0-9]+\\.[0-9]+\\.[0-9]+_x64_arm64\\.msixbundle\$' | sort -t_ -k2,2V | head -n -$KEEP || true")
    removed=0
    for name in $old; do
        [ "$name" = "${bundle_path##*/}" ] && continue # never the one the feed names
        run remote "rm -f -- '$REMOTE_BASE$bundle_folder/$name'"
        echo "    removed $name"
        removed=$((removed + 1))
    done
    [ "$removed" -gt 0 ] || echo "    nothing to remove"
fi

step "Done"
echo "Windows checks $feed_uri when AutoPaper launches (and every 8 hours) and installs $VERSION by the next launch."
