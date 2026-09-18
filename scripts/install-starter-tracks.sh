#!/bin/sh
# Downloads the five starter tracks by Undefunktis into
# $HOME/Music/Undefunktis, the music folder's default, which the window
# scans into the first library it makes. The script makes the folder if
# it is not there, skips a file that is already there, prints one line
# per file it downloads, and stops at the first download that fails.
# curl downloads the assets from the release on GitHub. When the gh
# command is on the path, the script downloads with gh instead.
# The tracks are licensed under CC BY-NC-SA 4.0.

set -e

REPO="mmacy/dermixen"
TAG="starter-tracks-v1"
DEST="$HOME/Music/Undefunktis"

mkdir -p "$DEST"

# GitHub replaces each space in an uploaded asset's file name with a
# dot, so the asset on the release is not spelled the way the track
# is named locally. Both download methods fetch the dotted name and
# then rename the file to the name this script uses everywhere else.

download_with_gh() {
    file="$1"
    asset=$(printf '%s' "$file" | tr ' ' '.')
    gh release download "$TAG" --repo "$REPO" --pattern "$asset" --dir "$DEST"
    mv "$DEST/$asset" "$DEST/$file"
}

download_with_curl() {
    file="$1"
    asset=$(printf '%s' "$file" | tr ' ' '.')
    url="https://github.com/$REPO/releases/download/$TAG/$asset"
    curl -fL -o "$DEST/$file" "$url"
}

for file in \
    "Undefunktis - Cosmic Gravy.mp3" \
    "Undefunktis - Polterheist.mp3" \
    "Undefunktis - Summer Mushroom Salad Surprise.mp3" \
    "Undefunktis - Communion With The Kelp People.mp3" \
    "Undefunktis - Mood Elevator.mp3"
do
    if [ -f "$DEST/$file" ]; then
        echo "$file is already in $DEST, skipping"
        continue
    fi

    echo "Downloading $file"
    if command -v gh >/dev/null 2>&1; then
        download_with_gh "$file"
    else
        download_with_curl "$file"
    fi
done
