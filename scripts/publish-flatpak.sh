#!/usr/bin/env bash
# Publishes AutoPaper for Linux to its own Flatpak repository, https://msitarzewski.com/app-updates/autopaper/flatpak
# (on pipx, served by Caddy), and makes the GitHub release's downloads. Run on the Mac at release time, once
# .github/workflows/flatpak.yml has built the tag's bundles. Needs Docker, gh (signed in), ssh access to pipx, and the
# repository key from scripts/flatpak-repo-key.sh.
#
#   scripts/publish-flatpak.sh --tag v0.1.0      CI's bundles for that tag
#   scripts/publish-flatpak.sh --run ID          a particular CI run's bundles (one started by hand, say)
#   scripts/publish-flatpak.sh --bundles DIR     bundles already on this Mac (AutoPaper-<version>-<arch>.flatpak, .sha256)
#
#   --test           the test repository instead: flatpak-test/, AutoPaper-test.flatpakref, autopaper-test.flatpakrepo
#   --no-upload      make the repository and the downloads here only (build/flatpak-repo/, build/flatpak/release/)
#   --base-url URL   where the repository is served (default https://msitarzewski.com/app-updates/autopaper); with
#                    --no-upload, for trying a local copy over plain HTTP
#   --remove-test    delete the test repository and its two files from pipx and here, and stop
#
# What it does:
#   1. Bundles: CI's (gh run download), each checked against its .sha256.
#   2. The local copy of the repository, build/flatpak-repo/<flatpak|flatpak-test> (git-ignored), is refreshed from
#      pipx first, so its history (earlier commits, static deltas from them) carries on from what's published.
#   3. In Docker (packaging/flatpak/publish.Dockerfile, as flatpak and ostree don't run on macOS): each bundle is
#      committed onto the published ref, signed (its parent is the version before); build-update-repo signs the
#      summary and regenerates the AppStream data GNOME
#      Software reads, static deltas from the previous commits, and prunes commits past the last three. Then the
#      GitHub release's downloads are made from the signed commits, each carrying the repository's URL, Flathub's
#      runtime repository and the public key, so installing one adds the repository and keeps AutoPaper updated.
#   4. Upload to pipx with rsync over ssh, in passes: objects and deltas, then the indexes (config, refs, summaries),
#      so a client never sees a summary naming objects that aren't there yet; then what pruning removed; then
#      AutoPaper.flatpakref (one-click install) and autopaper.flatpakrepo (adds the repository).
#
# Signing uses $AUTOPAPER_FLATPAK_GNUPGHOME (default ~/.config/autopaper/flatpak-gnupg), mounted read-only into the
# container for this run only and copied into its memory (GnuPG needs somewhere to write); the secret key is never
# printed or exported. Uploads follow ~/Clean/pipx/DEPLOYING.md: as michael, only under
# /srv/www/msitarzewski.com/app-updates/autopaper/, no sudo. Its folders are michael:www-static 2750 (setgid).
# macOS's rsync (openrsync) ignores --chmod, so no permissions are sent: pipx's rsync runs under umask 027
# (--rsync-path), which makes new files 0640 and new folders 0750 plus the setgid bit they inherit (2750), all group
# www-static, which Caddy reads.
set -euo pipefail

APP_ID="io.github.msitarzewski.AutoPaper"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PUBLIC_KEY="$ROOT/packaging/flatpak/autopaper-repo.gpg"
GNUPG_DIR="${AUTOPAPER_FLATPAK_GNUPGHOME:-$HOME/.config/autopaper/flatpak-gnupg}"
PIPX_HOST="${AUTOPAPER_PIPX_HOST:-pipx}"
REMOTE_BASE="/srv/www/msitarzewski.com/app-updates/autopaper"
BASE_URL="https://msitarzewski.com/app-updates/autopaper"
FLATHUB_REPO="https://dl.flathub.org/repo/flathub.flatpakrepo"
HOMEPAGE="https://msitarzewski.github.io/AutoPaper/"
ICON="https://msitarzewski.github.io/AutoPaper/icon-256.png"
IMAGE="autopaper-flatpak-publish"
WORKFLOW="flatpak.yml"

TAG=""
RUN_ID=""
BUNDLES=""
TEST=0
UPLOAD=1
REMOVE_TEST=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --tag) TAG="$2"; shift 2 ;;
    --run) RUN_ID="$2"; shift 2 ;;
    --bundles) BUNDLES="$2"; shift 2 ;;
    --test) TEST=1; shift ;;
    --no-upload) UPLOAD=0; shift ;;
    --base-url) BASE_URL="${2%/}"; shift 2 ;;
    --remove-test) REMOVE_TEST=1; shift ;;
    *) sed -n '2,20p' "$0"; exit 2 ;;
  esac
