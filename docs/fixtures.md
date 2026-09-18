# Test fixtures

Tests need audio, and audio is large, licensed, and slow to analyze. Dermixen uses three tiers, and every test says which tier it belongs to by how it gets its input.

## Synthetic audio, generated when the test runs

Sine tones, click tracks, white noise, and silence are generated in memory by the `dermixen-testkit` crate at the moment a test needs them. Nothing is stored. This tier is the default: it is exact, fast, license-free, and identical on every machine, which makes it the right input for round-trip tests, arithmetic, the render graph, and golden renders. Two synthetic kick tracks at different tempos are enough to prove that beats line up.

## Short real clips, kept in the repository

Some things cannot be tested on a sine wave: decoding a real MP3 written by a real encoder, reading tags, or checking that a beat analyzer finds a beat in music. For those, audio files go under `tests/fixtures/audio/`, and the README there lists each file with its source and why it may be committed. The directory contains synthetic tones whose exact contents the decode tests know. Real clips of ten to twenty seconds go there when a test needs music, and they stay small and few so that adding one is a deliberate act.

Four files in that directory hold no music, and they are what the decoder's limits are tested against. `silence-91-minutes.flac` and `silence-91-minutes-no-total.flac` are each 64 KB of digital silence that decodes to 91 minutes, which is past the 90 minutes a track may be. The first states its length in its header and the second states none, so the decoder refuses the first before it decodes anything and refuses the second as soon as its packets pass 90 minutes. `tools/make_long_flac.py` writes both, and its `--rate` option writes the same silence at another sample rate, which is how the memory a decode of a high-rate file takes is measured. `wav-65535-channels.wav` states 65,535 channels in its header and `m4a-huge-sample-count.m4a` states 4,294,967,295 samples in its `stts` box. Each of those two numbers reaches arithmetic in symphonia 0.6.1 that overflows, so a decode of either file has to come back as an error rather than end the process.

## Whole tracks from a local music collection

Analysis quality is measured on whole tracks, and those tracks are not in the repository. A test that needs them calls `dermixen_testkit::library_root()`, which reads the environment variable `DERMIXEN_LIBRARY`. When the variable is set and points at a directory, the test runs against the files there. When the variable is unset, the test returns early and passes, so continuous integration and other machines are never blocked by files they do not have.

To run these tests, point the variable at a local music collection:

```sh
DERMIXEN_LIBRARY=/Volumes/goa cargo test --workspace
```

Two more fixture directories back particular suites. `tests/fixtures/mix/` contains one valid project file in `valid/`, twelve malformed ones in `invalid/`, and eighteen in `out-of-range/` that are well formed and hold a number outside the limits `DESIGN.md` states, such as a tempo of 19.999 or a playlist that lays out as 36.1 hours of mix. Each refused file is paired with the error it must produce, which is how the project file's error contract stays pinned. `tests/fixtures/golden/` contains the golden renders its own README describes: audio the render tests compare against, replaced only deliberately.

The MixMeister project files under `tests/fixtures/mmp/` are the one exception to the size rule for real material: they are small, and `tools/anchor_truth.py` builds the anchor ground truth in `tests/ground-truth/anchors/` from them. The README beside them lists each file with its track count and tempo range.
