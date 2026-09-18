# Golden renders

A golden render is the audio Dermixen produced for a fixed document at a moment when a person had listened to it and accepted it. Tests render the same document again and compare the result with the stored file within a small tolerance, so a change that alters the sound of a render is caught even when every unit test still passes.

Each file here is a 16-bit stereo WAV at 44.1 kHz, kept short so the repository stays small, and each has an entry below saying which test renders it and what it holds.

To replace a golden file after a deliberate change, run its test with the environment variable `DERMIXEN_UPDATE_GOLDEN` set. The test then writes the file instead of comparing against it. A replaced golden file is a change to what the app sounds like and belongs in its own pull request, with a note saying that someone listened to the new render and what they heard.

| File | Rendered by | Holds |
| --- | --- | --- |
| `two-kicks.wav` | `crates/engine/tests/render.rs`, `the_golden_render_has_not_drifted` | Two six-second synthetic kick tracks at 130 and 140 BPM, the second entering at mix beat 4, joined by the default transition across two bars from mix beat 8, over which the tempo ramps from one to the other. About eight seconds. |
