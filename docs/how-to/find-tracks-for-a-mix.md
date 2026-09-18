# Find tracks for a mix

[The library](../library.md) is everything Dermixen has learned about your tracks, kept in one file in your data folder. It answers two kinds of question: which tracks meet a set of conditions, and which track a line of text names.

## List the tracks that meet conditions

```sh
dermixen library query --bpm 138-142 --compatible-with 8A --year 1995-1997
```

Every condition you give must hold. A range is `LOW-HIGH` with both ends included, or a single number for exactly that value. The conditions:

| Option | Matches tracks whose |
| --- | --- |
| `--under DIR` | file lies in that folder or one below it |
| `--bpm RANGE` | tempo lies in the range |
| `--year RANGE` | year lies in the range. A track with no year doesn't match |
| `--length RANGE` | length lies in the range, written as minutes and seconds like `7:00-9:00` or as seconds like `420-540` |
| `--key CODE` | Camelot code is the one given |
| `--compatible-with CODE` | Camelot code is the same as the one given, shares its number, or shares its letter with a number one step away around the wheel |
| `--artist TEXT` | artist contains the text, in any case |
| `--title TEXT` | title contains the text, in any case |
| `--no-approximate-years` | year is one a source states rather than an estimate. A track with no year and no estimate mark still matches this on its own, and with a `--year` range the command leaves out a track with no year either way |
| `--min-grid-confidence CONFIDENCE` | grid confidence is at least the value given, a number from 0 to 1 with the bound included |
| `--min-anchor-confidence CONFIDENCE` | anchor confidence is at least the value given, a number from 0 to 1 with the bound included |

Add `--no-approximate-years` to the query above when the era matters and an estimated year won't do.

With no conditions the query lists every track. Results come in ascending order of path, so you'll see one album's tracks together.

Each line of the output gives the Camelot code (`--` when the key is unknown), the tempo, the artist and title, and the path. A `?` after the artist or title means the scan guessed them from the file name because the file has no tags, so don't trust them without a look. A `-` in the artist column means the record has no artist: the file's tags name none, or the file name gave none (a track on a compilation, usually).

## Find harmonically compatible tracks

```sh
dermixen library query --compatible-with 12A --bpm 140-144
```

Every key has one Camelot code: a number from 1 to 12 and a letter, `A` for minor and `B` for major. Two codes mix well when they're the same, when they share a number, or when they share a letter and their numbers are neighbors on the wheel. [The library](../library.md#the-camelot-wheel) lists every code with its key. In the window, selecting a track highlights the compatible rows of the library panel in green.

## Find the track a line of text names

```sh
dermixen library find "Koxbox - Point of No Return"
```

The command ranks the tracks that match the text, best first, and prints each score with the path. The first two of the five lines this search printed:

```text
1.00  /Users/dermixenuser/audio/goa/comp/VA - Sun Trip [LRR52231CD]/01 Koxbox - Point of No Return.mp3
0.36  /Users/dermixenuser/audio/goa/comp/VA - Lucid Flux [ANJUNACD002]/04 Nervasystem & Aether - Distorted Waves of OM (Slight Return).mp3
```

The match compares words without regard to case or punctuation and forgives one wrong, missing, or extra letter in a word of five letters or more, so a typo in the listing doesn't lose the track. A leading track number or timestamp on the line, like `03.` or `12:34`, is ignored, so a line pasted from a tracklisting works as it is. A line that names the artist and the title scores one. A line that names only the title scores high, but below a line that names both. Take a top score of at least 0.8 as a resolved line and a top score below 0.5 as a guess you'll want to check by hand. `--limit N` changes how many matches you get, from the default of five. A track is a match only when the text names at least one word of its artist or title, so text that names nothing in any track prints nothing at all, and the command still exits with code 0.

## Get the results as JSON

```sh
dermixen library query --bpm 140-144 --json
dermixen library find "Koxbox - Point of No Return" --limit 1 --json
```

`query --json` prints a list of track records, and `find --json` prints a list of objects that each contain a `score` and a `record`, which is empty when the text matched no track. A record contains everything the scan stored: the path, the hash, the length, the grid, the key, the anchors, the metadata with its source, the loudness, and the phrases. [Build a mix from a tracklisting](build-a-mix-from-a-tracklisting.md) shows the JSON output feeding `mix add`.
