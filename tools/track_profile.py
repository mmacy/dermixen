#!/usr/bin/env python3
"""Print the level of a track, window by window, so a transition can be placed by number as well as by ear.

The `beats` command walks a stretch of a track on its own beat grid and prints
one line per window of `--per` beats: the beat the window starts on, the level
of the signal in decibels below full scale, how hard the low band pulses on the
beat, and a bar of `#` for scanning by eye. The pulse is what tells a kick from
a pad: the low band of a window with a kick in it swings by twelve decibels or
more between the hit and the gap after it, and the low band of a pad or a
breakdown swings by less than ten. A bass line playing between the kicks
lowers the number, so a track's own sections are compared with each other
rather than with another track's.

The `sections` command prints one line per beat at which a section changes, so
the beats an anchor can go on are found in one run instead of by profiling at
eight beats a line, then two, then one. A line names the beat the new state
begins on, what changed, and that beat modulo 32, which is where the beat sits
on a 32-beat phrase grid:

```
beat   128  kick leaves   (0 mod 32)
beat   160  kick returns  (0 mod 32)
beat   224  level +6.0 dB  (0 mod 32)
```

A beat has a kick when the low band's pitch sweeps down through that beat, the
way a kick drum's pitch does and a bass note's does not, judged on the bar of
beats around it. The kick leaves at the first beat of a run of eight beats or
more that have no kick, and returns at the first beat of a run of eight beats
or more that have one, so a single kick inside a break is a fill rather than
the return. The level steps when the level of the beats after a beat differs
from the level of the beats before it by two decibels or more. A level step
within two beats of a kick change is left out, because losing the kick is what
moved the level.

The `seconds` command prints the level on a fixed clock instead of a beat
grid, which is how a rendered clip is measured: a mix has no single grid once
two tracks overlap, and what matters there is whether the level dips across the
handover.

The tempo and the first beat of a track come from the `grid` object that
`dermixen library query --json` and `dermixen mix show --json` print, as `bpm`
and `first_beat_sample`.

Examples:
    ```
    python3 tools/track_profile.py beats track.mp3 --bpm 148.55 --first-beat-sample 68616 --from 960 --to 1216 --per 4
    python3 tools/track_profile.py sections track.mp3 --bpm 148.55 --first-beat-sample 68616 --from 0 --to 1024
    python3 tools/track_profile.py seconds clip.mp3 --window 2
    ```

It needs the built `dermixen` command, which decodes the audio, and the
`numpy` package.
"""

import argparse
import sys

import numpy as np

# The kick is found the same way `grid_check.py` finds it, by the pitch sweep
# that starts a kick drum, so both programs agree on what a kick is, and both
# read audio the same way. Python searches the directory of the file it is
# running before anywhere else, and the two programs and `decoding.py` sit in
# that same directory, so the plain names resolve.
import decoding
import grid_check

# The sample rate the whole project is fixed at, matching `SAMPLE_RATE` in
# `crates/core/src/units.rs`. Audio is decoded to it whatever rate the file has.
SAMPLE_RATE = 44100

# The top of the low band, in hertz. A kick drum's weight sits below 100 Hz
# and a bass line's fundamentals mostly below 150 Hz, while pads, leads, and
# hats sit above, so the band under this corner pulses with the kick and the
# bass and little else.
DEFAULT_CORNER_HZ = 150.0

# How finely a beat is divided when the pulse is measured. Sixteen slots on a
# beat at 150 BPM are 25 milliseconds each, which is shorter than a kick's
# burst, so the hit and the gap after it land in different slots, and long
# enough that the quietest slot is not a single dip in the waveform.
SLOTS_PER_BEAT = 16

# Added to every mean square before the logarithm, so digital silence prints
# as -180 dB instead of raising an error.
SILENCE = 1e-18

# How long a run or a comparison in the `sections` command is, in beats. A
# level step compares this many beats each side of a beat, and the kick has to
# leave or return for this many beats together before the change is reported.
# Eight beats is two bars, which is long enough that one loud beat or one beat
# with no kick under it is not a section, and short enough to place a change on
# the beat the change happens on.
RUN_BEATS = 8

# How many beats the kick test averages into one profile. The profile and the
# sweep test that reads it are the ones `grid_check.py` uses, and averaging the
# beats around the beat being judged steadies the test on a track whose bass
# note is as loud as its kick. A bar is the longest average that still places a
# section change on the beat the change happens on, so `sections` averages a
# bar where `grid_check.py` averages four bars for a check every eighth beat.
KICK_BEATS = 4

# How far the level has to step for `sections` to report it, in decibels.
STEP_DB = 2.0