done

die() { echo "publish-flatpak: $*" >&2; exit 1; }
step() { printf '\n== %s\n' "$*"; }

if [[ "$TEST" == 1 ]]; then
  NAME="flatpak-test"; REF_FILE="AutoPaper-test.flatpakref"; REPO_FILE="autopaper-test.flatpakrepo"
  REMOTE_NAME="autopaper-test"; TITLE="AutoPaper (test)"
else
  NAME="flatpak"; REF_FILE="AutoPaper.flatpakref"; REPO_FILE="autopaper.flatpakrepo"
  REMOTE_NAME="autopaper"; TITLE="AutoPaper"
fi
SITE="$ROOT/build/flatpak-repo"          # the local copy of app-updates/autopaper/ (its Flatpak parts)
MIRROR="$SITE/$NAME"
REPO_URL="$BASE_URL/$NAME"

# rsync to and from pipx; the far side's umask sets the permissions (see the header). OSTree's working files (tmp/,
# state/, .lock) stay on each side, never copied or deleted.
pipx_rsync() { rsync -rlt --rsync-path='umask 027 && rsync' --exclude=/tmp/ --exclude=/state/ --exclude=/.lock "$@"; }

if [[ "$REMOVE_TEST" == 1 ]]; then
  step "Removing the test repository"
  # Only these three paths, all inside AutoPaper's own folder on pipx.
  ssh "$PIPX_HOST" "rm -rf '$REMOTE_BASE/flatpak-test' '$REMOTE_BASE/AutoPaper-test.flatpakref' '$REMOTE_BASE/autopaper-test.flatpakrepo'"
  rm -rf "$SITE/flatpak-test" "$SITE/AutoPaper-test.flatpakref" "$SITE/autopaper-test.flatpakrepo" \
    "$ROOT/build/flatpak/release-test"
  echo "Removed flatpak-test/, AutoPaper-test.flatpakref and autopaper-test.flatpakrepo (if they were there)."
  exit 0
fi

[[ -n "$TAG$RUN_ID$BUNDLES" ]] || die "say which bundles: --tag, --run or --bundles (see the header)"
command -v docker >/dev/null || die "Docker is needed"
[[ -s "$PUBLIC_KEY" ]] || die "no public key at ${PUBLIC_KEY#"$ROOT/"}: run scripts/flatpak-repo-key.sh"
[[ -d "$GNUPG_DIR/private-keys-v1.d" ]] || die "no signing key in $GNUPG_DIR: run scripts/flatpak-repo-key.sh"

step "Bundles"
INBOX="$ROOT/build/flatpak/inbox"
rm -rf "$INBOX"
mkdir -p "$INBOX"
if [[ -n "$BUNDLES" ]]; then
  cp "$BUNDLES"/AutoPaper-*.flatpak "$BUNDLES"/AutoPaper-*.flatpak.sha256 "$INBOX/" 2>/dev/null ||
    die "no AutoPaper-*.flatpak with its .sha256 in $BUNDLES"
else
  command -v gh >/dev/null || die "gh is needed (brew install gh, then gh auth login)"
  if [[ -z "$RUN_ID" ]]; then
    RUN_ID="$(gh run list --workflow "$WORKFLOW" --branch "$TAG" --event push --status success --limit 1 \
      --json databaseId --jq '.[0].databaseId // empty')"
    [[ -n "$RUN_ID" ]] || die "no successful $WORKFLOW run for $TAG yet (gh run list --workflow $WORKFLOW)"
  fi
  echo "CI run $RUN_ID"
  gh run download "$RUN_ID" --dir "$INBOX/dl" --pattern 'AutoPaper-flatpak-*'
  find "$INBOX/dl" -name 'AutoPaper-*.flatpak*' -exec mv {} "$INBOX/" \;
  rm -rf "$INBOX/dl"
