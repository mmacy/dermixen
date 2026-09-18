#!/bin/sh
# Acceptance test for scripts/install-starter-tracks.sh. It serves five files
# that are not the starter tracks from a local web server, runs the installer
# against that server with a scratch home folder, and checks that the
# installer refuses every one of them. It then checks that the installer does
# not write through a symbolic link planted in the music folder.
#
# The installer downloads from the address in DERMIXEN_STARTER_TRACKS_URL when
# that variable is set, and always with curl in that case, so that this test
# needs no network and no GitHub login.
#
# Run it from anywhere: sh scripts/tests/install-starter-tracks-test.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
installer="$here/../install-starter-tracks.sh"
scratch=$(mktemp -d)
server=""
cleanup() {
    [ -n "$server" ] && kill "$server" 2>/dev/null || true
    rm -rf "$scratch"
}
trap cleanup EXIT

failures=0
fail() {
    echo "FAIL: $1"
    failures=$((failures + 1))
}

mkdir -p "$scratch/served" "$scratch/home"
for asset in \
    Undefunktis.-.Cosmic.Gravy.mp3 \
    Undefunktis.-.Polterheist.mp3 \
    Undefunktis.-.Summer.Mushroom.Salad.Surprise.mp3 \
    Undefunktis.-.Communion.With.The.Kelp.People.mp3 \
    Undefunktis.-.Mood.Elevator.mp3
do
    printf 'this is not the track\n' > "$scratch/served/$asset"
done

port=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
python3 -m http.server "$port" --bind 127.0.0.1 --directory "$scratch/served" >/dev/null 2>&1 &
server=$!
tries=0
until curl -fs -o /dev/null "http://127.0.0.1:$port/"; do
    tries=$((tries + 1))
    [ "$tries" -lt 50 ] || { echo "the test server did not start"; exit 2; }
    sleep 0.1
done

# 1. A download whose digest is wrong is refused, and nothing is left behind.
status=0
HOME="$scratch/home" DERMIXEN_STARTER_TRACKS_URL="http://127.0.0.1:$port" \
    sh "$installer" > "$scratch/first.log" 2>&1 || status=$?
[ "$status" -ne 0 ] || fail "the installer accepted a file with the wrong digest (exit 0)"
grep -qi 'sha-\{0,1\}256\|digest\|checksum' "$scratch/first.log" \
    || fail "the installer did not say the digest was wrong: $(cat "$scratch/first.log")"
left=$(find "$scratch/home/Music/Undefunktis" -mindepth 1 2>/dev/null | wc -l | tr -d ' ')
[ "$left" -eq 0 ] || fail "the installer left $left file(s) in the music folder: $(ls -A "$scratch/home/Music/Undefunktis")"

# 2. A symbolic link planted where a track goes is not written through.
rm -rf "${scratch:?}/home"
mkdir -p "$scratch/home/Music/Undefunktis"
ln -s "$scratch/victim.txt" "$scratch/home/Music/Undefunktis/Undefunktis - Cosmic Gravy.mp3"
status=0
HOME="$scratch/home" DERMIXEN_STARTER_TRACKS_URL="http://127.0.0.1:$port" \
    sh "$installer" > "$scratch/second.log" 2>&1 || status=$?
[ "$status" -ne 0 ] || fail "the installer exited 0 with a link planted in the music folder"
[ ! -e "$scratch/victim.txt" ] || fail "the installer wrote through the planted link"

# 3. An unset variable is an error, not an empty string.
grep -q '^set -eu' "$installer" || fail "the installer does not run with set -eu"

if [ "$failures" -eq 0 ]; then
    echo "ok"
else
    exit 1
fi