# How near a kick change a level step has to be for `sections` to leave the
# level step out, in beats.
COVERED_BEATS = 2

# How long a phrase is, in beats. Goa and psytrance sections change on the
# phrase, so a change at 0 modulo this number is on a phrase line and a change
# anywhere else is not.
PHRASE_BEATS = 32


def decibels(mean_square: np.ndarray) -> np.ndarray:
    """Turn mean squares into decibels relative to full scale."""
    return 10.0 * np.log10(mean_square + SILENCE)


def frame(samples: np.ndarray, window: int) -> np.ndarray:
    """Cut the samples into whole windows, one per row, dropping the remainder."""
    count = len(samples) // window
    return samples[: count * window].reshape(count, window)


def level(frames: np.ndarray) -> np.ndarray:
    """The level of each window, in decibels relative to full scale."""
    return decibels(np.mean(frames * frames, axis=1))


def pulse(frames: np.ndarray, per: int, corner: float) -> np.ndarray:
    """How hard the low band of each window pulses on the beat, in decibels.

    The band below `corner` is taken from each window by a Fourier transform
    with the bins above the corner set to zero. Its energy is folded onto one
    beat, averaging the `per` beats of the window, and the beat is divided
    into [`SLOTS_PER_BEAT`][tools.track_profile.SLOTS_PER_BEAT] slots. The
    pulse is the ratio of the loudest slot to the quietest. Folding does not
    depend on where the grid's beats fall, only on how far apart they are.

    Args:
        frames: The windows, one per row, each `per` whole beats long.
        per: How many beats each window is.
        corner: The top of the low band in hertz.

    Returns:
        One pulse per window.
    """
    count, window = frames.shape
    spectrum = np.fft.rfft(frames, axis=1)
    bins = np.fft.rfftfreq(window, 1.0 / SAMPLE_RATE)
    spectrum[:, bins >= corner] = 0.0
    low = np.fft.irfft(spectrum, n=window, axis=1)
    beat = window // per
    energy = (low * low)[:, : beat * per].reshape(count, per, beat).mean(axis=1)
    slot = beat // SLOTS_PER_BEAT
    slots = energy[:, : slot * SLOTS_PER_BEAT].reshape(count, SLOTS_PER_BEAT, slot).mean(axis=2)
    return decibels(slots.max(axis=1)) - decibels(slots.min(axis=1))


