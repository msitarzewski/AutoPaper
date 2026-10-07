#!/bin/sh
# Downloads the sentence-embedding model behind AutoPaper's memory (BAAI/bge-small-en-v1.5, MIT licence,
# 384-d) at a pinned Hugging Face revision, and verifies every file's SHA-256 before keeping it.
#
# Usage: scripts/fetch-model.sh [TARGET_DIR]
#   TARGET_DIR defaults to $AUTOPAPER_MODEL_DIR, else <repo>/models/bge-small-en-v1.5 (git-ignored).
#
# Idempotent: files already present with the right checksum are left alone; anything missing or wrong is
# downloaded again. A file only appears under its real name once it has been verified, so an interrupted
# run never leaves a half-written model behind. Needs: curl, and sha256sum or shasum.
#
# Pinned revision and checksums: docs/research/rust-linux.md ("Local sentence embeddings"). The
# model.safetensors hash is the LFS SHA-256 from the Hugging Face API (?blobs=true); config.json and
# tokenizer.json are plain git blobs there (SHA-1 only), so their SHA-256s were computed from downloads
# whose git blob ids matched the API's.

set -eu

REPO_ID="BAAI/bge-small-en-v1.5"
REVISION="5c38ec7c405ec4b44b94cc5a9bb96e735b38267a"
BASE_URL="https://huggingface.co/$REPO_ID/resolve/$REVISION"

# name  sha256  bytes
FILES="config.json 094f8e891b932f2000c92cfc663bac4c62069f5d8af5b5278c4306aef3084750 743
tokenizer.json d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66 711396
model.safetensors 3c9f31665447c8911517620762200d2245a2518d6e7208acc78cd9db317e21ad 133466304"

die() {
    printf 'fetch-model: %s\n' "$*" >&2
    exit 1
}

case "${1:-}" in
    -h | --help)
        sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'
        exit 0
        ;;
esac
[ "$#" -le 1 ] || die "expected at most one argument (the target directory); see --help"

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd) || die "can't locate the script's directory"
target=${1:-${AUTOPAPER_MODEL_DIR:-"$script_dir/../models/bge-small-en-v1.5"}}

command -v curl >/dev/null 2>&1 || die "curl is required"
if command -v sha256sum >/dev/null 2>&1; then
    sha256_of() { sha256sum -- "$1" | cut -d ' ' -f 1; }
elif command -v shasum >/dev/null 2>&1; then
    sha256_of() { shasum -a 256 -- "$1" | cut -d ' ' -f 1; }
else
    die "sha256sum or shasum is required"
fi

mkdir -p -- "$target" || die "can't create $target"
target=$(CDPATH='' cd -- "$target" && pwd) || die "can't enter $target"

partial=""
cleanup() {
    if [ -n "$partial" ]; then
        rm -f -- "$partial"
    fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# Progress bar only when someone is watching.
if [ -t 2 ]; then progress="--progress-bar"; else progress="--silent"; fi

# Fed by a here-document (not a pipe) so the loop runs in this shell: `partial` stays visible to the
# cleanup trap, and `die` ends the script.
while read -r name expected bytes; do
    dest="$target/$name"
    if [ -f "$dest" ]; then
        if [ "$(sha256_of "$dest")" = "$expected" ]; then
            printf 'ok        %s (already verified)\n' "$name"
            continue
        fi
        printf 'replacing %s (checksum mismatch)\n' "$name"
    fi

    partial="$dest.part"
    rm -f -- "$partial"
    printf 'fetching  %s (%s bytes)\n' "$name" "$bytes"
    curl --fail --location --max-redirs 5 --proto '=https' --proto-redir '=https' --tlsv1.2 \
        --connect-timeout 20 --retry 3 --retry-delay 2 $progress --show-error \
        --user-agent "AutoPaper fetch-model (https://github.com/msitarzewski/AutoPaper)" \
        --output "$partial" "$BASE_URL/$name" ||
        die "download failed: $BASE_URL/$name"

    actual=$(sha256_of "$partial")
    if [ "$actual" != "$expected" ]; then
        die "checksum mismatch for $name: expected $expected, got $actual (nothing was kept)"
    fi
    mv -f -- "$partial" "$dest" || die "can't move $name into $target"
    partial=""
    printf 'verified  %s\n' "$name"
done <<EOF
$FILES
EOF

printf 'model ready in %s\n' "$target"
