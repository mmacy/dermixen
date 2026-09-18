"""Decode audio through the built `dermixen` command.

`track_profile.py` and `grid_check.py` both import this module so the two
programs read audio the same way the library does. The plain name
`decoding` resolves for either script because Python searches a script's
own directory before the rest of the path, and both scripts sit in the
same folder as `decoding.py`. `find_dermixen` locates the executable and
`decode` runs it to read a span of a file as mono samples, the way the
library and a render read the same file, so the tools measure the audio
the library analyzed.
"""

import os
import shutil
import subprocess
import sys
import tempfile
import wave

import numpy as np

# The sample rate the whole project is fixed at, matching `SAMPLE_RATE` in
# `crates/core/src/units.rs`. `dermixen decode` writes every WAV at this rate
# whatever rate the source file is stored at.
SAMPLE_RATE = 44100


def find_dermixen() -> str:
    """Find the `dermixen` executable that decodes a track's audio.

    The `DERMIXEN` environment variable names the executable when it is set.
    Otherwise the function looks for `dermixen` on the path, then for
    `target/release/dermixen` and `target/debug/dermixen` under the
    repository root, which is the parent of the `tools/` folder this module
    lives in.

    Returns:
        The path to the executable.

    Raises:
        SystemExit: If `DERMIXEN` names a file that does not exist, or if
            `dermixen` is not built and not on the path.
    """
    named = os.environ.get("DERMIXEN")
    if named:
        if not os.path.isfile(named):
            sys.exit(f"error: DERMIXEN names {named}, which does not exist")
        return named
    found = shutil.which("dermixen")
    if found is not None:
        return found
    repository = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    for profile in ("release", "debug"):
        candidate = os.path.join(repository, "target", profile, "dermixen")
        if os.path.isfile(candidate):
            return candidate
    sys.exit(
        "error: dermixen is not built. Run cargo build --release -p dermixen-cli, "
        "or name the executable in DERMIXEN"
    )


def decode(path: str, start: float, length: float | None) -> np.ndarray:
    """Decode part of an audio file to mono samples at the project's sample rate.

    The function runs `dermixen decode PATH OUT --from START --for LENGTH`,
    with `--for` left off when `length` is `None`, reads `OUT` with the
    `wave` module, and averages its two channels. `OUT` is a temporary WAV
    under the system's temporary folder, removed after reading whether or
    not the command succeeded.

    The command line does not take a negative start. A caller can still ask for
    one, since a search that walks back from a beat can reach before the
    file starts, so a negative start is decoded from zero instead and the
    seconds it would have covered before the file started come back as
    silence, keeping the returned array as long as the span asked for. A
    span that ends at or before the file's start, `start + length <= 0`,
    is silence from end to end, and the function returns it without
    running the command at all.

    Args:
        path: The audio file, passed to the command exactly as given.
        start: Where to start, in seconds from the beginning of the file.
            May be negative.
        length: How many seconds to decode, or `None` for the rest of the
            file.

    Returns:
        The samples, as floats between -1 and 1.

    Raises:
        SystemExit: If `dermixen` cannot be found, cannot be run, or the
            decode fails.
    """
    if length is not None and start + length <= 0.0:
        return np.zeros(int(round(length * SAMPLE_RATE)))
    executable = find_dermixen()
    silence_seconds = 0.0
    call_start = start
    call_length = length
    if call_start < 0.0:
        silence_seconds = -call_start
        call_start = 0.0
        if call_length is not None:
            call_length = length + start
    with tempfile.TemporaryDirectory() as folder:
        out = os.path.join(folder, "decoded.wav")
        command = [executable, "decode", path, out, "--from", f"{call_start:.6f}"]
        if call_length is not None:
            command += ["--for", f"{call_length:.6f}"]
        try:
            done = subprocess.run(command, capture_output=True)
        except OSError as problem:
            named = os.environ.get("DERMIXEN")
            if named and named == executable:
                sys.exit(f"error: DERMIXEN names {named}, which cannot be run: {problem}")
            sys.exit(f"error: cannot run {executable}: {problem}")
        if done.returncode != 0:
            reason = done.stderr.decode(errors="replace").strip() or "dermixen gave no reason"
            sys.exit(f"error: cannot decode {path}: {reason}")
        with wave.open(out, "rb") as opened:
            channels = opened.getnchannels()
            frames = opened.readframes(opened.getnframes())
    samples = np.frombuffer(frames, np.int16).astype(np.float64) / 32768.0
    if channels == 2:
        samples = samples.reshape(-1, 2).mean(axis=1)
    pad = int(round(silence_seconds * SAMPLE_RATE))
    if pad:
        samples = np.concatenate([np.zeros(pad), samples])
    return samples
