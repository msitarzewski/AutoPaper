#!/usr/bin/env bash
# Builds AutoPaper's release Flatpak from this checkout and checks it with Flathub's linter. Writes, under
# build/flatpak/out/: the single-file bundle AutoPaper-<version>-<arch>.flatpak and its .sha256, and flathub/ with the
# files a Flathub submission needs (the manifest with the app's source pinned to the release tag and commit, and
# cargo-sources.json). It opens no pull request and pushes nothing.
#
#   scripts/linux-release.sh [--arch ARCH] [--tag TAG] [--commit SHA] [--update-cargo-sources] [--allow-lint-errors]
#
#   --arch ARCH              aarch64 or x86_64 (default: this machine's). Building for the other one needs
#                            qemu-user-static registered with binfmt_misc and that architecture's runtime, SDK and Rust
#                            extension; the compiler then runs emulated, which takes hours.
#   --tag TAG                the release tag the Flathub manifest builds (default: v<version> from Cargo.toml)
#   --commit SHA             the tag's commit (default: what TAG points to in this checkout)
#   --update-cargo-sources   regenerate packaging/flatpak/cargo-sources.json when it no longer matches Cargo.lock
#                            (otherwise a stale one stops the script)
#   --allow-lint-errors      finish with success even when the linter reports errors (they're still shown)
#
# Needs flatpak with the Flathub remote, python3 (3.11+), and org.flatpak.Builder, which has flatpak-builder and
# flatpak-builder-lint as Flathub runs them: flatpak install flathub org.flatpak.Builder. The GNOME runtime and SDK and
# the Rust extension are installed from Flathub (per user) when they aren't installed yet.
set -euo pipefail

APP_ID="io.github.msitarzewski.AutoPaper"
REPO_URL="https://github.com/msitarzewski/AutoPaper.git"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PACKAGING="$ROOT/packaging/flatpak"
MANIFEST="$PACKAGING/$APP_ID.yml"
CARGO_SOURCES="$PACKAGING/cargo-sources.json"
# flatpak-cargo-generator, pinned (flatpak/flatpak-builder-tools, cargo/flatpak-cargo-generator.py).
GENERATOR_COMMIT="74697c75b630d7330e77250fc13cb5ea688d9479"
GENERATOR_SHA256="0a2db6be87d75910facef28ab46d4d6460802e8419ab850d0caa6a364d26b380"
FLATHUB_REPO="https://dl.flathub.org/repo/flathub.flatpakrepo"

ARCH=""
TAG=""
COMMIT=""
UPDATE_SOURCES=0
ALLOW_LINT_ERRORS=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --arch) ARCH="$2"; shift 2 ;;
    --tag) TAG="$2"; shift 2 ;;
    --commit) COMMIT="$2"; shift 2 ;;
    --update-cargo-sources) UPDATE_SOURCES=1; shift ;;
    --allow-lint-errors) ALLOW_LINT_ERRORS=1; shift ;;
    *) sed -n '2,20p' "$0"; exit 2 ;;
  esac
done

die() { echo "linux-release: $*" >&2; exit 1; }
step() { printf '\n== %s\n' "$*"; }

command -v flatpak >/dev/null || die "flatpak is needed"
command -v python3 >/dev/null || die "python3 (3.11 or newer) is needed"
flatpak info org.flatpak.Builder >/dev/null 2>&1 ||
  die "org.flatpak.Builder is needed: flatpak install flathub org.flatpak.Builder"
builder() { flatpak run --command="$1" org.flatpak.Builder "${@:2}"; }

VERSION="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml")"
[[ -n "$VERSION" ]] || die "no [workspace.package] version in Cargo.toml"
TAG="${TAG:-v$VERSION}"
ARCH="${ARCH:-$(flatpak --default-arch)}"
case "$ARCH" in aarch64 | x86_64) ;; *) die "--arch must be aarch64 or x86_64 (Flathub's architectures)" ;; esac
if [[ "$ARCH" != "$(flatpak --default-arch)" ]] && ! flatpak --supported-arches | grep -qx "$ARCH"; then
  die "this machine can't run $ARCH builds: install qemu-user-static (with binfmt_misc) first"
fi

WORK="$ROOT/build/flatpak/$ARCH"
OUT="$ROOT/build/flatpak/out"
mkdir -p "$WORK" "$OUT"

# The metainfo's newest release is the one being made.
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
# Flathub's own options (flathub-build in org.flatpak.Builder): screenshots are downloaded and mirrored as Flathub
# serves them. Not --sandbox: the local manifest's sources are the checkout, outside packaging/flatpak.
# No rofiles-fuse: it can't mount from inside org.flatpak.Builder's sandbox.
started=$(date +%s)
set +e
builder flatpak-builder --force-clean --disable-rofiles-fuse --user --arch="$ARCH" --default-branch=stable \
  "${install_deps[@]}" --mirror-screenshots-url=https://dl.flathub.org/media --compose-url-policy=full \
  --state-dir="$WORK/state" --repo="$WORK/repo" "$WORK/builddir" "$MANIFEST" 2>&1 | tee "$WORK/build.log" |
  sed -u 's/\r/\n/g' | grep --line-buffered -Ev '^ *(Compiling |[0-9]+ +[0-9]|% Total|Dload)'
