# Build a mix from a tracklisting

Given a tracklisting, one track per line, the `dermixen` command builds the mix in that order, resolving each line to a file in [the library](../library.md), which is everything Dermixen has learned about your tracks, kept in one file in your data folder. An agent driving the app does the same, which is the second workflow `DESIGN.md` names.

## Prerequisites

- A library that contains the tracks, from [Scan your music](scan-your-music.md).
- The tracklisting as a text file, one track per line, in the order the mix plays. Leading track numbers and timestamps are fine.

## Resolve each line

```
dermixen library find "03. Koxbox - Point of No Return" --limit 1 --json
```

The top match's `score` measures how well the line matches the track. A score of at least 0.8 is a resolved line. A score below 0.5 is a guess: report the line rather than build on it. Between the two, look at the path the match names before you trust it.

## Build the mix in order

```
dermixen mix new set.dmx
dermixen mix add set.dmx "/Users/dermixenuser/audio/goa/comp/VA - Analog Dreams [DATCD005]/2 Space Tribe - The Great Spirit (Original Mix) - mastered.mp3"
dermixen mix add set.dmx "/Users/dermixenuser/audio/goa/comp/VA - Sun Trip [LRR52231CD]/01 Koxbox - Point of No Return.mp3"
```

Each `mix add` appends the track, joins it to the one before it with the default transition, the blend, and prints the timeline. `mix add` refuses an add it can't place, with a message that says why, and leaves the document as it was, so the timeline it prints is always a mix that renders.

## Do it in one script

The script below reads a tracklisting, resolves every line, and builds the mix only when every line resolved. It stops at the first line it can't resolve and writes no mix, so a set with a hole in it is never built by mistake.

```python
import json
import subprocess
import sys

listing, mix = sys.argv[1], sys.argv[2]
lines = [line.strip() for line in open(listing) if line.strip()]
paths = []
for line in lines:
    out = subprocess.run(
        ["dermixen", "library", "find", line, "--limit", "1", "--json"],
        capture_output=True, text=True, check=True,
    ).stdout
    matches = json.loads(out)
    if not matches or matches[0]["score"] < 0.8:
        print(f"unresolved: {line}", file=sys.stderr)
        sys.exit(1)
    paths.append(matches[0]["record"]["path"])

subprocess.run(["dermixen", "mix", "new", mix], check=True)
for path in paths:
    subprocess.run(["dermixen", "mix", "add", mix, path], check=True)
```

```
python3 build_mix.py tracklist.txt set.dmx
```

The lines that don't resolve are the ones to fix by hand: a track the scan hasn't seen, a file with no tags whose name the parser read wrongly, or a title spelled differently in the listing than in the file.

## Check the result

```
dermixen mix show set.dmx
dermixen render set.dmx set.mp3
```

`mix show` prints the timeline with every track's tempo, gain, anchors, and start and end times. [Check a transition by ear](check-a-transition-by-ear.md) covers rendering or playing one transition at a time, and [Change a transition](change-a-transition.md) covers presets, lengths, and anchors when the default isn't right for a pair of tracks.
