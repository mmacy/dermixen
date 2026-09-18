"""Read MixMeister Fusion `.mmp` playlist files and print what is inside them.

MixMeister stores a playlist as a RIFF container with the form type `MXMP`. Every
position in the file is a 64-bit count of microseconds, and automation nodes are
fixed 32-byte records. This module turns one of those files into ordinary Python
objects so that a future importer has something to build on, and so that
assumptions about the format can be checked against real playlists rather than
against a single sample.

Run it directly to print a readable summary:

```
python3 tools/mmp_dump.py "sample playlist.mmp"
python3 tools/mmp_dump.py --json "sample playlist.mmp"
```

The chunk names and field offsets below are checked against real playlists.
Anything unidentified says so rather than guessing: see
[`LANE_NAMES`][tools.mmp_dump.LANE_NAMES].
"""

import argparse
import dataclasses
import json
import re
import struct
import sys
from pathlib import Path

MICROSECONDS_PER_SECOND = 1_000_000
"""Every position stored in a `.mmp` file counts microseconds."""

BEATS_PER_BAR = 4
"""MixMeister writes a bar period rather than a tempo, and assumes four beats to the bar."""

MARKER_SIZE = 32
"""Size in bytes of one automation node (`TRKM`) or span (`TRKS`) record."""

CONTAINER_IDS = frozenset({b"RIFF", b"LIST"})
"""Chunk identifiers whose payload holds further chunks rather than data."""

MAX_FILE_SIZE = 64 * 1024 * 1024
"""Largest file this module reads as a playlist. The largest real playlist on
record is under 1 MB, so this limit is generous rather than tight."""

MAX_NESTING_DEPTH = 25
"""Deepest a chunk inside a track is let nest. A real playlist nests a handful
of levels deep at most, so a file that nests further is damaged rather than
unusual."""

CONTROL_CHARACTERS = re.compile(r"[\x00-\x1f\x7f-\x9f]")
"""A C0, C1, or DEL control character, the kind a terminal can act on."""

LANE_NAMES: dict[int, str] = {
    0x10000000: "beatgrid",
    0x02000000: "initial mix tempo",
    0x01000000: "mix tempo",
    0x00040000: "strip gain 4",
    0x00020000: "strip gain 3",
    0x00010000: "strip gain 2",
    0x00008000: "strip gain 1",
    0x00000100: "outro gain",
    0x00000080: "intro gain",
    0x00000002: "measure marker",
    0x00000001: "cue",
}
"""Automation lane identifiers, mapped to what each lane holds.

Most of these are checked against twenty real playlists holding 245 tracks
between them.

The beatgrid lane never has a value attached. The mix tempo lane holds beats
per minute and gets roughly two nodes per track, each equal to that track's own
original BPM, so the tempo of the mix follows each song in turn rather than
forcing every song to one number. The initial mix tempo lane holds a single node
per playlist, always on the first track. The intro and outro gain lanes hold
decibels and correspond to the separate intro and outro marker lists in
Fusion's own `transitions.xml`: intro nodes sit near the start of a track and
rise, outro nodes sit near the end and fall to the silence floor of -50 dB.

The four strip gain lanes occupy consecutive bits and all hold decibels. Fusion
gives each track four such strips, named Volume, Bass, Midrange, and Treble, and
a track can use more than one at once. Which bit belongs to which strip is not
established, so they are numbered rather than named.
"""


@dataclasses.dataclass
class Marker:
    """One automation node, stored as a `TRKM` chunk.

    Attributes:
        lane: Identifier of the lane this node belongs to. See
            [`LANE_NAMES`][tools.mmp_dump.LANE_NAMES].
        position_us: Position in microseconds.
        value: Meaning depends on the lane. The gain lanes hold decibels, the
            two tempo lanes hold beats per minute, and the beatgrid, cue, and
            measure marker lanes leave this at zero.
        interpolation: Small integer that varies between nodes. Its exact
            meaning is not established.
    """

    lane: int
    position_us: int
    value: float
    interpolation: int

    @property
    def lane_name(self) -> str:
        """Human-readable name of the lane, or the raw identifier if unknown."""
        return LANE_NAMES.get(self.lane, f"unknown lane 0x{self.lane:08x}")

    @property
    def position_seconds(self) -> float:
        """Position expressed in seconds."""
        return self.position_us / MICROSECONDS_PER_SECOND


