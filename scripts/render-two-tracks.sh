#!/bin/sh
# Renders a beat-matched crossfade of two tracks with the dermixen command.
#
# Usage:
#   scripts/render-two-tracks.sh A.wav A_BPM A_OUTRO_BEAT B.wav B_BPM B_INTRO_BEAT out.wav
#
# A plays first and hands over to B. A_OUTRO_BEAT is the beat of A where the
# transition starts, and B_INTRO_BEAT is the beat of B that lands on it; the
# transition then runs for eight bars. Both tempos are taken as given, so the
# script works without any analyzer built in. The mix document is written
# next to the output as out.wav.dmx so it can be inspected or edited.
set -eu
if [ "$#" -ne 7 ]; then
    sed -n '2,11p' "$0" >&2
    exit 2
fi
cargo build --release -p dermixen-cli
BIN=target/release/dermixen
MIX="$7.dmx"
rm -f "$MIX"
"$BIN" mix new "$MIX"
"$BIN" mix add "$MIX" "$1" --bpm "$2" --outro "$3"
"$BIN" mix add "$MIX" "$4" --bpm "$5" --intro "$6"
"$BIN" render "$MIX" "$7"
