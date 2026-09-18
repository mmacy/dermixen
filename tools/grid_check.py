#!/usr/bin/env python3
"""Check a track's beat grid against the kicks in its audio.

A kick drum's low band sweeps down in pitch over its first hundred
milliseconds, from 150-175 Hz to around 50 Hz, and a bass note holds one
pitch. That difference is what this program looks for, so a grid that sits on
an offbeat bass note reads as wrong even when the bass is as loud as the kick.
Every `--every` beats across the stretch given, the program searches the audio
around the beat the grid predicts for the moment a sweep starts, and the
distance from the grid's beat to that moment is the offset.

The program measures each offset against the offset the check before it found,
folds that difference into one beat, and adds the differences up, the way a
phase is unwrapped. A check that re-found the kick a whole beat away after a
breakdown does not read as drift, and a steady drift of any size still adds up
however far the offsets walk from the grid. The program fits a line through
the offsets it has added up. The first line of the report gives the offset at
the first beat, how the offset drifts along the track, how many of the checks
sit on the line out of every check the program made, and the tempo that would
remove the drift. The second line says where the kick starts and, when there
is one kick per beat, which `--first-beat` would move the grid onto the kick.

A grid that holds shows no drift, a fitted tempo within a few thousandths of
a BPM of the given one, and about three quarters or more of the checks on the
line, with the rest in stretches that have no kick. An offset near a quarter
or a half of a beat is a grid whose first beat is off by that much. A drift is
a tempo that is off, and the fitted tempo is the one to give `mix add --bpm`.
A check that finds no kick is counted and left off the line, with no entry in
the listing. A check whose kick sits further from where the previous check
predicts it than the search reaches, or whose offset misses the fitted line by
more than the tolerance, is off the line. Those checks are beats with no kick
under them, as in a breakdown, unless there are many of them along the whole
stretch, in which case the grid does not describe the track.

Two kick onsets per beat is what a rendered handover contains when its two
tracks' grids disagree. The report says so and how far apart the two onsets
sit, instead of picking one of them and calling the other drift.

The tempo and the first beat come from the `grid` object that `dermixen
library query --json` and `dermixen mix show --json` print, as `bpm` and
`first_beat_sample`. Run this on a track whose `grid_confidence` is low, or
whose transition sounds off although the anchors sit on the right beats.

Example:
    ```
    python3 tools/grid_check.py track.mp3 --bpm 148.55 --first-beat-sample 68616 --from 0 --to 1024
    ```

It needs the built `dermixen` command, which decodes the audio, and the
`numpy` package.
"""

import argparse
import sys

import numpy as np

# Python searches the directory of the file it is running before anywhere
# else, and `decoding.py` sits in that same directory, so the plain name
# resolves whether this program runs on its own or `track_profile.py` imports
# it.
import decoding

# The sample rate the whole project is fixed at, matching `SAMPLE_RATE` in
# `crates/core/src/units.rs`. Audio is decoded to it whatever rate the file has.
SAMPLE_RATE = 44100

# The band a kick's sweep runs through, in hertz. The sweep starts under
# 200 Hz and ends around 50 Hz, and cutting everything outside the band keeps
# the zero crossings from following a hi-hat or a lead.
BAND_LOW_HZ = 30.0
BAND_HIGH_HZ = 200.0

# The length of the moving average that smooths the band into an envelope, in
# seconds. Ten milliseconds is short enough that a kick's attack is not spread
# across the hit and the gap after it. The average trails the moment it is read
# at rather than being centered on it, so a kick does not read as loud before
# the kick has started.
SMOOTH_SECONDS = 0.010

# How wide each bin of a beat's profile is, in seconds. Ten milliseconds
# places a sweep start to within five milliseconds, and forty-odd bins cover a
# beat at any tempo a mix is built at.
BIN_SECONDS = 0.010

# How far ahead of a bin the pitch is compared with, in seconds, and how far
# the pitch has to fall over that stretch for the bin to start a sweep. A kick
# falls by forty to eighty hertz over its first sixty milliseconds and a bass
# note falls by nothing, so twenty-five hertz separates the two with room to
# spare either way.
SWEEP_SECONDS = 0.060
SWEEP_DROP_HZ = 25.0

# How loud the band has to stay across those sixty milliseconds for the fall
# in pitch to count, as a share of the loudest moment of the beat. A fall into
# silence is the end of a note, not the start of a kick.
ENVELOPE_SHARE = 1.0 / 3.0

