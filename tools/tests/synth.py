"""Synthetic audio for the tool tests, written as WAV files.

A kick is a burst whose pitch sweeps from 160 Hz down to 50 Hz over its
first hundred milliseconds under a short click, and a bass note holds one
pitch. A track is a kick on every beat with a bass note half a beat after
each one, so a grid placed on the bass note is half a beat from the kick,
which is the mistake the tools have to see.
"""

import wave

import numpy as np

SAMPLE_RATE = 44100


def kick(length: float = 0.25) -> np.ndarray:
    """One kick: a pitch sweep from 160 Hz to 50 Hz that decays, under a 2 ms click."""
    count = int(SAMPLE_RATE * length)
    t = np.arange(count) / SAMPLE_RATE
    frequency = 50.0 + 110.0 * np.exp(-t / 0.04)
    phase = 2.0 * np.pi * np.cumsum(frequency) / SAMPLE_RATE
    body = np.sin(phase) * np.exp(-t / 0.09)
    click = np.random.default_rng(1).uniform(-1.0, 1.0, count) * np.exp(-t / 0.002) * 0.6
    return body * 0.8 + click


def bass(frequency: float = 70.0, length: float = 0.16) -> np.ndarray:
    """One bass note that holds `frequency`, with a 5 ms attack and a 20 ms release."""
    count = int(SAMPLE_RATE * length)
    t = np.arange(count) / SAMPLE_RATE
    envelope = np.minimum(1.0, t / 0.005) * np.where(t > length - 0.02, (length - t) / 0.02, 1.0)
    return np.sin(2.0 * np.pi * frequency * t) * envelope * 0.5


def add(out: np.ndarray, piece: np.ndarray, at: float, gain: float = 1.0) -> None:
    """Mix `piece` into `out` starting `at` seconds in."""
    start = int(round(at * SAMPLE_RATE))
    if start >= len(out):
        return
    end = min(len(out), start + len(piece))
    out[start:end] += piece[: end - start] * gain


def track(
    bpm: float,
    seconds: float,
    first_kick: float,
    kick_beats=None,
    bass_beats=None,
    bass_offset: float = 0.5,
    gain=None,
) -> np.ndarray:
    """A kick on every beat from `first_kick` and a bass note `bass_offset` beats after each.

    `kick_beats` and `bass_beats` are predicates on the beat index that say
    whether that beat gets its kick or its bass note, and `gain` is a function
    of the beat index giving a linear gain for everything on that beat. All
    three default to a kick, a bass note, and unity on every beat.
    """
    out = np.zeros(int(SAMPLE_RATE * seconds))
    beat = 60.0 / bpm
    one_kick = kick()
    one_bass = bass()
    index = 0
    while True:
        at = first_kick + index * beat
        if at >= seconds:
            break
        level = gain(index) if gain else 1.0
        if kick_beats is None or kick_beats(index):
            add(out, one_kick, at, level)
        if bass_beats is None or bass_beats(index):
            add(out, one_bass, at + bass_offset * beat, level)
        index += 1
    return out


def write_wav(path, samples: np.ndarray) -> None:
    """Write mono 16-bit samples at the project's sample rate."""
    data = (np.clip(samples, -1.0, 1.0) * 32767).astype("<i2").tobytes()
    with wave.open(str(path), "wb") as handle:
        handle.setnchannels(1)
        handle.setsampwidth(2)
        handle.setframerate(SAMPLE_RATE)
        handle.writeframes(data)
