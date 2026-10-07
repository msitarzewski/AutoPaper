#!/bin/bash
# Copies the embedding model into the macOS app bundle (Xcode build phase "Bundle the embedding model",
# apps/macos/project.yml). Dev builds take it from the repository's models/ folder (scripts/fetch-model.sh);
# when it's missing the build carries on with a warning: the engine then runs memory in reduced mode and
# Settings → Memory says so.
#
# Usage: scripts/macos-bundle-model.sh <source folder> <destination folder in the bundle>
set -euo pipefail

SRC="${1:?source folder}"
DEST="${2:?destination folder}"
FILES=(config.json tokenizer.json model.safetensors)

missing=()
for file in "${FILES[@]}"; do
    [[ -f "$SRC/$file" ]] || missing+=("$file")
done
if ((${#missing[@]})); then
    echo "warning: the embedding model isn't in $SRC (missing: ${missing[*]}); run scripts/fetch-model.sh. AutoPaper will run memory in reduced mode."
    rm -rf "$DEST"
    exit 0
fi

mkdir -p "$DEST"
for file in "${FILES[@]}"; do
    # Copy only when changed (size or time), so incremental builds don't rewrite 133 MB.
    if [[ ! -f "$DEST/$file" || "$SRC/$file" -nt "$DEST/$file" || $(stat -f %z "$SRC/$file") != $(stat -f %z "$DEST/$file") ]]; then
        cp -c "$SRC/$file" "$DEST/$file" 2>/dev/null || cp "$SRC/$file" "$DEST/$file"
    fi
done
# Nothing else in the folder: a file left from an older model would be signed into the bundle.
for existing in "$DEST"/*; do
    name="$(basename "$existing")"
    [[ " ${FILES[*]} " == *" $name "* ]] || rm -rf "$existing"
done