# How many beats the program averages into the profile of one check. The
# average makes the sweep plain on a track whose bass note is as loud as its
# kick, and sixteen beats is four bars, which is shorter than any section.
AVERAGE_BEATS = 16

# How far each side of where the previous offset predicts the kick a later
# check is searched, in seconds. A beat at 150 BPM is 400 milliseconds, so this
# window is under a third of a beat and cannot reach the kick of the beat
# before or after, while a grid off by a sixteenth note still finds its kick.
SEARCH_SECONDS = 0.12

# How many checks in a row may fail to find a kick within the search reach
# before the search widens to half a beat each side again. A stretch that opens
# before the kick arrives leaves the search pointing at nothing, and so does a
# breakdown that runs on, and the wider search is what finds the kick again.
LOST_CHECKS = 3

# A stretch whose envelope never rises above this is silence, and the grid
# cannot be checked there. It is 80 decibels below full scale.
FLOOR = 1e-4

# How far a check may sit from the fitted line and still count as following
# it, in milliseconds. Two kicks twenty milliseconds apart sound as one, so
# a check within this of the line is a beat the grid has right.
TOLERANCE_MS = 20.0

# Where the sweep should start, in milliseconds after the grid beat, so that
# every grid this check passes lines up with every other. The `--first-beat`
# the report suggests is the one that would put the sweep here.
TARGET_MS = 20.0

# How far apart two sweep starts have to sit to come from two kicks rather
# than one, in milliseconds, and what share of the checks each of the two has
# to appear on before the report calls them two kicks.
APART_MS = 60.0
ONSET_SHARE = 0.4

# How many of the first checks are searched half a beat each side and pooled
# to find where the kick sits at the start. One check alone can land on a
# bass accent instead of the kick, and the median of several does not.
STARTING_CHECKS = 8


def low_band(samples: np.ndarray) -> np.ndarray:
    """Cut the audio down to the band a kick's sweep runs through.

    The band is taken by a Fourier transform of the whole stretch with the bins
    outside [`BAND_LOW_HZ`][tools.grid_check.BAND_LOW_HZ] and
    [`BAND_HIGH_HZ`][tools.grid_check.BAND_HIGH_HZ] set to zero.

    Args:
        samples: The audio.

    Returns:
        The band, one value per input sample.
    """
    spectrum = np.fft.rfft(samples)
    bins = np.fft.rfftfreq(len(samples), 1.0 / SAMPLE_RATE)
    spectrum[(bins < BAND_LOW_HZ) | (bins >= BAND_HIGH_HZ)] = 0.0
    return np.fft.irfft(spectrum, n=len(samples))


def instant_frequency(band: np.ndarray) -> np.ndarray:
    """Read the pitch of the band from its zero crossings, one value per sample.

    The stretch between two crossings that are `span` samples apart is one half
    cycle, so the pitch across it is `SAMPLE_RATE / (2 * span)`. The result is
    clipped to the band, because a pair of crossings a sample or two apart is
    noise rather than a pitch.

    Args:
        band: The band-limited audio.

    Returns:
        The pitch in hertz at every sample.
    """
    negative = np.signbit(band)
    crossings = np.flatnonzero(negative[1:] != negative[:-1]) + 1
    pitch = np.full(len(band), BAND_HIGH_HZ)
    if len(crossings) < 2:
        return pitch
    spans = np.diff(crossings)
    half_cycles = np.clip(SAMPLE_RATE / (2.0 * spans), BAND_LOW_HZ, BAND_HIGH_HZ)
    pitch[crossings[0] : crossings[-1]] = np.repeat(half_cycles, spans)
    pitch[: crossings[0]] = half_cycles[0]
    pitch[crossings[-1] :] = half_cycles[-1]
    return pitch


def band_envelope(band: np.ndarray) -> np.ndarray:
    """Average the size of the band over the [`SMOOTH_SECONDS`][tools.grid_check.SMOOTH_SECONDS] before each sample.

    The average trails rather than being centered on the sample it belongs to,
    so the envelope at a moment says how loud the band has been up to that
    moment and not how loud it is about to become. A centered average would
    report a kick as loud five milliseconds before the kick starts.

    Args:
        band: The band-limited audio.

    Returns:
        The envelope, one value per input sample.
    """
    size = np.abs(band)
    width = max(1, int(round(SMOOTH_SECONDS * SAMPLE_RATE)))
    # A trailing moving average by cumulative sum.
    padded = np.concatenate([np.zeros(width), size])
    sums = np.cumsum(padded)
    return (sums[width:] - sums[:-width]) / width


