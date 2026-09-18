# Measure the analyzers on your own corrections

Every grid you fix and every anchor you move in the window is written out as an annotation, and the scoreboard measures every analyzer against a folder of annotations. Your corrections are the ground truth against which an analyzer change is measured on your own music.

## Prerequisites

- Corrections in `dermixen/corrections` in your data folder (`~/Library/Application Support` on macOS, `~/.local/share` on Linux), written by the window's grid editor and anchor drags.
- A local copy of the audio the corrections refer to.
- Python 3, for `tools/anchor_truth.py`.

## Gather the corrections beside their audio

Each correction is a file named after the track's audio file, then a hyphen and eight digits of the track's content hash, then `.anchors`. It contains the track's path, the grid's tempo and beat zero, and each anchor in seconds, marked `ear` as placed by listening. The scoreboard reads an annotation only from beside the audio file it belongs to, named the same way, and `tools/anchor_truth.py link` arranges that:

```sh
python3 tools/anchor_truth.py link ~/Library/Application\ Support/dermixen/corrections ~/audio --out ~/dermixen-truth
```

The command copies the annotations, finds the audio under the second path by the `file` line each annotation contains, and writes each annotation's tempo as a `.bpm` file beside it so that the beat scoreboard runs on the same folder.

## Run the scoreboard

```sh
dermixen scoreboard ~/dermixen-truth
```

The command runs every built-in analyzer over the folder and prints five tables: the beat scoreboard, the key scoreboard, and, since the folder contains anchor annotations, the anchor, grid, and phrase scoreboards. Each row is one analyzer. [Ground truth for the scoreboard](../ground-truth.md) defines each column: under "Metrics" for the beat and key tables, and under "Anchor metrics", "Grid metrics on the same tracks", and "Phrase metrics on the same tracks" for the other three. `--json` prints the same figures with every analyzer's score on every track.

## Keep the corrections you trust

The committed ground truth is `tests/ground-truth/anchors/`, without the audio. Copy the corrections you trust into that folder, and every analyzer is measured against them from then on:

```sh
dermixen scoreboard tests/ground-truth/anchors
```

The audio has to be beside the annotations for that command to run, which `tools/anchor_truth.py link` arranges as above. The scoreboard reports each label source on a row of its own, and only the `ear` rows measure what the app is built for.

## Measure against a public dataset

```sh
dermixen scoreboard --giantsteps path/to/giantsteps-tempo-dataset
```

The GiantSteps tempo and key datasets are annotated Beatport previews of electronic dance music, and `--giantsteps` reads a checkout's layout. Those datasets contain no anchor annotations, so the command prints the beat and key tables only. [How analysis is trusted](../explanation/how-analysis-is-trusted.md) says what the numbers are for.
