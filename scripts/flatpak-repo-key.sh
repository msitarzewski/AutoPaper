#!/usr/bin/env bash
# Makes the signing key of AutoPaper's own Flatpak repository (once), and writes its public half to
# packaging/flatpak/autopaper-repo.gpg, which is committed (publish-flatpak.sh puts it in the .flatpakref, the
# .flatpakrepo and every download, so installations verify updates with it). Run on the Mac; needs Docker.
#
#   scripts/flatpak-repo-key.sh      make the key if there's none yet, then (re)write the public key file
#
# The key lives only in its own GnuPG home, $AUTOPAPER_FLATPAK_GNUPGHOME (default ~/.config/autopaper/flatpak-gnupg,
# mode 700), made with the GnuPG in the publishing container (packaging/flatpak/publish.Dockerfile), the same one that
# signs with it. RSA 4096, signing only, no expiry (an expired repository key would stop every installation's updates),
# no passphrase (publish-flatpak.sh signs unattended; the folder's permissions and FileVault protect it). This script
# never prints or exports the secret key: only the public key and its fingerprint.
#
# Back the whole folder up somewhere safe and private (an encrypted backup, a password manager's file attachment):
# without it no new version can be published to existing installations. They'd have to remove the repository and add
# it again with a new key.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GNUPG_DIR="${AUTOPAPER_FLATPAK_GNUPGHOME:-$HOME/.config/autopaper/flatpak-gnupg}"
PUBLIC="$ROOT/packaging/flatpak/autopaper-repo.gpg"
IMAGE="autopaper-flatpak-publish"
USER_ID="AutoPaper Flatpak repository (https://msitarzewski.com/app-updates/autopaper/)"

die() { echo "flatpak-repo-key: $*" >&2; exit 1; }
command -v docker >/dev/null || die "Docker is needed"
docker build -q -t "$IMAGE" -f "$ROOT/packaging/flatpak/publish.Dockerfile" "$ROOT/packaging/flatpak" >/dev/null

mkdir -p "$GNUPG_DIR"
chmod 700 "$GNUPG_DIR"

# GnuPG works on a copy in the container's memory (its agent's sockets can't live on a folder shared from the Mac);
# a new key is copied back, sockets left out.
docker run --rm -i -v "$GNUPG_DIR:/gnupg" -e "USER_ID=$USER_ID" "$IMAGE" bash -euo pipefail -s <<'IN' >&2
mkdir -m 700 -p "$GNUPGHOME"
cp -a /gnupg/. "$GNUPGHOME/" 2>/dev/null || true
rm -f "$GNUPGHOME"/S.*
chmod -R go-rwx "$GNUPGHOME"
if gpg --batch --list-secret-keys --with-colons 2>/dev/null | grep -q '^sec'; then
  echo "The repository key is already there."
else
  echo "Making the repository key (RSA 4096, signing only, no expiry)…"
  gpg --batch --quiet --passphrase '' --quick-gen-key "$USER_ID" rsa4096 sign never
  find "$GNUPGHOME" -mindepth 1 -maxdepth 1 ! -name 'S.*' -exec cp -a {} /gnupg/ \;
fi
IN

# Only the public key and its fingerprint leave the container.
fingerprint="$(docker run --rm -v "$GNUPG_DIR:/gnupg:ro" "$IMAGE" bash -c \
  'mkdir -m 700 -p "$GNUPGHOME"; cp -a /gnupg/. "$GNUPGHOME/"; rm -f "$GNUPGHOME"/S.*; chmod -R go-rwx "$GNUPGHOME";
   gpg --batch --list-secret-keys --with-colons 2>/dev/null | awk -F: "/^fpr/ { print \$10; exit }"')"
[[ "$fingerprint" =~ ^[0-9A-F]{40}$ ]] || die "no secret key found in $GNUPG_DIR"
docker run --rm -v "$GNUPG_DIR:/gnupg:ro" -e "FPR=$fingerprint" "$IMAGE" bash -c \
  'mkdir -m 700 -p "$GNUPGHOME"; cp -a /gnupg/. "$GNUPGHOME/"; rm -f "$GNUPGHOME"/S.*; chmod -R go-rwx "$GNUPGHOME";
   gpg --batch --export "$FPR"' > "$PUBLIC.part"
[[ -s "$PUBLIC.part" ]] || die "exporting the public key failed"
mv "$PUBLIC.part" "$PUBLIC"
chmod -R go-rwx "$GNUPG_DIR"

echo "Key fingerprint: $fingerprint"
echo "Public key:      ${PUBLIC#"$ROOT/"} (commit it)"
echo "Secret key:      $GNUPG_DIR (mode 700; back the folder up somewhere safe and private)"
