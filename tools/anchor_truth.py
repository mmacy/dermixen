#!/usr/bin/env python3
"""Builds anchor ground truth from MixMeister's files and gathers audio beside it.

The anchor scoreboard reads a directory of `name.anchors` annotation files with
the audio file `name.ext` beside each one; `docs/ground-truth.md` describes the
format. This program writes those annotation files from the two kinds of file
MixMeister leaves behind, and links audio in from a local audio collection,
which is not in the repository, so the scoreboard can run.

    python3 tools/anchor_truth.py from-mmp tests/fixtures/mmp/*.mmp --out tests/ground-truth/anchors
    python3 tools/anchor_truth.py from-mxm ~/.cache/dermixen/plots/*.mxm --out tests/ground-truth/anchors
    python3 tools/anchor_truth.py phrases tests/ground-truth/anchors
    python3 tools/anchor_truth.py link tests/ground-truth/anchors ~/audio --out ~/.cache/dermixen/anchors

`from-mmp` reads playlists with `mmp_dump.py` and writes one annotation per
track: the grid fitted to the playlist's section markers, the intro anchor
from the cue node, which is where the track enters the mix, and the outro
anchor from the measure marker, which is where its fade out begins. Both are labeled
`mixmeister`. A track used more than once in a playlist, a sample shorter than
a minute, a track whose grid does not fit its own section markers, and a track
that has no cue or no measure marker are all left out, and the program says
why. An annotation file that already exists is left alone unless `--force` is
given, so labels corrected by ear survive a rebuild.

`from-mxm` reads plot files and writes the grid, fitted to the plot file's
section markers the same way, the effective beginning and ending, and the
start of the intro and outro ranges, all labeled `plot`.

`phrases` writes a `phrase` line into every annotation for each intro and
outro label that sits on a bar line of the grid outside the first bar, with
the label's time and source, because each of those is a bar a transition was
aligned to. It says which labels it left out and why.

`link` makes a directory the scoreboard can run on: it copies every
annotation, writes the annotation's tempo as a `name.bpm` file so the beat
scoreboard can run on the same tracks, and links the audio file named by the
annotation's `file` line, looked up under the given audio root, first by that
path and then by file name anywhere below the root.
"""

import argparse
import json
import os
import struct
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

SAMPLE_RATE = 44100

BEATS_PER_BAR = 4

# The lane identifiers mmp_dump.py reports for the nodes this program reads.
LANE_CUE = 0x00000001
LANE_MEASURE_MARKER = 0x00000002

# The Windows roots the playlists' track paths start with, mapped onto
# folders below the audio root the `link` command is given.
ROOTS = {
    "D:\\audio\\goa\\": "goa/",
    "E:\\audio\\psytrance\\": "psytrance/",
    "\\\\media\\data\\audio\\psytrance\\": "psytrance/",
    "C:\\audio\\vinyl_rips\\": "psytrance/vinyl_rips/",
}


def seconds(microseconds: int) -> float:
    """Converts a playlist position to seconds, reading wrapped negatives as negative."""
    if microseconds >= 2**63:
        microseconds -= 2**64
    return microseconds / 1e6


def collection_path(windows_path: str) -> str | None:
    """The path of a playlist track inside the audio collection, or None for a sample."""
    for root, folder in ROOTS.items():
        if windows_path.startswith(root):
            return folder + windows_path[len(root) :].replace("\\", "/")
    return None


@dataclass
class Annotation:
    """One track's ground truth, ready to be written."""

    name: str
    file: str | None
    bpm: float
    first_beat: float
    labels: dict[str, tuple[float, str]]
    comment: str

    def text(self) -> str:
        lines = [f"# {self.comment}"]
        if self.file:
            lines.append(f"file {self.file}")
        lines.append(f"bpm {self.bpm:.4f}")
        lines.append(f"first_beat {self.first_beat:.6f}")
        for key in ("begins", "ends", "intro", "outro"):
            if key in self.labels:
                at, source = self.labels[key]
                lines.append(f"{key} {at:.6f} {source}")
        return "\n".join(lines) + "\n"


def write_annotation(annotation: Annotation, out: Path, force: bool) -> None:
    path = out / f"{annotation.name}.anchors"
    if path.exists() and not force:
        print(f"kept {path.name}: it already exists")
        return
    path.write_text(annotation.text(), encoding="utf-8")
    print(f"wrote {path.name}")


