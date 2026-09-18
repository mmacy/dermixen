#!/bin/sh
# Downloads the five starter tracks by Undefunktis into
# $HOME/Music/Undefunktis, the music folder's default, which the window
# scans into the first library it makes. The script makes the folder if
# it is not there, prints one line per file it downloads, and stops at
# the first download that fails or whose digest is wrong.
#
# curl downloads the assets from the release on GitHub. When the gh
# command is on the path, the script downloads with gh instead. Setting
# DERMIXEN_STARTER_TRACKS_URL downloads every asset with curl from that
# address instead, which is how this script's own test in
# scripts/tests/install-starter-tracks-test.sh runs it against a local
# server, with no network and no GitHub login.
#
# Every download is checked against the SHA-256 digest recorded in the
# digest_for function below before it is kept. A file already at a
# track's destination is checked the same way: a correct one is left
# alone and reported as already there, and a wrong one is reported and
# downloaded again. Each download goes to a temporary name inside the
# destination folder first, made with mktemp, and is moved into place
# only once its digest matches, so a failed, interrupted, or mismatched
# download leaves nothing at the name the window will read rather than
# a partial or wrong file. A leftover temporary file from an earlier,
# interrupted run is removed before the script downloads anything.
#
# A destination that is already a symbolic link is refused, so this
# script never writes through a link planted where a track belongs, and
# the music folder itself is refused the same way when it is a symbolic
# link.
#
# The tracks are licensed under CC BY-NC-SA 4.0.

set -eu

REPO="mmacy/dermixen"
TAG="starter-tracks-v1"
DEST="$HOME/Music/Undefunktis"

# The SHA-256 digest of each release asset, keyed by the asset's name on the
# release, which is the local file name with every space turned into a dot.
digest_for() {
    case "$1" in
        "Undefunktis.-.Communion.With.The.Kelp.People.mp3")
            printf '%s\n' "099ba388b9b92e799e65186da811be3293868039895caec9cc1d24b2198d0919"
            ;;
        "Undefunktis.-.Cosmic.Gravy.mp3")
            printf '%s\n' "96c92178a8ccc63cbc19b30583aece4ba9dc07400695027fc9fce5e956f660aa"
            ;;
        "Undefunktis.-.Mood.Elevator.mp3")
            printf '%s\n' "38b7ce3005dce8486a02310e4d7c6c4eb9c2495764819bb7e27b9d283c426a72"
            ;;
        "Undefunktis.-.Polterheist.mp3")
            printf '%s\n' "f833ce1b075bdbdd606620d276c457043fed481cdcf399781c01ea9cf57c5920"
            ;;
        "Undefunktis.-.Summer.Mushroom.Salad.Surprise.mp3")
            printf '%s\n' "80579cb2191ebb42c8854fc296bab1c19c1d241ce56730250511e3ee8fe44b9e"
            ;;
        *)
            printf ''
            ;;
    esac
}

# The SHA-256 digest of a file, read with whichever of the two common
# digest commands is on the path. macOS ships shasum, and shasum is
# preferred where both are present.
sha256_of() {
    if command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        sha256sum "$1" | awk '{print $1}'
    fi
}

download_with_gh() {
    gh release download "$TAG" --repo "$REPO" --pattern "$1" --output "$2" --clobber
}

download_with_curl() {
    if [ -n "${DERMIXEN_STARTER_TRACKS_URL:-}" ]; then
        url="$DERMIXEN_STARTER_TRACKS_URL/$1"
    else
        url="https://github.com/$REPO/releases/download/$TAG/$1"
    fi
    curl -fL -o "$2" "$url"
}

if [ -L "$DEST" ]; then
    echo "$DEST is a symbolic link, refusing to use it as the music folder" >&2
    exit 1
fi

mkdir -p "$DEST"

# A temporary file from a run that failed or was interrupted before its
# trap could remove it. It has no destination it belongs to, so it is
# safe to remove before this run downloads anything.
rm -f "$DEST"/.starter-track.*

for file in \
    "Undefunktis - Cosmic Gravy.mp3" \
    "Undefunktis - Polterheist.mp3" \
    "Undefunktis - Summer Mushroom Salad Surprise.mp3" \
    "Undefunktis - Communion With The Kelp People.mp3" \
    "Undefunktis - Mood Elevator.mp3"
do
    destination="$DEST/$file"

    if [ -L "$destination" ]; then
        echo "$destination is a symbolic link, refusing to write through it" >&2
        exit 1
    fi

    # GitHub replaces each space in an uploaded asset's file name with a
    # dot, so the asset on the release is not spelled the way the track
    # is named locally.
    asset=$(printf '%s' "$file" | tr ' ' '.')
    expected=$(digest_for "$asset")
    if [ -z "$expected" ]; then
        echo "no SHA-256 digest is recorded for $asset" >&2
        exit 1
    fi

    if [ -f "$destination" ]; then
        if [ "$(sha256_of "$destination")" = "$expected" ]; then
            echo "$file is already in $DEST, skipping"
            continue
        fi
        echo "$file is in $DEST with the wrong SHA-256 digest, downloading it again" >&2
        rm -f "$destination"
    fi

    echo "Downloading $file"
    tmp=$(mktemp "$DEST/.starter-track.XXXXXX")
    trap 'rm -f "$tmp"' EXIT INT TERM

    if [ -z "${DERMIXEN_STARTER_TRACKS_URL:-}" ] && command -v gh >/dev/null 2>&1; then
        download_with_gh "$asset" "$tmp"
    else
        download_with_curl "$asset" "$tmp"
    fi

    actual=$(sha256_of "$tmp")
    if [ "$actual" != "$expected" ]; then
        echo "$file downloaded with the wrong SHA-256 digest: expected $expected, got $actual" >&2
        exit 1
    fi

    mv "$tmp" "$destination"
    trap - EXIT INT TERM
done
