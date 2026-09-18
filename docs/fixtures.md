# Test fixtures

Tests need audio, and audio is large, licensed, and slow to analyze. Dermixen uses three tiers, and every test says which tier it belongs to by how it gets its input.

## Synthetic audio, generated when the test runs

Sine tones, click tracks, white noise, and silence are generated in memory by the `dermixen-testkit` crate at the moment a test needs them. Nothing is stored. This tier is the default: it is exact, fast, license-free, and identical on every machine, which makes it the right input for round-trip tests, arithmetic, the render graph, and golden renders. Two synthetic kick tracks at different tempos are enough to prove that beats line up.

## Short real clips, kept in the repository

Some things cannot be tested on a sine wave: decoding a real MP3 written by a real encoder, reading tags, or checking that a beat analyzer finds a beat in music. For those, audio files go under `tests/fixtures/audio/`, and the README there lists each file with its source and why it may be committed. The directory contains synthetic tones whose exact contents the decode tests know. Real clips of ten to twenty seconds go there when a test needs music, and they stay small and few so that adding one is a deliberate act.

## Whole tracks from a local music collection

Analysis quality is measured on whole tracks, and those tracks are not in the repository. A test that needs them calls `dermixen_testkit::library_root()`, which reads the environment variable `DERMIXEN_LIBRARY`. When the variable is set and points at a directory, the test runs against the files there. When the variable is unset, the test returns early and passes, so continuous integration and other machines are never blocked by files they do not have.

To run these tests, point the variable at a local music collection:

```
DERMIXEN_LIBRARY=/Volumes/goa cargo test --workspace
```

Two more fixture directories back particular suites. `tests/fixtures/mix/` contains one valid project file and twelve invalid ones, each invalid file paired with the error it must produce, which is how the project file's error contract stays pinned. `tests/fixtures/golden/` contains the golden renders its own README describes: audio the render tests compare against, replaced only deliberately.

The MixMeister project files under `tests/fixtures/mmp/` are the one exception to the size rule for real material: they are small, and `tools/anchor_truth.py` builds the anchor ground truth in `tests/ground-truth/anchors/` from them. The README beside them lists each file with its track count and tempo range.