def bin_edges(width: int) -> np.ndarray:
    """Where the bins of a beat's profile start and end, in samples.

    The bins divide the whole beat, so a sweep that starts at the very edge of
    the beat still lands in a bin.

    Args:
        width: How many samples one beat covers.

    Returns:
        One more edge than there are bins, running from zero to `width`.
    """
    count = max(4, int(round(width / (BIN_SECONDS * SAMPLE_RATE))))
    return np.linspace(0, width, count + 1).astype(int)


def beat_profile(
    values: np.ndarray, centers: list[int], half: int, edges: np.ndarray
) -> np.ndarray | None:
    """Average a per-sample measurement over one beat, across several beats.

    Each beat contributes the stretch from `half` samples before its center to
    `half` samples after, cut into the bins that `edges` describes, and the
    bins are averaged across the beats. A beat whose stretch falls outside the
    audio is left out.

    Args:
        values: The measurement, one value per sample.
        centers: The sample each beat is centered on.
        half: Half a beat, in samples.
        edges: The bin edges from [`bin_edges`][tools.grid_check.bin_edges].

    Returns:
        One value per bin, or None when no beat fell inside the audio.
    """
    total = np.zeros(len(edges) - 1)
    used = 0
    width = int(edges[-1])
    sizes = np.diff(edges)
    for center in centers:
        start = center - half
        if start < 0 or start + width > len(values):
            continue
        total += np.add.reduceat(values[start : start + width], edges[:-1]) / sizes
        used += 1
    if used == 0:
        return None
    return total / used


def sweep_starts(pitch: np.ndarray, envelope: np.ndarray, look: int) -> list[int]:
    """The bins of a beat's profile where a kick's pitch sweep starts.

    A bin starts a sweep when the pitch `look` bins later is at least
    [`SWEEP_DROP_HZ`][tools.grid_check.SWEEP_DROP_HZ] lower and the envelope
    stays at [`ENVELOPE_SHARE`][tools.grid_check.ENVELOPE_SHARE] of the beat's
    loudest moment or above the whole way. The comparison with the pitch ahead
    and the comparison with the envelope ahead both wrap around the beat,
    because the profile is a fold of many beats onto one. A kick fills a run of
    bins, and the sweep starts at the first bin of the run.

    Args:
        pitch: The pitch profile, one value per bin.
        envelope: The envelope profile, one value per bin.
        look: How many bins ahead the pitch is compared with.

    Returns:
        The first bin of each run, in the order the bins come.
    """
    fall = pitch - np.roll(pitch, -look)
    held = envelope.copy()
    for step in range(1, look + 1):
        held = np.minimum(held, np.roll(envelope, -step))
    qualifies = (fall >= SWEEP_DROP_HZ) & (held >= envelope.max() * ENVELOPE_SHARE)
    if not qualifies.any():
        return []
    if qualifies.all():
        return [int(np.argmax(fall))]
    starts = qualifies & ~np.roll(qualifies, 1)
    return [int(index) for index in np.flatnonzero(starts)]


def gap(value: float, center: float, period: float) -> float:
    """How far `value` sits from `center`, going the short way round one beat.

    Args:
        value: A position, in the same unit as `period`.
        center: The position to measure from.
        period: One beat.

    Returns:
        A distance between minus half a beat and half a beat.
    """
    return (value - center + period / 2.0) % period - period / 2.0


def fold(values: np.ndarray, center: float, period: float) -> np.ndarray:
    """Bring every value into the one beat that `center` sits in the middle of.

    Args:
        values: The positions to fold.
        center: The middle of the beat to fold into.
        period: One beat.

    Returns:
        The folded positions.
    """
    return center + (values - center + period / 2.0) % period - period / 2.0