def kicking(samples: np.ndarray, window: int) -> np.ndarray:
    """Whether each beat of the audio has a kick under it.

    A beat has a kick when the low band's pitch sweeps down through it, which
    is what [`sweep_starts`][tools.grid_check.sweep_starts] finds. The profile
    the test reads is the average of the
    [`KICK_BEATS`][tools.track_profile.KICK_BEATS] beats around the beat being
    judged, the way [`offsets`][tools.grid_check.offsets] builds the profile
    for a check. One beat on its own is not enough, because a single kick's
    sweep can sit under a bass note or a fill, and the average over a bar holds
    where one beat does not. The pulse the `beats` command
    prints cannot answer this either: a bass line that stops between the beats
    pulses harder than a kick whose tail fills the beat, so a high pulse is not
    a kick.

    Args:
        samples: The audio, starting on a grid beat.
        window: How many samples one beat covers.

    Returns:
        True for each whole beat of the audio that has a kick.
    """
    band = grid_check.low_band(samples)
    pitch = grid_check.instant_frequency(band)
    envelope = grid_check.band_envelope(band)
    half = window // 2
    edges = grid_check.bin_edges(2 * half)
    bin_seconds = float(edges[-1]) / (len(edges) - 1) / SAMPLE_RATE
    look = max(1, int(round(grid_check.SWEEP_SECONDS / bin_seconds)))
    total = len(samples) // window
    found = []
    for index in range(total):
        first = max(0, min(index - KICK_BEATS // 2, total - KICK_BEATS))
        centers = [beat * window + half for beat in range(first, min(total, first + KICK_BEATS))]
        beat_pitch = grid_check.beat_profile(pitch, centers, half, edges)
        beat_envelope = grid_check.beat_profile(envelope, centers, half, edges)
        if beat_pitch is None or beat_envelope.max() < grid_check.FLOOR:
            found.append(False)
            continue
        found.append(bool(grid_check.sweep_starts(beat_pitch, beat_envelope, look)))
    return np.array(found)


def kick_changes(has_kick: np.ndarray) -> list[tuple[int, str]]:
    """The beats where the kick leaves and where it returns.

    A run of [`RUN_BEATS`][tools.track_profile.RUN_BEATS] beats or more that
    all have a kick is the kick playing, and a run of that many beats or more
    that all have none is the kick gone. The kick leaves at the first beat of a
    run with none and returns at the first beat of a run with one, so a single
    kick inside a break is a fill rather than the return, and a beat whose kick
    the test misses inside a section is not a break. A run shorter than
    `RUN_BEATS` beats belongs to neither state, and the command reports nothing
    for it.

    Args:
        has_kick: True for each beat that has a kick.

    Returns:
        The beat and the words to print, in beat order.
    """
    changes: list[tuple[int, str]] = []
    state: bool | None = None
    beat = 0
    while beat < len(has_kick):
        value = bool(has_kick[beat])
        end = beat
        while end < len(has_kick) and bool(has_kick[end]) == value:
            end += 1
        if end - beat >= RUN_BEATS and value != state:
            if state is not None:
                changes.append((beat, "kick returns" if value else "kick leaves"))
            state = value
        beat = end
    return changes


def level_steps(levels: np.ndarray) -> list[tuple[int, float]]:
    """The beats where the level steps, and how far it steps.

    The step at a beat is the median level of the
    [`RUN_BEATS`][tools.track_profile.RUN_BEATS] beats from that beat on, less
    the median level of the same many beats before it. A step of
    [`STEP_DB`][tools.track_profile.STEP_DB] or more marks a stretch of beats
    rather than one beat, because the two runs straddle the change for several
    beats either side of it, so the change is placed on the beat within that
    stretch whose own level moved furthest in the step's direction.

    Args:
        levels: The level of each beat, in decibels.

    Returns:
        The beat and the size of the step, in beat order.
    """
    first = RUN_BEATS
    last = len(levels) - RUN_BEATS + 1
    steps = {}
    for beat in range(first, last):
        before = float(np.median(levels[beat - RUN_BEATS : beat]))
        after = float(np.median(levels[beat : beat + RUN_BEATS]))
        if abs(after - before) >= STEP_DB:
            steps[beat] = after - before
    changes = []
    stretch: list[int] = []
    for beat in list(range(first, last)) + [None]:
        if beat is not None and beat in steps:
            stretch.append(beat)
            continue
        if stretch:
            direction = 1.0 if np.mean([steps[one] for one in stretch]) > 0.0 else -1.0
            best = max(stretch, key=lambda one: direction * (levels[one] - levels[one - 1]))
            changes.append((best, steps[best]))
            stretch = []
    return changes


def print_rows(labels: list[str], columns: list[tuple[str, np.ndarray]]) -> None:
    """Print one line per window, ending in a bar that grows with the level."""
    levels = columns[0][1]
    for row, label in enumerate(labels):
        numbers = "  ".join(f"{name} {values[row]:6.1f}" for name, values in columns)
        bar = "#" * int(max(0.0, levels[row] + 40.0))
        print(f"{label}  {numbers}  {bar}")


def run_beats(args: argparse.Namespace) -> int:
    """Run the `beats` command."""
    if args.bpm <= 0:
        sys.exit("error: --bpm must be more than zero")
    if args.per < 1:
        sys.exit("error: --per must be at least one beat")
    if args.to <= args.begin:
        sys.exit("error: --to must be a later beat than --from")
    if args.first_beat_sample is not None:
        first_beat = args.first_beat_sample / SAMPLE_RATE
    else:
        first_beat = args.first_beat or 0.0
    beat = 60.0 / args.bpm
    start = first_beat + args.begin * beat
    if start < 0:
        sys.exit(f"error: beat {args.begin} is before the start of the file")
    samples = decoding.decode(args.file, start, (args.to - args.begin) * beat)
    # Every window is a whole number of beats of a whole number of samples,
    # so the beats fold onto each other exactly. The rounding drifts by at
    # most half a sample per beat, a few milliseconds over a long stretch.
    window = int(round(beat * SAMPLE_RATE)) * args.per
    frames = frame(samples, window)
    if len(frames) == 0:
        sys.exit(f"error: the file ends before beat {args.begin + args.per}")
    labels = [f"beat {args.begin + i * args.per:5d}" for i in range(len(frames))]
    print_rows(labels, [("full", level(frames)), ("pulse", pulse(frames, args.per, args.corner))])
    return 0


def run_sections(args: argparse.Namespace) -> int:
    """Run the `sections` command."""
    if args.bpm <= 0:
        sys.exit("error: --bpm must be more than zero")
    if args.to <= args.begin:
        sys.exit("error: --to must be a later beat than --from")
    if args.first_beat_sample is not None:
        first_beat = args.first_beat_sample / SAMPLE_RATE
    else:
        first_beat = args.first_beat or 0.0
    beat = 60.0 / args.bpm
    start = first_beat + args.begin * beat
    if start < 0:
        sys.exit(f"error: beat {args.begin} is before the start of the file")
    samples = decoding.decode(args.file, start, (args.to - args.begin) * beat)
    window = int(round(beat * SAMPLE_RATE))
    frames = frame(samples, window)
    if len(frames) < 2 * RUN_BEATS + 1:
        sys.exit(f"error: the stretch is under {2 * RUN_BEATS + 1} beats long, which is too short to compare")
    changes = list(kick_changes(kicking(samples, window)))
    covered = {beat_index for beat_index, _ in changes}
    for beat_index, step in level_steps(level(frames)):
        if any(abs(beat_index - one) <= COVERED_BEATS for one in covered):
            continue
        changes.append((beat_index, f"level {step:+.1f} dB"))
    changes.sort()
    for beat_index, words in changes:
        at = args.begin + beat_index
        print(f"beat {at:5d}  {words:12s}  ({at % PHRASE_BEATS} mod {PHRASE_BEATS})")
    if not changes:
        print(f"nothing changes between beat {args.begin} and beat {args.to}")
    return 0


def run_seconds(args: argparse.Namespace) -> int:
    """Run the `seconds` command."""
    if args.window <= 0:
        sys.exit("error: --window must be more than zero seconds")
    if args.begin < 0:
        sys.exit("error: --from must not be before the file starts")
    if args.to is not None and args.to <= args.begin:
        sys.exit("error: --to must be later than --from")
    length = None if args.to is None else args.to - args.begin
    samples = decoding.decode(args.file, args.begin, length)
    frames = frame(samples, int(round(args.window * SAMPLE_RATE)))
    if len(frames) == 0:
        sys.exit(f"error: the file ends before {args.begin + args.window:.1f} seconds")
    labels = [f"{args.begin + i * args.window:7.1f}s" for i in range(len(frames))]
    print_rows(labels, [("full", level(frames))])
    return 0


def main() -> int:
    """Parse the command line and run the command it names."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)

    beats = commands.add_parser("beats", help="one line per window of beats on the track's grid")
    beats.add_argument("file", help="the audio file")
    beats.add_argument("--bpm", type=float, required=True, help="the track's tempo")
    first = beats.add_mutually_exclusive_group()
    first.add_argument(
        "--first-beat",
        type=float,
        metavar="SECONDS",
        help="the time of beat zero in seconds (zero if neither form is given)",
    )
    first.add_argument(
        "--first-beat-sample",
        type=int,
        metavar="N",
        help="the sample of beat zero, as `first_beat_sample` in the JSON the library prints",
    )
    beats.add_argument("--from", dest="begin", type=int, default=0, metavar="BEAT", help="the first beat (0)")
    beats.add_argument("--to", type=int, required=True, metavar="BEAT", help="the beat to stop before")
    beats.add_argument("--per", type=int, default=1, metavar="BEATS", help="beats per line (1); 4 gives bars")
    beats.add_argument(
        "--corner",
        type=float,
        default=DEFAULT_CORNER_HZ,
        metavar="HZ",
        help=f"the top of the band whose pulse is measured ({DEFAULT_CORNER_HZ:.0f})",
    )
    beats.set_defaults(run=run_beats)

    sections = commands.add_parser("sections", help="one line per beat at which a section changes")
    sections.add_argument("file", help="the audio file")
    sections.add_argument("--bpm", type=float, required=True, help="the track's tempo")
    section_first = sections.add_mutually_exclusive_group()
    section_first.add_argument(
        "--first-beat",
        type=float,
        metavar="SECONDS",
        help="the time of beat zero in seconds (zero if neither form is given)",
    )
    section_first.add_argument(
        "--first-beat-sample",
        type=int,
        metavar="N",
        help="the sample of beat zero, as `first_beat_sample` in the JSON the library prints",
    )
    sections.add_argument("--from", dest="begin", type=int, default=0, metavar="BEAT", help="the first beat (0)")
    sections.add_argument("--to", type=int, required=True, metavar="BEAT", help="the beat to stop before")
    sections.set_defaults(run=run_sections)

    seconds = commands.add_parser("seconds", help="one line per window of seconds, for a rendered clip")
    seconds.add_argument("file", help="the audio file")
    seconds.add_argument("--window", type=float, default=2.0, metavar="SECONDS", help="seconds per line (2)")
    seconds.add_argument("--from", dest="begin", type=float, default=0.0, metavar="SECONDS", help="where to start (0)")
    seconds.add_argument("--to", type=float, metavar="SECONDS", help="where to stop (the end of the file)")
    seconds.set_defaults(run=run_seconds)

    args = parser.parse_args()
    return args.run(args)


if __name__ == "__main__":
    sys.exit(main())
