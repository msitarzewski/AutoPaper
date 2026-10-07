#!/usr/bin/env bash
# One-time (idempotent) setup on pipx for AutoPaper's update hosting at https://msitarzewski.com/app-updates/autopaper/:
#   flatpak/                    the Flatpak repository (OSTree, GPG-signed), filled by scripts/publish-flatpak.sh
#   windows/                    the MSIX bundles, filled by scripts/publish-windows.sh
#   AutoPaper.appinstaller      Windows' App Installer feed
#   AutoPaper.flatpakref        one-click Linux install (adds the repository, so Flatpak keeps it updated)
#
# Follows ~/Clean/pipx/DEPLOYING.md: run as michael; sudo only creates the four folders (michael:www-static, setgid,
# so files uploaded later inherit www-static and Caddy can read them, as provision-static.sh does). Touches only /srv/www/msitarzewski.com/app-updates/ and
# /etc/caddy/sites/msitarzewski.com.caddy (backed up first), reloads with caddy-apply (validate + hot reload), and
# puts the backup back if validation fails. Safe to run twice.
#
#   scp scripts/pipx-app-updates.sh pipx: && ssh -t pipx bash pipx-app-updates.sh
set -euo pipefail

SITE=/srv/www/msitarzewski.com
DEST=$SITE/app-updates/autopaper
CADDY=/etc/caddy/sites/msitarzewski.com.caddy
MARK_BEGIN="	# >>> AutoPaper app-updates (scripts/pipx-app-updates.sh in github.com/msitarzewski/AutoPaper)"
MARK_END="	# <<< AutoPaper app-updates"

[ "$(id -un)" != root ] || { echo "run as michael, not root (DEPLOYING.md rule 1)"; exit 1; }
[ -d "$SITE" ] || { echo "no $SITE"; exit 1; }
[ -w "$CADDY" ] || { echo "can't write $CADDY (michael must be in the caddy group: caddy-editable-setup.sh)"; exit 1; }
command -v caddy-apply >/dev/null || { echo "caddy-apply missing (caddy-editable-setup.sh)"; exit 1; }

echo "==> [1/4] folders under $SITE/app-updates (sudo: michael:www-static, 2750 — setgid, so uploads stay www-static)"
umask 027
for dir in "$SITE/app-updates" "$DEST" "$DEST/flatpak" "$DEST/windows"; do
    sudo install -d -o michael -g www-static -m 2750 "$dir"
done
if [ ! -f "$DEST/index.html" ]; then
    cat > "$DEST/index.html" <<'HTML'
<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>AutoPaper updates</title></head>
<body><p>AutoPaper's updates for Windows and Linux are served from here. To install AutoPaper, visit
<a href="https://msitarzewski.github.io/AutoPaper/">msitarzewski.github.io/AutoPaper</a>.</p></body></html>
HTML
    chmod 0640 "$DEST/index.html"
fi

echo "==> [2/4] Caddy: fresh feeds and indexes, the right file types (inside msitarzewski.com's block only)"
if grep -qF "$MARK_BEGIN" "$CADDY"; then
    echo "    already there"
else
    backup="$CADDY.bak.$(date +%s)"
    cp -p "$CADDY" "$backup"
    snippet=$(cat <<EOF
$MARK_BEGIN
	# Feeds and the Flatpak repository's indexes must never be stale; its objects and the packages are immutable.
	@ap_fresh path /app-updates/autopaper/*.appinstaller /app-updates/autopaper/*.flatpakref /app-updates/autopaper/*.flatpakrepo /app-updates/autopaper/flatpak/config /app-updates/autopaper/flatpak/summary /app-updates/autopaper/flatpak/summary.* /app-updates/autopaper/flatpak/summaries/* /app-updates/autopaper/flatpak/refs/*
	header @ap_fresh Cache-Control "no-cache"
	@ap_flatpakref path /app-updates/*.flatpakref /app-updates/*/*.flatpakref
	header @ap_flatpakref Content-Type "application/vnd.flatpak.ref"
	@ap_flatpakrepo path /app-updates/*.flatpakrepo /app-updates/*/*.flatpakrepo
	header @ap_flatpakrepo Content-Type "application/vnd.flatpak.repo"
	@ap_appinstaller path /app-updates/*.appinstaller /app-updates/*/*.appinstaller
	header @ap_appinstaller Content-Type "application/appinstaller"
	@ap_msixbundle path /app-updates/*/windows/*.msixbundle
	header @ap_msixbundle Content-Type "application/msixbundle"
$MARK_END
EOF
)
    # Inserted just before the block's file_server line.
    grep -qE '^[[:space:]]*file_server[[:space:]]*$' "$CADDY" || { echo "no file_server line in $CADDY"; exit 1; }
    awk -v snippet="$snippet" '!done && /^[[:space:]]*file_server[[:space:]]*$/ { print snippet; done = 1 } { print }' \
        "$backup" > "$CADDY"
    echo "==> [3/4] validate + reload (caddy-apply); the backup goes back if it fails"
    if ! caddy-apply; then
        cp -p "$backup" "$CADDY"
        caddy-apply || true
        echo "FAILED: $CADDY restored from $backup"
        exit 1
    fi
    echo "    backup: $backup"
fi

echo "==> [4/4] smoke test through pipx's Caddy (no edge, no public DNS)"
probe="$DEST/probe-$$.flatpakref"
printf '[Flatpak Ref]\nName=probe\n' > "$probe"
code=$(curl -s -o /dev/null -w '%{http_code}' -H 'Host: msitarzewski.com' http://127.0.0.1/app-updates/autopaper/)
type=$(curl -s -o /dev/null -w '%{content_type}' -H 'Host: msitarzewski.com' "http://127.0.0.1/app-updates/autopaper/$(basename "$probe")")
cache=$(curl -sI -H 'Host: msitarzewski.com' "http://127.0.0.1/app-updates/autopaper/$(basename "$probe")" | tr -d '\r' | sed -n 's/^[Cc]ache-[Cc]ontrol: //p')
rm -f "$probe"
echo "    GET /app-updates/autopaper/      → HTTP $code"
echo "    GET …/probe.flatpakref           → $type, Cache-Control: ${cache:-none}"
[ "$code" = 200 ] && [ "$type" = application/vnd.flatpak.ref ] && [ "$cache" = no-cache ] || { echo "smoke test failed"; exit 1; }
echo "Done. Public check:  curl -sI https://msitarzewski.com/app-updates/autopaper/"