def circular_center(values: np.ndarray, period: float) -> float:
    """Where a set of positions sits on the beat, treating the beat as a circle.

    A plain average would put two positions either side of the fold's edge in
    the middle of the beat instead of at its edge. Averaging them as directions
    round a circle puts the answer where the positions are.

    Args:
        values: The positions.
        period: One beat.

    Returns:
        A position between minus half a beat and half a beat.
    """
    angles = 2.0 * np.pi * values / period
    direction = np.arctan2(float(np.mean(np.sin(angles))), float(np.mean(np.cos(angles))))
    return float(direction / (2.0 * np.pi) * period)


def offsets(
    pitch: np.ndarray,
    envelope: np.ndarray,
    beat_seconds: float,
    lead: int,
    count: int,
    every: int,
) -> tuple[np.ndarray, np.ndarray, list[list[float]], int]:
    """Measure how far the kick's sweep starts from every `every`th grid beat.

    The first [`STARTING_CHECKS`][tools.grid_check.STARTING_CHECKS] checks are
    each searched half a beat each side and the median of what they find is
    where the kick starts out, so a grid whose first beat is off by any amount
    is measured from the kick nearest it. Each later check is searched
    [`SEARCH_SECONDS`][tools.grid_check.SEARCH_SECONDS] each side of where the
    previous offset predicts its kick, and the sweep it finds there is added to
    the previous offset. Each step is a difference folded into one beat and the
    offsets are those differences added up, so a tempo that is off is followed
    along the track and measured however far the offsets walk.

    A check that finds a sweep outside the search reach keeps the offset it
    measured but does not move the search, because a kick that far away is more
    likely another instrument than the kick the program was following. After
    [`LOST_CHECKS`][tools.grid_check.LOST_CHECKS] checks in a row that find
    nothing to follow, the search widens to half a beat each side until it
    finds a kick again, which is how the program finds the kick in a stretch
    that opens before the kick arrives.

    Args:
        pitch: The pitch at every sample.
        envelope: The envelope at every sample.
        beat_seconds: The length of a beat.
        lead: The sample that the first grid beat falls on.
        count: How many beats the audio covers from that sample.
        every: The stride in beats.

    Returns:
        The beats measured, counted from the first grid beat, the offset of the
        sweep from the grid at each in milliseconds, positive when the sweep is
        late, every sweep start each check found, also in milliseconds, and how
        many checks the program made in all, measured or not.
    """
    half = int(round(beat_seconds * SAMPLE_RATE / 2))
    edges = bin_edges(2 * half)
    bin_seconds = float(edges[-1]) / (len(edges) - 1) / SAMPLE_RATE
    look = max(1, int(round(SWEEP_SECONDS / bin_seconds)))
    reach = int(round(SEARCH_SECONDS * SAMPLE_RATE))

    def grid_of(beat: int) -> int:
        return lead + int(round(beat * beat_seconds * SAMPLE_RATE))

    def sweeps_near(beat: int, previous: int) -> list[tuple[int, float]]:
        first = max(0, min(beat - AVERAGE_BEATS // 2, count - AVERAGE_BEATS))
        centers = [grid_of(k) + previous for k in range(first, min(count, first + AVERAGE_BEATS))]
        beat_pitch = beat_profile(pitch, centers, half, edges)
        beat_envelope = beat_profile(envelope, centers, half, edges)
        if beat_pitch is None or beat_envelope.max() < FLOOR:
            return []
        # A sweep is placed at the middle of the bin it starts in, which is the
        # best guess at where in that ten milliseconds the kick began.
        return [
            ((int(edges[index]) + int(edges[index + 1])) // 2 - half, float(beat_envelope[index]))
            for index in sweep_starts(beat_pitch, beat_envelope, look)
        ]

    starts: list[int] = []
    for beat in range(0, count, every):
        near = sweeps_near(beat, 0)
        if near:
            starts.append(max(near, key=lambda found: found[1])[0])
        if len(starts) == STARTING_CHECKS:
            break
    if not starts:
        return np.array([]), np.array([]), [], len(range(0, count, every))
    # The starting checks are pooled round the first of them, so two checks
    # either side of the fold's edge do not average to the middle of the beat.
    width = float(2 * half)
    pooled = [starts[0] + gap(float(start), float(starts[0]), width) for start in starts]
    previous = int(round(float(np.median(pooled))))

    beats: list[int] = []
    found: list[float] = []
    seen: list[list[float]] = []
    checks = 0
    lost = 0
    for beat in range(0, count, every):
        checks += 1
        limit = half if lost >= LOST_CHECKS else reach
        near = sweeps_near(beat, previous)
        if not near:
            lost += 1
            continue
        seen.append([(previous + offset) / SAMPLE_RATE * 1000.0 for offset, _ in near])
        within = [candidate for candidate in near if abs(candidate[0]) <= limit]
        if not within:
            # The check measured a sweep, so it has an offset to report, and
            # that offset lands off the fitted line. The search stays where it
            # was rather than following a sweep this far from the kick.
            lost += 1
            loudest = max(near, key=lambda candidate: candidate[1])[0]
            beats.append(beat)
            found.append((previous + loudest) / SAMPLE_RATE * 1000.0)
            continue
        lost = 0
        previous += max(within, key=lambda candidate: candidate[1])[0]
        beats.append(beat)
        found.append(previous / SAMPLE_RATE * 1000.0)
    return np.array(beats), np.array(found), seen, checks


def fit_line(beats: np.ndarray, found: np.ndarray) -> tuple[float, float]:
    """Fit a line through the offsets that a breakdown cannot pull off course.

    The slope is the median of the slopes between every pair of checks, and the
    intercept the median of what is left once that slope is taken out, so a
    stretch with no kick, where the search wanders, does not move the line the
    way a least-squares fit would let it.

    Args:
        beats: The beat each check sits on.
        found: The offset at each check, in milliseconds.

    Returns:
        The slope in milliseconds per beat, and the offset at beat zero.
    """
    i, j = np.triu_indices(len(beats), k=1)
    slope = float(np.median((found[j] - found[i]) / (beats[j] - beats[i])))
    intercept = float(np.median(found - slope * beats))
    return slope, intercept


def onset_clusters(seen: list[list[float]], center: float, period: float) -> list[tuple[float, int]]:
    """Group every sweep start the checks found into the kicks the sweeps came from.

    Two sweep starts within half of [`APART_MS`][tools.grid_check.APART_MS] of
    each other come from one kick. The groups are taken largest first, so the
    kick that most checks found is the first group.

    Args:
        seen: The sweep starts each check found, in milliseconds.
        center: The middle of the beat to fold the groups into.
        period: One beat, in milliseconds.

    Returns:
        Each group's position in milliseconds and how many checks found it,
        largest group first.
    """
    items = [(value, index) for index, row in enumerate(seen) for value in row]
    radius = APART_MS / 2.0
    clusters: list[tuple[float, int]] = []
    while items:
        best = max(
            (value for value, _ in items),
            key=lambda value: len({index for other, index in items if abs(gap(other, value, period)) <= radius}),
        )
        members = [pair for pair in items if abs(gap(pair[0], best, period)) <= radius]
        items = [pair for pair in items if abs(gap(pair[0], best, period)) > radius]
        aligned = [best + gap(value, best, period) for value, _ in members]
        position = float(fold(np.array([float(np.median(aligned))]), center, period)[0])
        clusters.append((position, len({index for _, index in members})))
    clusters.sort(key=lambda cluster: -cluster[1])
    return clusters


def onset_line(
    seen: list[list[float]],
    folded: np.ndarray,
    on_line: np.ndarray,
    first_beat: float,
    period: float,
) -> str:
    """Write the line that says where the kick starts, and whether there are two.

    The `--first-beat` the line suggests is moved forward a whole beat at a
    time until it is at or after the start of the file. A kick that sits
    further before the grid beat than the grid's first beat sits after the
    start of the file would otherwise give a negative time, and beat zero
    cannot fall before the file starts. A kick sits on every beat, so every
    whole beat later names the same grid.

    Args:
        seen: The sweep starts each check found, in milliseconds.
        folded: The offset of every check, folded into one beat.
        on_line: True for each check that sits on the fitted line.
        first_beat: The first beat the user gave, in seconds.
        period: One beat, in milliseconds.

    Returns:
        The line to print.
    """
    center = circular_center(folded, period)
    clusters = onset_clusters(seen, center, period)
    enough = max(1, int(round(ONSET_SHARE * len(seen))))
    kicks = [cluster for cluster in clusters if cluster[1] >= enough]
    if len(kicks) == 2 and abs(gap(kicks[0][0], kicks[1][0], period)) >= APART_MS:
        low, high = sorted(position for position, _ in kicks)
        return f"two kick onsets per beat: {low:+.0f} ms and {high:+.0f} ms, {high - low:.0f} ms apart"
    settled = folded[on_line] if on_line.any() else folded
    middle = float(np.median(settled))
    moved = first_beat + (middle - TARGET_MS) / 1000.0
    while moved < 0.0:
        moved += period / 1000.0
    return (
        f"kick starts {middle:+.0f} ms after the grid beat, one kick onset per beat. "
        f"--first-beat {moved:.3f} would put it at {TARGET_MS:+.0f} ms"
    )


def main() -> int:
    """Parse the command line and print the report.

    Returns:
        The process exit status.
    """
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("file", help="the audio file")
    parser.add_argument("--bpm", type=float, required=True, help="the grid's tempo")
    first = parser.add_mutually_exclusive_group()
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
    parser.add_argument("--from", dest="begin", type=int, default=0, metavar="BEAT", help="the first beat (0)")
    parser.add_argument("--to", type=int, required=True, metavar="BEAT", help="the beat to stop before")
    parser.add_argument("--every", type=int, default=8, metavar="BEATS", help="the stride between checks (8)")
    args = parser.parse_args()

    if args.bpm <= 0:
        sys.exit("error: --bpm must be more than zero")
    if args.every < 1:
        sys.exit("error: --every must be at least one beat")
    if args.to <= args.begin:
        sys.exit("error: --to must be a later beat than --from")
    if args.first_beat_sample is not None:
        first_beat = args.first_beat_sample / SAMPLE_RATE
    else:
        first_beat = args.first_beat or 0.0
    beat = 60.0 / args.bpm
    count = args.to - args.begin
    if first_beat + args.begin * beat < 0:
        sys.exit(f"error: beat {args.begin} is before the start of the file")
    # The stretch starts half a beat and one search window early, so the first
    # beat can be searched on both sides however far the kick sits from it, and
    # it runs the same distance past the last beat. That lead can itself reach
    # before the file starts, for a beat near the start of the track, and
    # `decoding.decode` covers that by reading from the start of the file and
    # putting silence in front of the result, so the samples still line up
    # with the beats being searched.
    lead = int(round((beat / 2.0 + SEARCH_SECONDS) * SAMPLE_RATE))
    start = first_beat + args.begin * beat - lead / SAMPLE_RATE
    samples = decoding.decode(args.file, start, count * beat + 2.0 * lead / SAMPLE_RATE)
    band = low_band(samples)
    beats, found, seen, checks = offsets(
        instant_frequency(band), band_envelope(band), beat, lead, count, args.every
    )
    if len(beats) < 2:
        sys.exit(
            "error: fewer than two beats in the stretch have a kick to check against. "
            "A tempo given more than about three percent off the track's own reads the same way, "
            "because the sweep then smears out of the averaged profile"
        )
    beats = beats + args.begin
    period = beat * 1000.0
    slope, intercept = fit_line(beats, found)
    # Kicks repeat every beat, so an offset of a whole number of beats names
    # the same grid. Taking those whole beats out of every offset at once
    # leaves the drift and the residuals as they are and keeps the offsets
    # near the grid rather than whole beats from it. The offset printed at
    # `--from` still grows along a stretch whose tempo is off, because that
    # growth is the drift the report measures.
    whole = round(intercept / period) * period
    found = found - whole
    intercept -= whole
    residual = found - (slope * beats + intercept)
    on_line = np.abs(residual) <= TOLERANCE_MS
    fitted = 60000.0 / (beat * 1000.0 + slope)
    print(
        f"kick offset from the grid every {args.every} beats from {args.begin} to {args.to}: "
        f"{slope * args.begin + intercept:+.0f} ms at beat {args.begin}, "
        f"drift {slope * 1000.0:+.1f} ms per 1000 beats, "
        f"{int(np.sum(on_line))} of {checks} checks within {TOLERANCE_MS:.0f} ms of that line, "
        f"tempo fits {fitted:.3f} (given {args.bpm:.3f})"
    )
    # The second line says where on the beat the kick sits, so it reads the
    # offsets folded back into one beat. The fit above reads them added up.
    phase = fold(found, circular_center(found, period), period)
    print(onset_line(seen, phase, on_line, first_beat, period))
    stride = max(1, len(beats) // 12)
    for index in range(0, len(beats), stride):
        note = "" if on_line[index] else "  off the line"
        print(f"  beat {beats[index]:5d}: {found[index]:+6.0f} ms{note}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
