#!/usr/bin/env bash
# Builds AutoPaper's Flatpak from this checkout and makes its bundle, AutoPaper-<version>-<arch>.flatpak, and its
# .sha256 under build/flatpak/out/. CI runs it (.github/workflows/flatpak.yml, in Flathub's GNOME 51 builder container)
# and so can any Linux machine with flatpak.
#
# The bundle is the build's output, unsigned. scripts/publish-flatpak.sh (on the Mac, with the repository's signing key)
# imports it into AutoPaper's own Flatpak repository, https://msitarzewski.com/app-updates/autopaper/flatpak, and makes
# the GitHub release's downloads from the signed repository. To try a build here:
#   flatpak install --user build/flatpak/out/AutoPaper-<version>-<arch>.flatpak
#
#   scripts/linux-release.sh [--arch ARCH] [--update-cargo-sources] [--lint] [--allow-lint-errors]
#
#   --arch ARCH              aarch64 or x86_64 (default: this machine's). CI builds each on its own runner; building
#                            for the other here needs qemu-user-static registered with binfmt_misc and that
#                            architecture's runtime, SDK and Rust extension, and then the compiler runs emulated (hours).
#   --update-cargo-sources   regenerate packaging/flatpak/cargo-sources.json when it no longer matches Cargo.lock
#                            (otherwise a stale one stops the script)
#   --lint                   also run flatpak-builder-lint (manifest, build directory, repository): Flathub's linter, a
#                            useful check of the manifest, permissions and AppStream data even off Flathub
#   --allow-lint-errors      with --lint, finish with success even when the linter reports errors (they're shown)
#
# Needs flatpak and python3 (3.11+), and flatpak-builder: the Flatpak org.flatpak.Builder (flatpak install flathub
# org.flatpak.Builder; it also has flatpak-builder-lint), else the system's with elfutils (as in CI's container). The GNOME
# runtime and SDK and the Rust extension are installed from Flathub (per user) when they aren't installed yet.
set -euo pipefail

APP_ID="io.github.msitarzewski.AutoPaper"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PACKAGING="$ROOT/packaging/flatpak"
MANIFEST="$PACKAGING/$APP_ID.yml"
CARGO_SOURCES="$PACKAGING/cargo-sources.json"
# flatpak-cargo-generator, pinned (flatpak/flatpak-builder-tools, cargo/flatpak-cargo-generator.py).
GENERATOR_COMMIT="74697c75b630d7330e77250fc13cb5ea688d9479"
GENERATOR_SHA256="0a2db6be87d75910facef28ab46d4d6460802e8419ab850d0caa6a364d26b380"
# The GNOME runtime comes from Flathub, for the build and for every installation.
FLATHUB_REPO="https://dl.flathub.org/repo/flathub.flatpakrepo"

ARCH=""
UPDATE_SOURCES=0
LINT=0
ALLOW_LINT_ERRORS=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --arch) ARCH="$2"; shift 2 ;;
    --update-cargo-sources) UPDATE_SOURCES=1; shift ;;
    --lint) LINT=1; shift ;;
    --allow-lint-errors) ALLOW_LINT_ERRORS=1; shift ;;
    *) sed -n '2,26p' "$0"; exit 2 ;;
  esac
done

die() { echo "linux-release: $*" >&2; exit 1; }
step() { printf '\n== %s\n' "$*"; }

command -v flatpak >/dev/null || die "flatpak is needed"
command -v python3 >/dev/null || die "python3 (3.11 or newer) is needed"
# flatpak-builder: Flathub's org.flatpak.Builder when it's installed (it carries every tool a build calls), else the
# system's (as in CI's container), which runs eu-strip from elfutils outside the build sandbox.
if flatpak info org.flatpak.Builder >/dev/null 2>&1; then
  builder() { flatpak run --command="$1" org.flatpak.Builder "${@:2}"; }
elif command -v flatpak-builder >/dev/null; then
  command -v eu-strip >/dev/null || die "the system's flatpak-builder needs eu-strip (Ubuntu: apt install elfutils)"
  builder() { "$@"; }
else
  die "flatpak-builder is needed: flatpak install flathub org.flatpak.Builder (or the system's, with elfutils)"
fi

VERSION="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml")"
[[ -n "$VERSION" ]] || die "no [workspace.package] version in Cargo.toml"
ARCH="${ARCH:-$(flatpak --default-arch)}"
case "$ARCH" in aarch64 | x86_64) ;; *) die "--arch must be aarch64 or x86_64" ;; esac
if [[ "$ARCH" != "$(flatpak --default-arch)" ]] && ! flatpak --supported-arches | grep -qx "$ARCH"; then
  die "this machine can't run $ARCH builds: install qemu-user-static (with binfmt_misc) first"
