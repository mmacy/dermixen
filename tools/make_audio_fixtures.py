"""Generate the synthetic audio fixtures under tests/fixtures/audio/.

The fixtures are sine tones with exactly known contents, written as WAV by
this script and then encoded to MP3, FLAC, and AAC in an MP4 container with
the `lame`, `flac`, and `ffmpeg` programs. Tests compare what Dermixen
decodes against the numbers this script prints, so rerun it and update the
tests together if anything here changes.

Run from the repository root:

    python3 tools/make_audio_fixtures.py
"""

import math
import pathlib
import struct
import subprocess
import wave
import zlib

OUT = pathlib.Path("tests/fixtures/audio")


def sine_frames(rate: int, seconds: float, frequency: float, amplitudes: list[float]) -> bytes:
    """Interleaved 16-bit frames of a sine at `frequency`, one channel per amplitude."""
    count = round(rate * seconds)
    out = bytearray()
    for n in range(count):
        value = math.sin(2 * math.pi * frequency * n / rate)
        for amplitude in amplitudes:
            out += struct.pack("<h", round(value * amplitude * 32767))
    return bytes(out)


def write_wav(name: str, rate: int, channels: int, frames: bytes) -> None:
    path = OUT / name
    with wave.open(str(path), "wb") as handle:
        handle.setnchannels(channels)
        handle.setsampwidth(2)
        handle.setframerate(rate)
        handle.writeframes(frames)
    count = len(frames) // (2 * channels)
    print(f"{name}: {count} frames, {rate} Hz, {channels} channel(s), crc32 of samples {zlib.crc32(frames):08x}")


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    stereo_44k = sine_frames(44100, 2.0, 440.0, [0.5, 0.25])
    write_wav("sine-440-44k.wav", 44100, 2, stereo_44k)
    write_wav("sine-440-48k.wav", 48000, 2, sine_frames(48000, 2.0, 440.0, [0.5, 0.25]))
    write_wav("sine-220-mono-44k.wav", 44100, 1, sine_frames(44100, 1.0, 220.0, [0.5]))
    subprocess.run(
        ["lame", "--quiet", "-b", "192", "--cbr", str(OUT / "sine-440-44k.wav"), str(OUT / "sine-440-44k.mp3")],
        check=True,
    )
    subprocess.run(
        ["flac", "--silent", "--force", "-o", str(OUT / "sine-440-44k.flac"), str(OUT / "sine-440-44k.wav")],
        check=True,
    )
    # Tagged copies of the tone, for the tag reader. The MP3 gets ID3 tags
    # from lame and the FLAC gets Vorbis comments from flac; a third file has
    # an artist and a title made of spaces, which the reader must treat as absent.
    artist = "Slinky Wizard"
    title = "Lunar Juice (Hallucinogen Moon Strudel Remix)"
    subprocess.run(
        ["lame", "--quiet", "-b", "128", "--cbr", "--id3v2-only",
         "--tt", title, "--ta", artist, "--ty", "1996",
         str(OUT / "sine-440-44k.wav"), str(OUT / "tagged.mp3")],
        check=True,
    )
    subprocess.run(
        ["lame", "--quiet", "-b", "128", "--cbr", "--id3v2-only",
         "--tt", "   ", "--ta", artist,
         str(OUT / "sine-440-44k.wav"), str(OUT / "tagged-blank.mp3")],
        check=True,
    )
    subprocess.run(
        ["flac", "--silent", "--force", f"-TARTIST={artist}", f"-TTITLE={title}", "-TDATE=1996",
         "-o", str(OUT / "tagged.flac"), str(OUT / "sine-440-44k.wav")],
        check=True,
    )
    # The MP4 pair: the tone as AAC, and the same with iTunes-style tags.
    subprocess.run(
        ["ffmpeg", "-loglevel", "error", "-y", "-i", str(OUT / "sine-440-44k.wav"),
         "-c:a", "aac", "-b:a", "128k", str(OUT / "sine-440-44k.m4a")],
        check=True,
    )
    subprocess.run(
        ["ffmpeg", "-loglevel", "error", "-y", "-i", str(OUT / "sine-440-44k.wav"),
         "-c:a", "aac", "-b:a", "128k",
         "-metadata", f"artist={artist}", "-metadata", f"title={title}", "-metadata", "date=1996",
         str(OUT / "tagged.m4a")],
        check=True,
    )
    for name in [
        "sine-440-44k.mp3", "sine-440-44k.flac", "sine-440-44k.m4a",
        "tagged.mp3", "tagged-blank.mp3", "tagged.flac", "tagged.m4a",
    ]:
        print(f"{name}: {(OUT / name).stat().st_size} bytes")


if __name__ == "__main__":
    main()