built="${PIPESTATUS[0]}"
set -e
[[ "$built" == 0 ]] || die "the build failed; see build/flatpak/$ARCH/build.log"
echo "Built in $(( ($(date +%s) - started) / 60 )) min $(( ($(date +%s) - started) % 60 )) s."

step "Bundle"
bundle="$OUT/AutoPaper-$VERSION-$ARCH.flatpak"
flatpak build-bundle --arch="$ARCH" --runtime-repo="$FLATHUB_REPO" "$WORK/repo" "$bundle" "$APP_ID" stable
(cd "$OUT" && sha256sum "$(basename "$bundle")" > "$(basename "$bundle").sha256")
echo "$bundle ($(du -h "$bundle" | cut -f 1))"
cat "$bundle.sha256"

step "Flathub submission files"
flathub_manifest=""
rm -rf "$OUT/flathub"
if [[ -z "$COMMIT" ]]; then
  COMMIT="$(git -C "$ROOT" rev-parse -q --verify "refs/tags/$TAG^{commit}" 2>/dev/null || true)"
fi
if [[ -z "$COMMIT" ]]; then
  echo "Skipped: no tag $TAG in this checkout (tag the release, or pass --tag and --commit)."
else
  [[ "$COMMIT" =~ ^[0-9a-f]{40}$ ]] || die "--commit must be a full 40-character commit hash"
  if [[ -n "$(git -C "$ROOT" status --porcelain 2>/dev/null)" ]] ||
    [[ "$(git -C "$ROOT" rev-parse -q HEAD 2>/dev/null)" != "$COMMIT" ]]; then
    echo "Note: this checkout isn't $TAG ($COMMIT) as committed, so the bundle may differ from what Flathub builds."
  fi
  mkdir -p "$OUT/flathub"
  flathub_manifest="$OUT/flathub/$APP_ID.yml"
  # The local manifest with its header and its app-sources block swapped for the release's git source.
  python3 - "$MANIFEST" "$flathub_manifest" "$REPO_URL" "$TAG" "$COMMIT" <<'PY'
import re, sys

source, target, url, tag, commit = sys.argv[1:]
text = open(source).read()
body = text[text.index("\nid: ") + 1:]
block = re.compile(r"^( *)# app-sources: begin\n.*?^ *# app-sources: end\n", re.M | re.S)
match = block.search(body)
if not match:
    sys.exit("the manifest has no app-sources block")
indent = match.group(1)
# YAML reads a hash of digits (with at most one "e") as a number, so only such a hash is quoted.
if re.fullmatch(r"[0-9]*(e[0-9]*)?", commit):
    commit = f"'{commit}'"
git = (f"{indent}- type: git\n{indent}  url: {url}\n{indent}  tag: {tag}\n{indent}  commit: {commit}\n")
header = "# AutoPaper for Linux on Flathub: GNOME runtime 51, built offline from the release tag.\n"
open(target, "w").write(header + body[:match.start()] + git + body[match.end():])
PY
  cp "$CARGO_SOURCES" "$OUT/flathub/cargo-sources.json"
  # Flathub builds x86_64 and aarch64 by default, which AutoPaper supports, so there's no flathub.json.
  ls -1 "$OUT/flathub"
fi

step "Flathub's linter"
lint_failed=()
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
[[ -n "$flathub_manifest" ]] && lint "Flathub manifest" manifest "$flathub_manifest"
lint "build directory" builddir "$WORK/builddir"
lint "repository" repo "$WORK/repo"

step "Done"
echo "Bundle:   $bundle"
echo "SHA-256:  $bundle.sha256"
[[ -n "$flathub_manifest" ]] && echo "Flathub:  $OUT/flathub/ (manifest, cargo-sources.json)"
echo "Install:  flatpak install --user $bundle"
cat <<'NOTE'
Flathub's rules (docs.flathub.org/docs/for-app-authors/requirements, "Generative AI policy"): the manifest submitted to
Flathub must not contain AI-generated or AI-assisted content, the submission pull request must be opened by a person,
and AI-generated code or packaging in the app must be disclosed.
NOTE
if [[ ${#lint_failed[@]} -gt 0 ]]; then
  printf -v failed '%s, ' "${lint_failed[@]}"
  echo "The linter reported errors for: ${failed%, } (docs.flathub.org/linter explains each)."
  [[ "$ALLOW_LINT_ERRORS" == 1 ]] || exit 1
fi