@dataclasses.dataclass
class Span:
    """A range within a track, stored as a `TRKS` chunk.

    Three microsecond positions appear in each record. The first two bound a
    range on the timeline and the third points into the source audio, which is
    consistent with the segments Fusion lets a user cut a track into. The exact
    roles are not confirmed, so all three are reported as they are found.

    Attributes:
        start_us: First position in the record.
        end_us: Second position in the record.
        source_us: Third position in the record.
    """

    start_us: int
    end_us: int
    source_us: int


@dataclasses.dataclass
class Track:
    """One entry in the playlist, stored as a `LIST` chunk of type `TRKI`.

    Attributes:
        path: Source audio file, as written by whichever machine saved the
            playlist. These are usually Windows paths and need remapping before
            the audio can be found.
        original_bpm: Tempo MixMeister detects when a file enters its library.
            This value never changes; the tempo a track actually plays at comes
            from the mix tempo curve.
        bar_period_us: Length of one bar in microseconds, from the `TKTH` chunk.
        transition_in: Name of the transition into this track, if any.
        transition_out: Name of the transition out of this track, if any.
        beatgrid: Markers from the `TKTM` list, which hold the beat grid.
        automation: Markers from the `TKLY` list, which hold volume, tempo, and
            the other automation lanes.
        spans: Every `TRKS` record found anywhere in the track.
        identity: Sixteen bytes that identify the track, as a hex string.
    """

    path: str = ""
    original_bpm: float = 0.0
    bar_period_us: int = 0
    transition_in: str = ""
    transition_out: str = ""
    beatgrid: list[Marker] = dataclasses.field(default_factory=list)
    automation: list[Marker] = dataclasses.field(default_factory=list)
    spans: list[Span] = dataclasses.field(default_factory=list)
    identity: str = ""

    @property
    def bar_period_bpm(self) -> float:
        """Tempo implied by [`bar_period_us`][tools.mmp_dump.Track.bar_period_us].

        Returns:
            Beats per minute, or 0.0 when no bar period was recorded.
        """
        if not self.bar_period_us:
            return 0.0
        return BEATS_PER_BAR * 60 * MICROSECONDS_PER_SECOND / self.bar_period_us


@dataclasses.dataclass
class Playlist:
    """A whole `.mmp` file.

    Attributes:
        tracks: Playlist entries in order.
        version: Numbers from the `MMVR` chunk at the end of the file.
    """

    tracks: list[Track] = dataclasses.field(default_factory=list)
    version: tuple[int, ...] = ()


def _visible(text: str) -> str:
    """`text` with every control character replaced by a visible escape.

    A path or a transition name comes straight out of the file, and a control
    character in it, such as an escape or a bell, can act on the terminal
    printing it. Each C0, C1, or DEL character becomes `\\xHH` instead.
    """
    return CONTROL_CHARACTERS.sub(lambda match: f"\\x{ord(match.group()):02x}", text)


def _decode_text(raw: bytes) -> str:
    """Decode a UTF-16LE string chunk, dropping its terminating null."""
    return raw.decode("utf-16-le", errors="replace").rstrip("\x00")


