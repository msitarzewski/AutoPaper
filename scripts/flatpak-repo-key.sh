#!/usr/bin/env bash
# Makes the signing key of AutoPaper's own Flatpak repository (once), and writes its public half to
# packaging/flatpak/autopaper-repo.gpg, which is committed (publish-flatpak.sh puts it in the .flatpakref, the
# .flatpakrepo and every download, so installations verify updates with it). Run on the Mac; needs Docker.
#
#   scripts/flatpak-repo-key.sh        make the key if there's none yet, then (re)write the public key file
#   scripts/flatpak-repo-key.sh --new  retire the current key (kept beside it, renamed) and make a new one. Every
#                                      installation must then add the repository again to trust the new key.
#
# The key lives only in its own GnuPG home, $AUTOPAPER_FLATPAK_GNUPGHOME (default ~/.config/autopaper/flatpak-gnupg,
# mode 700), made with the GnuPG in the publishing container (packaging/flatpak/publish.Dockerfile), the same one that
# signs with it. RSA 4096, signing only, no expiry (an expired repository key would stop every installation's updates).
# The secret key has a passphrase: a random one kept in the login Keychain (service autopaper-flatpak-signing), made so
# that every use asks you (macOS shows an Allow prompt when publish-flatpak.sh needs it). Anything that merely reads the
# key folder gets a key it can't use. This script never prints or exports the secret key or the passphrase: only the
# public key and its fingerprint.
#
# Back the whole folder up somewhere safe and private (an encrypted backup, a password manager's file attachment):
# without it no new version can be published to existing installations. They'd have to remove the repository and add
# it again with a new key.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GNUPG_DIR="${AUTOPAPER_FLATPAK_GNUPGHOME:-$HOME/.config/autopaper/flatpak-gnupg}"
PUBLIC="${AUTOPAPER_FLATPAK_PUBLIC:-$ROOT/packaging/flatpak/autopaper-repo.gpg}"
KEY_SERVICE="${AUTOPAPER_FLATPAK_KEY_SERVICE:-autopaper-flatpak-signing}"
NEW=0; [[ "${1:-}" == "--new" ]] && NEW=1
IMAGE="autopaper-flatpak-publish"
USER_ID="AutoPaper Flatpak repository (https://msitarzewski.com/app-updates/autopaper/)"

die() { echo "flatpak-repo-key: $*" >&2; exit 1; }
command -v docker >/dev/null || die "Docker is needed"
docker build -q -t "$IMAGE" -f "$ROOT/packaging/flatpak/publish.Dockerfile" "$ROOT/packaging/flatpak" >/dev/null

if [[ "$NEW" == 1 && -d "$GNUPG_DIR" ]] && ls "$GNUPG_DIR" >/dev/null 2>&1; then
  retired="$GNUPG_DIR.retired-$(date +%Y%m%d%H%M%S)"
  mv "$GNUPG_DIR" "$retired"
  echo "The old key folder is kept at $retired (keep it: it has the old key's revocation certificate)."
  security delete-generic-password -s "$KEY_SERVICE" -a repository-key >/dev/null 2>&1 || true
fi
mkdir -p "$GNUPG_DIR"
chmod 700 "$GNUPG_DIR"

# The passphrase: made once, kept in the login Keychain with no application trusted, so each use asks.
if ! security find-generic-password -s "$KEY_SERVICE" -a repository-key >/dev/null 2>&1; then
  if ls "$GNUPG_DIR/private-keys-v1.d"/*.key >/dev/null 2>&1; then
    die "$GNUPG_DIR holds a key but the Keychain has no passphrase for it (service $KEY_SERVICE): use --new to make a protected one"
  fi
  # (AUTOPAPER_FLATPAK_KEY_NO_PROMPT=1 is for trying the scripts with a throwaway key: the item then needs no click.)
  TRUST=(-T ""); [[ "${AUTOPAPER_FLATPAK_KEY_NO_PROMPT:-0}" == 1 ]] && TRUST=()
  security add-generic-password -s "$KEY_SERVICE" -a repository-key -l "AutoPaper Flatpak repository signing key" \
    ${TRUST[@]+"${TRUST[@]}"} -w "$(openssl rand -base64 36)" >/dev/null
fi
export FLATPAK_KEY_PASSPHRASE
FLATPAK_KEY_PASSPHRASE="$(security find-generic-password -s "$KEY_SERVICE" -a repository-key -w)"

# GnuPG works on a copy in the container's memory (its agent's sockets can't live on a folder shared from the Mac);
# a new key is copied back, sockets left out.
docker run --rm -i -v "$GNUPG_DIR:/gnupg" -e "USER_ID=$USER_ID" -e FLATPAK_KEY_PASSPHRASE "$IMAGE" bash -euo pipefail -s <<'IN' >&2
mkdir -m 700 -p "$GNUPGHOME"
cp -a /gnupg/. "$GNUPGHOME/" 2>/dev/null || true
rm -f "$GNUPGHOME"/S.*
chmod -R go-rwx "$GNUPGHOME"
if gpg --batch --list-secret-keys --with-colons 2>/dev/null | grep -q '^sec'; then
  echo "The repository key is already there."
else
  echo "Making the repository key (RSA 4096, signing only, no expiry, with a passphrase)…"
  gpg --batch --quiet --pinentry-mode loopback --passphrase "$FLATPAK_KEY_PASSPHRASE" \
    --quick-gen-key "$USER_ID" rsa4096 sign never
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