fi
VERSION=""
ARCHES=()
for bundle in "$INBOX"/AutoPaper-*.flatpak; do
  file="$(basename "$bundle")"
  [[ "$file" =~ ^AutoPaper-(.+)-(x86_64|aarch64)\.flatpak$ ]] || die "unexpected bundle name: $file"
  [[ -z "$VERSION" || "$VERSION" == "${BASH_REMATCH[1]}" ]] || die "bundles of two versions: $VERSION and ${BASH_REMATCH[1]}"
  VERSION="${BASH_REMATCH[1]}"
  ARCHES+=("${BASH_REMATCH[2]}")
  [[ -f "$bundle.sha256" ]] || die "$file has no .sha256"
  (cd "$INBOX" && shasum -a 256 -c "$file.sha256" >/dev/null) || die "$file doesn't match its .sha256"
  echo "$file ($(du -h "$bundle" | cut -f 1)), checksum matches"
done
[[ ${#ARCHES[@]} -gt 0 ]] || die "no bundles"
[[ -z "$TAG" || "$TAG" == "v$VERSION" ]] || die "the bundles are $VERSION, not $TAG"
[[ ${#ARCHES[@]} == 2 ]] || echo "Note: only ${ARCHES[*]} (a release has x86_64 and aarch64)."

step "The repository's copy here ($NAME)"
mkdir -p "$SITE"
if [[ "$UPLOAD" == 1 ]]; then
  if ssh "$PIPX_HOST" "test -f '$REMOTE_BASE/$NAME/config'"; then
    echo "Refreshing from pipx"
    mkdir -p "$MIRROR"
    pipx_rsync --delete "$PIPX_HOST:$REMOTE_BASE/$NAME/" "$MIRROR/"
  else
    echo "Nothing published yet: starting a new repository"
    rm -rf "$MIRROR"
  fi
else
  echo "Not uploading: using the copy here as it is (${MIRROR#"$ROOT/"})"
fi
mkdir -p "$MIRROR"

step "Import, sign, update the repository, make the downloads (Docker)"
# (The suffix is set apart: `$([[ … ]] && echo -test)` exits 1 when TEST isn't 1, and set -e stops the script there.)
RELEASE_SUFFIX=""; if [[ "$TEST" == 1 ]]; then RELEASE_SUFFIX=-test; fi
RELEASE="$ROOT/build/flatpak/release$RELEASE_SUFFIX"
rm -rf "$RELEASE"
mkdir -p "$RELEASE"
docker build -q -t "$IMAGE" -f "$ROOT/packaging/flatpak/publish.Dockerfile" "$ROOT/packaging/flatpak" >/dev/null
docker run --rm -i --tmpfs /run:rw,mode=755 \
  -v "$MIRROR:/work/repo" -v "$INBOX:/work/in:ro" -v "$RELEASE:/work/out" \
  -v "$GNUPG_DIR:/gnupg:ro" -v "$PUBLIC_KEY:/work/public.gpg:ro" \
  -e "APP_ID=$APP_ID" -e "VERSION=$VERSION" -e "ARCHES=${ARCHES[*]}" -e "REPO_URL=$REPO_URL" \
  -e "FLATHUB_REPO=$FLATHUB_REPO" -e "TITLE=$TITLE" -e "HOMEPAGE=$HOMEPAGE" -e "ICON=$ICON" \
  "$IMAGE" bash -euo pipefail -s <<'IN'
# The key, in the container's memory for this run only.
mkdir -m 700 -p "$GNUPGHOME"
cp -a /gnupg/. "$GNUPGHOME/"
rm -f "$GNUPGHOME"/S.*
chmod -R go-rwx "$GNUPGHOME"
KEY="$(gpg --batch --show-keys --with-colons public.gpg 2>/dev/null | awk -F: '/^fpr/ { print $10; exit }')"
gpg --batch --list-secret-keys "$KEY" >/dev/null 2>&1 || { echo "the signing key doesn't match public.gpg" >&2; exit 1; }
echo "Signing with $KEY"
sign=(--gpg-sign="$KEY" --gpg-homedir="$GNUPGHOME")

if [[ ! -f repo/config ]]; then
  ostree init --mode=archive-z2 --repo=repo
fi
# Each bundle into a scratch repository first, then committed onto the published ref: the new commit's parent is the
# version before it, so clients get a static delta from it and `flatpak update --commit` can go back.
for bundle in in/*.flatpak; do
  rm -rf /run/import
  ostree init --mode=archive-z2 --repo=/run/import
  flatpak build-import-bundle --no-update-summary /run/import "$bundle" >/dev/null
  ref="$(ostree --repo=/run/import refs | grep "^app/$APP_ID/")"
  flatpak build-commit-from "${sign[@]}" --src-repo=/run/import --subject="AutoPaper $VERSION" --no-update-summary \
    repo "$ref" >/dev/null
  echo "$ref: $(ostree --repo=repo rev-parse "$ref") (AutoPaper $VERSION)"
done
rm -rf /run/import
# The summary (signed), AppStream for GNOME Software, static deltas from earlier commits, three commits kept per ref.
flatpak build-update-repo "${sign[@]}" --gpg-import=public.gpg --title="$TITLE" \
  --comment="AutoPaper's own Flatpak repository" --homepage="$HOMEPAGE" --icon="$ICON" --default-branch=stable \
  --generate-static-deltas --prune --prune-depth=3 repo
echo "Refs:"
ostree --repo=repo refs | sed 's/^/  /'

# The GitHub release's downloads, from the signed commits: installing one adds this repository, verified with the key.
for arch in $ARCHES; do
  file="AutoPaper-$VERSION-$arch.flatpak"
  flatpak build-bundle --arch="$arch" --repo-url="$REPO_URL" --runtime-repo="$FLATHUB_REPO" \
    --gpg-keys=public.gpg repo "out/$file" "$APP_ID" stable
  (cd out && sha256sum "$file" > "$file.sha256")
done
IN

step "Install files ($REF_FILE, $REPO_FILE)"
gpg_key="$(base64 < "$PUBLIC_KEY" | tr -d '\n')"
cat > "$SITE/$REF_FILE" <<EOF
[Flatpak Ref]
Title=$TITLE
Name=$APP_ID
Branch=stable
Url=$REPO_URL
SuggestRemoteName=$REMOTE_NAME
IsRuntime=false
Homepage=$HOMEPAGE
Comment=New wallpapers from a few keywords
Description=Give AutoPaper a few keywords, and it keeps making new wallpapers from them.
Icon=$ICON
RuntimeRepo=$FLATHUB_REPO
GPGKey=$gpg_key
EOF
cat > "$SITE/$REPO_FILE" <<EOF
[Flatpak Repo]
Title=$TITLE
Url=$REPO_URL
Homepage=$HOMEPAGE
Comment=AutoPaper's own Flatpak repository
Description=AutoPaper for Linux, signed by its developer. The GNOME runtime it needs comes from Flathub.
Icon=$ICON
GPGKey=$gpg_key
EOF
echo "${SITE#"$ROOT/"}/$REF_FILE"
echo "${SITE#"$ROOT/"}/$REPO_FILE"

if [[ "$UPLOAD" == 1 ]]; then
  step "Upload to pipx ($REMOTE_BASE/$NAME)"
  remote="$PIPX_HOST:$REMOTE_BASE/$NAME/"
  indexes=(--include=/config --include=/refs/ --include='/refs/**' --include='/summary' --include='/summary.*'
    --include=/summaries/ --include='/summaries/**')
  echo "1/4 objects and deltas"
  pipx_rsync --exclude=/config --exclude=/refs/ --exclude='/summary' --exclude='/summary.*' --exclude=/summaries/ \
    "$MIRROR/" "$remote"
  echo "2/4 indexes"
  pipx_rsync --delete-after "${indexes[@]}" --exclude='*' "$MIRROR/" "$remote"
  echo "3/4 what pruning removed"
  pipx_rsync --delete-after "$MIRROR/" "$remote"
  echo "4/4 install files"
  pipx_rsync "$SITE/$REF_FILE" "$SITE/$REPO_FILE" "$PIPX_HOST:$REMOTE_BASE/"
  ssh "$PIPX_HOST" "stat -c '%a %G %n' '$REMOTE_BASE/$NAME' '$REMOTE_BASE/$NAME/config' '$REMOTE_BASE/$REF_FILE'" |
    sed 's/^/  /'
fi

step "Done"
echo "Repository:  $REPO_URL"
echo "Install:     $BASE_URL/$REF_FILE"
echo "Downloads for the GitHub release (signed; installing one keeps AutoPaper updated from the repository):"
for arch in "${ARCHES[@]}"; do
  echo "  ${RELEASE#"$ROOT/"}/AutoPaper-$VERSION-$arch.flatpak  ($(cut -d ' ' -f 1 "$RELEASE/AutoPaper-$VERSION-$arch.flatpak.sha256"))"
done
if [[ "$TEST" == 0 && "$UPLOAD" == 1 ]]; then
  echo "Attach them: gh release upload v$VERSION ${RELEASE#"$ROOT/"}/AutoPaper-$VERSION-*.flatpak*"
fi