def _chunk_header(data: bytes, offset: int, end: int) -> tuple[bytes, int, int]:
    """The identifier, declared size, and body offset of the chunk at `offset`.

    Args:
        data: The whole file.
        offset: Offset of the chunk's identifier.
        end: Offset the chunk's header must fit before.

    Returns:
        The four-byte identifier, the declared payload size, and the offset
        where the payload begins.

    Raises:
        ValueError: If the header runs past `end`, or the payload it declares
            reaches past the bytes the file actually holds. A playlist is
            judged against what is really there, not against what a chunk
            says about itself.
    """
    if offset + 8 > end:
        raise ValueError("a chunk header runs past the end of its container")
    chunk_id = data[offset : offset + 4]
    (size,) = struct.unpack_from("<I", data, offset + 4)
    body = offset + 8
    if body + size > len(data):
        raise ValueError("a chunk declares more data than the file holds")
    return chunk_id, size, body


def _decode_marker(raw: bytes) -> Marker:
    """Build a [`Marker`][tools.mmp_dump.Marker] from one `TRKM` payload."""
    lane, position_us = struct.unpack_from("<4xIQ", raw, 0)
    interpolation, value = struct.unpack_from("<If", raw, 16)
    return Marker(lane=lane, position_us=position_us, value=value, interpolation=interpolation)


def _decode_span(raw: bytes) -> Span:
    """Build a [`Span`][tools.mmp_dump.Span] from one `TRKS` payload."""
    start_us, end_us, source_us = struct.unpack_from("<8xQQQ", raw, 0)
    return Span(start_us=start_us, end_us=end_us, source_us=source_us)


def _read_track(
    data: bytes, start: int, end: int, track: Track, list_type: bytes, depth: int = 0
) -> None:
    """Walk one track's chunks, filling in `track` as each is recognised.

    Args:
        data: The whole file.
        start: Offset of the first chunk to read.
        end: Offset just past the last chunk to read.
        track: Track being filled in.
        list_type: Type of the enclosing `LIST`, which decides whether markers
            found here belong to the beat grid or to the automation lanes.
        depth: How many containers deep this call is nested inside the track.

    Raises:
        ValueError: If a chunk's header or declared size does not fit the
            bytes the file holds, or if containers nest past
            [`MAX_NESTING_DEPTH`][tools.mmp_dump.MAX_NESTING_DEPTH].
    """
    if depth > MAX_NESTING_DEPTH:
        raise ValueError("chunks nest deeper than a real playlist ever does")
    offset = start
    while offset + 8 <= end:
        chunk_id, size, body = _chunk_header(data, offset, end)
        raw = data[body : body + size]

        if chunk_id in CONTAINER_IDS:
            _read_track(
                data, body + 4, min(body + size, end), track, data[body : body + 4], depth + 1
            )
        elif chunk_id == b"TRKF":
            track.path = _decode_text(raw)
        elif chunk_id == b"TRSI":
            track.transition_in = _decode_text(raw)
        elif chunk_id == b"TRSO":
            track.transition_out = _decode_text(raw)
        elif chunk_id == b"TRKH" and size >= 32:
            track.identity = raw[8:24].hex()
            (millibeats,) = struct.unpack_from("<Q", raw, 24)
            track.original_bpm = millibeats / 1000
        elif chunk_id == b"TKTH" and size >= 4:
            (track.bar_period_us,) = struct.unpack_from("<I", raw, 0)
        elif chunk_id == b"TRKM" and size >= MARKER_SIZE:
            marker = _decode_marker(raw)
            if list_type == b"TKTM":
                track.beatgrid.append(marker)
            else:
                track.automation.append(marker)
        elif chunk_id == b"TRKS" and size >= MARKER_SIZE:
            track.spans.append(_decode_span(raw))

        offset = body + size + (size & 1)