fi

WORK="$ROOT/build/flatpak/$ARCH"
OUT="$ROOT/build/flatpak/out"
mkdir -p "$WORK" "$OUT"

# The metainfo's newest release is the one being made (GNOME Software shows its notes).
METAINFO="$ROOT/apps/linux/data/$APP_ID.metainfo.xml"
release="$(sed -n 's/.*<release version="\([^"]*\)".*/\1/p' "$METAINFO" | head -n 1)"
[[ "$release" == "$VERSION" ]] ||
  die "the metainfo's newest release is $release, Cargo.toml says $VERSION: add a <release> for $VERSION first"

step "cargo-sources.json against Cargo.lock"
# Every crates.io package (by checksum) and git dependency (by commit) in Cargo.lock must be in cargo-sources.json,
# and nothing else.
sources_stale() {
  python3 - "$ROOT/Cargo.lock" "$CARGO_SOURCES" <<'PY'
import json, sys, tomllib
from urllib.parse import urlparse

with open(sys.argv[1], "rb") as f:
    lock = tomllib.load(f)
want, want_git = set(), set()
for package in lock.get("package", []):
    source = package.get("source", "")
    if source.startswith("registry+"):
        want.add(package["checksum"])
    elif source.startswith("git+"):
        url = urlparse(source[len("git+"):])
        want_git.add((f"{url.scheme}://{url.netloc}{url.path}", url.fragment))
have, have_git = set(), set()
with open(sys.argv[2]) as f:
    for source in json.load(f):
        if source.get("type") == "archive" and "static.crates.io" in source.get("url", ""):
            have.add(source["sha256"])
        elif source.get("type") == "git":
            have_git.add((source["url"], source.get("commit")))
missing, extra = len(want - have) + len(want_git - have_git), len(have - want) + len(have_git - want_git)
if missing or extra:
    print(f"{missing} package(s) missing, {extra} no longer used")
    sys.exit(1)
PY
}
if [[ ! -f "$CARGO_SOURCES" ]] || ! report="$(sources_stale)"; then
  [[ "$UPDATE_SOURCES" == 1 ]] ||
    die "packaging/flatpak/cargo-sources.json doesn't match Cargo.lock (${report:-missing}); run again with --update-cargo-sources"
  echo "Regenerating it (${report:-missing})"
  tools="$ROOT/build/flatpak/tools"
  mkdir -p "$tools"
  generator="$tools/flatpak-cargo-generator.py"
  curl --fail --location --silent --show-error -o "$generator.part" \
    "https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/$GENERATOR_COMMIT/cargo/flatpak-cargo-generator.py"
  [[ "$(sha256sum "$generator.part" | cut -d ' ' -f 1)" == "$GENERATOR_SHA256" ]] ||
    die "flatpak-cargo-generator.py doesn't match its pinned SHA-256"
  mv "$generator.part" "$generator"
  # Its dependencies (aiohttp, tomlkit) go into a virtual environment next to it.
  if ! "$tools/venv/bin/python" -c 'import aiohttp, tomlkit' 2>/dev/null; then
    rm -rf "$tools/venv"
    python3 -m venv "$tools/venv" || { rm -rf "$tools/venv"; die "python3 -m venv failed (Ubuntu: apt install python3-venv)"; }
    "$tools/venv/bin/pip" install --quiet 'aiohttp>=3.9.5,<4' 'tomlkit>=0.13.3,<1'
  fi
  "$tools/venv/bin/python" "$generator" "$ROOT/Cargo.lock" -o "$CARGO_SOURCES"
  sources_stale >/dev/null || die "the regenerated cargo-sources.json still doesn't match Cargo.lock"
fi
echo "Up to date with Cargo.lock."

step "Runtime, SDK and Rust extension ($ARCH)"
runtime_version="$(sed -n "s/^runtime-version: '\(.*\)'/\1/p" "$MANIFEST")"
deps=("org.gnome.Platform/$ARCH/$runtime_version" "org.gnome.Sdk/$ARCH/$runtime_version")
install_deps=()
for ref in "${deps[@]}"; do
  flatpak info "$ref" >/dev/null 2>&1 || install_deps=(--install-deps-from=flathub)
done
if [[ ${#install_deps[@]} == 0 ]]; then
  # The Rust extension's branch is the freedesktop SDK's that the GNOME SDK is built on.
  sdk_branch="$(flatpak info -m "org.gnome.Sdk/$ARCH/$runtime_version" |
    sed -n '/^\[Extension org.freedesktop.Sdk.Extension\]/,/^\[/s/^version *= *//p' | head -n 1)"
  flatpak info "org.freedesktop.Sdk.Extension.rust-stable/$ARCH/${sdk_branch:-none}" >/dev/null 2>&1 ||
    install_deps=(--install-deps-from=flathub)
fi
if [[ ${#install_deps[@]} -gt 0 ]]; then
  echo "Some are missing: flatpak-builder installs them for this user from Flathub."
  flatpak remote-add --user --if-not-exists flathub "$FLATHUB_REPO"
else
  echo "Installed."
fi

step "Building $APP_ID $VERSION for $ARCH (log: build/flatpak/$ARCH/build.log)"
# Branch "stable": the one AutoPaper's repository publishes. Not --sandbox: the manifest's sources are the checkout,
# outside packaging/flatpak. No rofiles-fuse: it can't mount inside org.flatpak.Builder's sandbox or CI's container.
started=$(date +%s)
set +e
builder flatpak-builder --force-clean --disable-rofiles-fuse --user --arch="$ARCH" --default-branch=stable \
  "${install_deps[@]}" --state-dir="$WORK/state" --repo="$WORK/repo" "$WORK/builddir" "$MANIFEST" 2>&1 |
  tee "$WORK/build.log" |
  sed -u 's/\r/\n/g' | grep --line-buffered -Ev '^ *(Compiling |[0-9]+ +[0-9]|% Total|Dload)'
built="${PIPESTATUS[0]}"
set -e
[[ "$built" == 0 ]] || die "the build failed; see build/flatpak/$ARCH/build.log"
echo "Built in $(( ($(date +%s) - started) / 60 )) min $(( ($(date +%s) - started) % 60 )) s."

step "AppStream screenshots"
# GNOME Software shows the screenshots from the metainfo's URLs, pinned to the release's tag. appstreamcli compose drops
# any it can't download (screenshot-download-error), so a build made before the tag is pushed has none: a tag build
# (CI on the tag's push) must keep them all.
count_screenshots() { { grep -o '<screenshot[ >]' || true; } | wc -l | tr -d ' '; }
catalog="$WORK/builddir/files/share/app-info/xmls/$APP_ID.xml.gz"
[[ -f "$catalog" ]] || die "flatpak-builder made no AppStream catalog ($catalog)"
wanted="$(count_screenshots < "$METAINFO")"
kept="$(gzip -dc "$catalog" | count_screenshots)"
if [[ "$kept" == "$wanted" ]]; then
  echo "All $wanted kept."
elif [[ "${GITHUB_REF_TYPE:-}" == tag ]]; then
  die "the AppStream catalog kept $kept of the metainfo's $wanted screenshots: are their URLs (tag v$VERSION) reachable?"
else
  echo "Only $kept of $wanted kept: their URLs are pinned to the tag v$VERSION, which isn't pushed yet (fine for a test build)."
fi

step "Bundle"
# Unsigned and pointing nowhere for updates: publish-flatpak.sh makes the signed downloads from the repository.
bundle="$OUT/AutoPaper-$VERSION-$ARCH.flatpak"
flatpak build-bundle --arch="$ARCH" --runtime-repo="$FLATHUB_REPO" "$WORK/repo" "$bundle" "$APP_ID" stable
(cd "$OUT" && sha256sum "$(basename "$bundle")" > "$(basename "$bundle").sha256")
echo "$bundle ($(du -h "$bundle" | cut -f 1))"
cat "$bundle.sha256"

lint_failed=()
if [[ "$LINT" == 1 ]]; then
  step "flatpak-builder-lint"
  lint() {
    local name="$1"
    shift
    echo "-- $name"
    if builder flatpak-builder-lint "$@"; then
      echo "clean"
    else
      lint_failed+=("$name")
    fi
  }
  lint "manifest" manifest "$MANIFEST"
  lint "build directory" builddir "$WORK/builddir"
  lint "repository" repo "$WORK/repo"
fi

step "Done"
echo "Bundle:   $bundle"
echo "SHA-256:  $bundle.sha256"
echo "Try it:   flatpak install --user $bundle"
echo "Publish:  scripts/publish-flatpak.sh (on the Mac), which signs it into AutoPaper's repository"
if [[ ${#lint_failed[@]} -gt 0 ]]; then
  printf -v failed '%s, ' "${lint_failed[@]}"
  echo "The linter reported errors for: ${failed%, } (docs.flathub.org/linter explains each)."
  [[ "$ALLOW_LINT_ERRORS" == 1 ]] || exit 1
fi