def dump_playlist(path: Path) -> dict:
    """Reads a playlist with mmp_dump.py."""
    dumper = Path(__file__).with_name("mmp_dump.py")
    output = subprocess.run(
        [sys.executable, str(dumper), "--json", str(path)],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return json.loads(output)


def to_source(position: float, segments: list[tuple[float, float, float]]) -> float:
    """Maps a position on a track's edited timeline back to the source file.

    Each segment plays part of the source file at some place on the timeline,
    so a position inside a segment is the segment's source position plus the
    distance into the segment. A track that was not edited has one segment
    covering the whole file, which maps every position to itself.
    """
    for start, end, source in segments:
        if start - 1e-6 <= position <= end + 1e-6:
            return source + (position - start)
    return position


def fit_grid(bpm: float, markers: list[float]) -> tuple[float, float, int] | None:
    """Fits a tempo and a beat zero to the section markers of one track.

    MixMeister places its section markers on whole beats of its own grid, so
    the markers pin the grid down more precisely than the tempo the playlist
    stores to three decimals: over a thousand beats a rounding of the tempo
    walks the stored grid a fifth of a beat away from the markers. Each marker
    is assigned the whole beat nearest to it at the stored tempo, counted from
    the first marker, and a straight line through those points gives the
    tempo and the beat zero. The result is the tempo, the beat zero in
    seconds, and how many markers sit within a tenth of a beat of the line,
    or None when the markers do not describe one grid, which happens when a
    track holds a section edit that shifts its beats, or when there are fewer
    than four markers.
    """
    if len(markers) < 4:
        return None
    period = 60.0 / bpm
    beats = [round((m - markers[0]) / period) for m in markers]
    n = len(markers)
    mean_beat = sum(beats) / n
    mean_time = sum(markers) / n
    covariance = sum((b - mean_beat) * (t - mean_time) for b, t in zip(beats, markers))
    variance = sum((b - mean_beat) ** 2 for b in beats)
    if variance == 0:
        return None
    slope = covariance / variance
    intercept = mean_time - slope * mean_beat
    residuals = [abs((t - intercept) / slope - b) for b, t in zip(beats, markers)]
    fitting = sum(1 for r in residuals if r <= 0.1)
    if fitting < 0.9 * n:
        return None
    return 60.0 / slope, intercept, fitting


def from_mmp(paths: list[Path], out: Path, force: bool) -> None:
    out.mkdir(parents=True, exist_ok=True)
    for path in paths:
        playlist = dump_playlist(path)
        tracks = playlist["tracks"]
        uses: dict[str, int] = {}
        for track in tracks:
            uses[track["path"]] = uses.get(track["path"], 0) + 1
        for track in tracks:
            name = Path(track["path"].replace("\\", "/")).stem
            file = collection_path(track["path"])
            if file is None:
                print(f"skipped {name}: it is a sample, not a track")
                continue
            if uses[track["path"]] > 1:
                print(f"skipped {name}: {path.name} uses it {uses[track['path']]} times")
                continue
            spans = track["spans"]
            if not spans or seconds(spans[0]["end_us"]) < 60:
                print(f"skipped {name}: shorter than a minute")
                continue
            markers = [seconds(m["position_us"]) for m in track["beatgrid"]]
            if not markers:
                print(f"skipped {name}: no beat grid")
                continue
            fitted = fit_grid(track["original_bpm"], markers)
            if fitted is None:
                print(f"skipped {name}: its section markers do not sit on one grid")
                continue
            bpm, first_beat, _ = fitted
            nodes = track["automation"]
            cue = [seconds(n["position_us"]) for n in nodes if n["lane"] == LANE_CUE]
            measure = [seconds(n["position_us"]) for n in nodes if n["lane"] == LANE_MEASURE_MARKER]
            if not cue or not measure:
                print(f"skipped {name}: no cue or no measure marker")
                continue
            segments = [
                (seconds(s["start_us"]), seconds(s["end_us"]), seconds(s["source_us"]))
                for s in spans[1:]
            ]
            intro = to_source(cue[0], segments)
            outro = to_source(measure[0], segments)
            if outro <= intro:
                print(f"skipped {name}: the measure marker is not after the cue")
                continue
            annotation = Annotation(
                name=name,
                file=file,
                bpm=bpm,
                first_beat=first_beat,
                labels={"intro": (intro, "mixmeister"), "outro": (outro, "mixmeister")},
                comment="The grid fitted to the section markers, the cue as the intro anchor, and the measure marker as the outro anchor.",
            )
            write_annotation(annotation, out, force)


def read_plot(path: Path) -> dict:
    """Reads the analysis results from a MixMeister plot file."""
    raw = path.read_bytes()
    if raw[:4] != b"RIFF" or raw[8:12] != b"MXM6":
        raise ValueError(f"{path} is not a MixMeister plot file")
    chunks = {}
    position = 12
    while position + 8 <= len(raw):
        identifier = raw[position : position + 4]
        size = struct.unpack_from("<I", raw, position + 4)[0]
        chunks[identifier] = raw[position + 8 : position + 8 + size]
        position += 8 + size + (size & 1)
    profile = chunks[b"prof"]
    tempo = struct.unpack_from("<f", profile, 8)[0]
    begins, ends = struct.unpack_from("<Q", profile, 24)[0], struct.unpack_from("<Q", profile, 32)[0]
    intro = struct.unpack_from("<Q", profile, 56)[0]
    outro = struct.unpack_from("<Q", profile, 76)[0]
    # After the ranges come two counted lists of 64-bit sample positions:
    # first a short list whose meaning is not established, then the section
    # markers, the same bar-line markers the playlists hold for each track.
    count = struct.unpack_from("<I", profile, 92)[0]
    markers_at = 96 + 8 * count
    marker_count = struct.unpack_from("<I", profile, markers_at)[0]
    markers = [
        struct.unpack_from("<Q", profile, markers_at + 4 + 8 * index)[0]
        for index in range(marker_count)
    ]
    return {
        "bpm": tempo,
        "begins": begins,
        "ends": ends,
        "intro": intro,
        "outro": outro,
        "markers": markers,
    }


def from_mxm(paths: list[Path], out: Path, force: bool, collection: str | None) -> None:
    out.mkdir(parents=True, exist_ok=True)
    for path in paths:
        plot = read_plot(path)
        rate = SAMPLE_RATE
        # The section markers sit on bar lines of MixMeister's grid, so the
        # grid is fitted through them exactly as it is for a playlist track,
        # which puts beat zero on a bar line and pins the tempo down more
        # precisely than the tempo the plot file stores as a single float.
        # A plot file whose markers do not describe one grid, which is what
        # a beat that shifts partway through the track looks like, gets the
        # tempo it stores and the first marker moved back to the first bar
        # line at or after the start of the file, with a warning.
        intro = plot["intro"] / rate
        markers = [marker / rate for marker in plot["markers"]]
        fitted = fit_grid(plot["bpm"], markers)
        if fitted is None:
            print(f"warning {path.stem}: its section markers do not sit on one grid, so the grid is the stored tempo through the first marker")
            bpm = plot["bpm"]
            bar = BEATS_PER_BAR * 60.0 / bpm
            first_beat = markers[0] - bar * (markers[0] // bar)
        else:
            bpm, first_beat, _ = fitted
        file = f"{collection}/{path.stem}.mp3" if collection else None
        annotation = Annotation(
            name=path.stem,
            file=file,
            bpm=bpm,
            first_beat=first_beat,
            labels={
                "begins": (plot["begins"] / rate, "plot"),
                "ends": (plot["ends"] / rate, "plot"),
                "intro": (intro, "plot"),
                "outro": (plot["outro"] / rate, "plot"),
            },
            comment="MixMeister's own analysis of the track.",
        )
        write_annotation(annotation, out, force)


PHRASE_COMMENT = "# Phrase labels: the bars the anchor labels above sit on, as bars a transition was aligned to."
"""The comment line written above the phrase labels an annotation derives from its anchors."""

FIRST_BARS_LEFT_OUT = 1
"""A label within this many bars of beat zero says nothing about the phrase structure."""


def phrases(truth: Path) -> None:
    """Writes phrase labels into every annotation from its intro and outro labels.

    An intro or outro label is a bar a transition was aligned to, so each
    becomes a `phrase` line with the same time and source. A label that does not sit on a bar line of the annotation's grid
    is left out, because the phrase scoreboard measures bars, and so is a
    label in the first bar of the track, because the first bar starts the
    count by definition and says nothing about where the phrases fall. The
    phrase lines this program wrote before are replaced; phrase and section
    lines written by hand are kept.
    """
    tracks = 0
    labels_written = 0
    left_out: list[str] = []
    for annotation in sorted(truth.glob("*.anchors")):
        lines = annotation.read_text(encoding="utf-8").splitlines()
        bpm = first_beat = None
        anchors: list[tuple[str, float, str]] = []
        for line in lines:
            if not line.strip() or line.startswith("#"):
                continue
            key, rest = line.split(None, 1)
            if key == "bpm":
                bpm = float(rest)
            elif key == "first_beat":
                first_beat = float(rest)
            elif key in ("intro", "outro"):
                at, source = rest.split()
                anchors.append((key, float(at), source))
        if bpm is None or first_beat is None:
            print(f"skipped {annotation.name}: it has no grid")
            continue
        derived = {(f"{at:.6f}", source) for _, at, source in anchors}
        # The phrase block written before is replaced where it stands, so
        # lines written by hand after it stay after it.
        kept: list[str] = []
        block_at: int | None = None
        for line in lines:
            if line == PHRASE_COMMENT or (
                line.startswith("phrase ") and tuple(line.split()[1:3]) in derived
            ):
                if block_at is None:
                    block_at = len(kept)
                continue
            kept.append(line)
        while kept and not kept[-1].strip():
            kept.pop()
        if block_at is None:
            block_at = len(kept)
        period = 60.0 / bpm
        new: list[str] = []
        for key, at, source in anchors:
            beat = (at - first_beat) / period
            bars = beat / BEATS_PER_BAR
            if abs(bars - round(bars)) * BEATS_PER_BAR > 0.1:
                left_out.append(
                    f"{annotation.stem}: the {key} {source} label at {at:.3f} s sits {beat:.2f} beats from beat zero, not on a bar line"
                )
                continue
            if abs(round(bars)) < FIRST_BARS_LEFT_OUT:
                left_out.append(
                    f"{annotation.stem}: the {key} {source} label at {at:.3f} s is in the first bar, which starts the count by definition"
                )
                continue
            line = f"phrase {at:.6f} {source}"
            if line not in new:
                new.append(line)
        if new:
            kept[block_at:block_at] = [PHRASE_COMMENT, *new]
            tracks += 1
            labels_written += len(new)
        text = "\n".join(kept) + "\n"
        if text != annotation.read_text(encoding="utf-8"):
            annotation.write_text(text, encoding="utf-8")
    for reason in left_out:
        print(f"left out {reason}")
    print(f"wrote {labels_written} phrase labels on {tracks} tracks; left out {len(left_out)} labels")


def link(truth: Path, audio_root: Path, out: Path) -> None:
    out.mkdir(parents=True, exist_ok=True)
    by_name: dict[str, Path] = {}
    for annotation in sorted(truth.glob("*.anchors")):
        file = None
        bpm = None
        for line in annotation.read_text(encoding="utf-8").splitlines():
            if line.startswith("file "):
                file = line[5:].strip()
            if line.startswith("bpm "):
                bpm = line[4:].strip()
        if file is None:
            print(f"skipped {annotation.name}: it names no audio file")
            continue
        candidate = audio_root / file
        if not candidate.is_file():
            if not by_name:
                for found in audio_root.rglob("*"):
                    if found.is_file():
                        by_name.setdefault(found.name, found)
            candidate = by_name.get(Path(file).name)
        if candidate is None or not candidate.is_file():
            print(f"missing {annotation.name}: {file} is not under {audio_root}")
            continue
        target = out / f"{annotation.stem}{candidate.suffix.lower()}"
        if target.is_symlink() or target.exists():
            target.unlink()
        target.symlink_to(candidate.resolve())
        (out / annotation.name).write_text(annotation.read_text(encoding="utf-8"), encoding="utf-8")
        if bpm is not None:
            (out / f"{annotation.stem}.bpm").write_text(f"{bpm}\n", encoding="utf-8")
        print(f"linked {annotation.stem}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)

    mmp = commands.add_parser("from-mmp", help="write annotations from MixMeister playlists")
    mmp.add_argument("playlists", nargs="+", type=Path)
    mmp.add_argument("--out", required=True, type=Path)
    mmp.add_argument("--force", action="store_true", help="overwrite annotations that already exist")

    mxm = commands.add_parser("from-mxm", help="write annotations from MixMeister plot files")
    mxm.add_argument("plots", nargs="+", type=Path)
    mxm.add_argument("--out", required=True, type=Path)
    mxm.add_argument("--force", action="store_true", help="overwrite annotations that already exist")
    mxm.add_argument("--collection", help="the folder below the audio root that holds the tracks")

    phraser = commands.add_parser("phrases", help="write phrase labels from the anchor labels")
    phraser.add_argument("truth", type=Path)

    linker = commands.add_parser("link", help="gather audio beside copies of the annotations")
    linker.add_argument("truth", type=Path)
    linker.add_argument("audio_root", type=Path)
    linker.add_argument("--out", required=True, type=Path)

    args = parser.parse_args()
    if args.command == "from-mmp":
        from_mmp(args.playlists, args.out, args.force)
    elif args.command == "from-mxm":
        from_mxm(args.plots, args.out, args.force, args.collection)
    elif args.command == "phrases":
        phrases(args.truth)
    else:
        link(args.truth, args.audio_root, args.out)


if __name__ == "__main__":
    main()