def read_playlist(path: str | Path) -> Playlist:
    """Read a MixMeister playlist from disk.

    Args:
        path: Path to a `.mmp` file.

    Returns:
        The parsed playlist.

    Raises:
        ValueError: If the file is larger than
            [`MAX_FILE_SIZE`][tools.mmp_dump.MAX_FILE_SIZE], if it is not a
            RIFF container of form type `MXMP`, or if a chunk inside it is
            damaged: its header or its declared size does not fit the bytes
            the file holds, or containers nest too deep.

    Examples:
        ```python
        playlist = read_playlist("sample playlist.mmp")
        for track in playlist.tracks:
            print(track.path, track.original_bpm)
        ```
    """
    path = Path(path)
    size_on_disk = path.stat().st_size
    if size_on_disk > MAX_FILE_SIZE:
        raise ValueError(
            f"{path} is {size_on_disk} bytes, larger than a MixMeister playlist "
            f"ever is (the limit is {MAX_FILE_SIZE} bytes)"
        )
    data = path.read_bytes()
    if data[:4] != b"RIFF" or data[8:12] != b"MXMP":
        raise ValueError(f"{path} is not a MixMeister playlist (expected a RIFF file of form MXMP)")

    playlist = Playlist()
    try:
        offset, end = 12, len(data)
        while offset + 8 <= end:
            chunk_id, size, body = _chunk_header(data, offset, end)

            if chunk_id == b"LIST" and data[body : body + 4] == b"TRKL":
                inner, inner_end = body + 4, min(body + size, end)
                while inner + 8 <= inner_end:
                    inner_id, inner_size, inner_body = _chunk_header(data, inner, inner_end)
                    if inner_id == b"LIST" and data[inner_body : inner_body + 4] == b"TRKI":
                        track = Track()
                        _read_track(
                            data, inner_body + 4, min(inner_body + inner_size, inner_end), track, b"TRKI"
                        )
                        playlist.tracks.append(track)
                    inner = inner_body + inner_size + (inner_size & 1)
            elif chunk_id == b"MMVR":
                count = size // 2
                playlist.version = struct.unpack_from(f"<{count}H", data, body)

            offset = body + size + (size & 1)
    except ValueError as error:
        raise ValueError(f"{path} is a damaged MixMeister playlist ({error})") from error

    return playlist


def format_playlist(playlist: Playlist) -> str:
    """Render a playlist as readable text.

    Args:
        playlist: A playlist from [`read_playlist`][tools.mmp_dump.read_playlist].

    Returns:
        A multi-line summary, one block per track.
    """
    lines: list[str] = []
    if playlist.version:
        lines.append(f"file version: {'.'.join(str(part) for part in playlist.version)}")
    lines.append(f"{len(playlist.tracks)} tracks")

    for number, track in enumerate(playlist.tracks, start=1):
        lines.append("")
        lines.append(f"track {number}: {_visible(track.path) if track.path else '(no path)'}")
        lines.append(
            f"  original BPM {track.original_bpm:.3f}"
            f"   bar period implies {track.bar_period_bpm:.2f} BPM"
        )
        if track.transition_in:
            lines.append(f"  transition in:  {_visible(track.transition_in)}")
        if track.transition_out:
            lines.append(f"  transition out: {_visible(track.transition_out)}")
        lines.append(f"  {len(track.beatgrid)} beatgrid markers, {len(track.spans)} spans")

        for marker in track.automation:
            lines.append(
                f"    {marker.position_seconds:9.3f}s  {marker.lane_name:<16}"
                f"  value {marker.value:9.3f}  interpolation {marker.interpolation}"
            )

    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    """Command line entry point.

    Args:
        argv: Arguments to parse. Defaults to `sys.argv[1:]`.

    Returns:
        A process exit status.
    """
    parser = argparse.ArgumentParser(description="Print the contents of a MixMeister .mmp playlist.")
    parser.add_argument("path", help="the .mmp file to read")
    parser.add_argument("--json", action="store_true", help="print JSON instead of readable text")
    args = parser.parse_args(argv)

    try:
        playlist = read_playlist(args.path)
    except (OSError, ValueError) as error:
        print(error, file=sys.stderr)
        return 1

    if args.json:
        print(json.dumps(dataclasses.asdict(playlist), indent=2))
    else:
        print(format_playlist(playlist))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
