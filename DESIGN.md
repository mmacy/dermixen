# Dermixen design

## What this is

Dermixen is a desktop app for macOS and Linux for building a continuous DJ mix in the studio: drag analyzed tracks onto a timeline, get automatic beatmatched transitions, adjust them, and render the whole thing to a single audio file. It is inspired by MixMeister and is a studio tool, not a live performance tool. It has two front doors (a GUI and a CLI), both consumers of the same engine (see "The CLI is a product surface").

The two values that outrank everything else, including features:

1. **Bulletproof.** It never crashes, never loses work, never renders music different from what you heard in preview (sample for sample for a track without keylock, and the same beats at the same levels for a keylocked track, as "Architecture" explains), and never claims confidence it doesn't have.
2. **Easy.** A former MixMeister user should feel at home in minutes. No frills.

The core mental model: a mix is a **document**, an ordered list of track references, one tempo curve shared by the whole mix, and per-track volume and EQ envelopes. Rendering the document is deterministic. That determinism is what makes "bulletproof" testable rather than aspirational.

## Non-goals

These are deliberate exclusions, not omissions. Do not add them without revisiting this document:

- No live performance features and no built-in "auto" DJ mode. Automated mix *assembly* is in scope, but the intelligence lives outside the app in an agent driving the CLI (see "The CLI is a product surface").
- No MIDI support
- No VST or any other plugin support
- No effects beyond the per-track 3-band EQ described in the feature kernel: no adjustable-frequency filters, no reverb, delay, or any other processing. Tonal correction and mastering belong to pre-processing and post-processing outside the app. Volume and EQ envelopes are the only per-clip processing.
- No variable-tempo source support. Analysis assumes each track holds one constant tempo, and a track whose tempo drifts is flagged rather than accommodated: there is no facility for pinning a moving beatgrid down at several points across a track. This restriction covers source material only. The mix tempo curve on the timeline varies freely, and the intro and outro anchors in the feature kernel are an unrelated feature that has nothing to do with tempo drift.
- No master volume or any other control over the combined output. Final level and limiting belong to post-processing outside the app, alongside the tonal correction and mastering already excluded above.
- No OGG or raw AAC stream files, no streaming service integration, no video

## Supported formats

Import: WAV, MP3, FLAC, and MP4/M4A (which contain AAC-LC audio, that's what stores and rippers produce). All four formats are handled by the `symphonia` pure-Rust decoding library, so the entire import list is one dependency with no ffmpeg.

Export: WAV and MP3 only. AAC encoding is a patent-licensing swamp and unnecessary for a finished mix. WAV is written at 16-bit/44.1 kHz, which is the format of nearly every source. MP3 is written at 320 kbps CBR.

Internal processing format: 32-bit float stereo at 44.1 kHz, chosen because nearly all dance music sources are 44.1 kHz, so almost nothing gets resampled on the way in. All timeline math is done in samples at this rate.

## Feature kernel (version 1)

- **Library** with background analysis: BPM, beatgrid, musical key, loudness, duration, and the effective beginning and ending the app uses to place anchors, plus tag metadata (artist, title, year) read from ID3 and MP4 tags, falling back to best-effort filename parsing ("Artist - Title" patterns) when tags are missing, flagged as low-confidence so queries and the UI can treat guessed metadata differently from real tags. The year means the year the music was first released. The tags of a reissue state the pressing's year instead, so the library keeps a year that a Discogs lookup has corrected, and a year that is an estimate from the years of a release's other tracks is marked as one: a query can leave estimated years out, and the UI shows an estimate as an estimate. The library also stores the release each track came from as Discogs identifies it (the label, the catalog number, the release title, and the track number). No scan fills the release or corrects the year, because a tag names the pressing a file was ripped from. The two Discogs programs in `tools/` do, and the library keeps what they wrote across every later scan. The scanner writes everything to a queryable SQLite index shared by the GUI and the CLI, keyed by content hash so re-adding or moving files never re-analyzes. The window's library panel is a table with one row per track and a column for each of the artist, the title, the year, the tempo, the key, the label, the catalog number, the release title, the track number, and the length. A click on a column's heading sorts the rows by that column, and a second click reverses the order. A row with no value in the sorted column comes after every row with one, whichever way the sort runs. The panel narrows the rows by a filter on each of the artist, the title, the key (any set of the 24 Camelot codes), the label, a year range, a tempo range, and the file path, and by one free text: a track matches the text when every word of the text is found in the track's artist, title, year, tempo, Camelot code, label, catalog number, release title, track number, or file path. One control at the right end of the row of controls above the timeline, drawn as other applications draw a sidebar toggle, hides the panel, so the timeline has the whole width of the window once the tracks are chosen, and shows it again from the same place. Whether the panel is hidden is a setting, so the window opens with the panel as the person left it. Whether a cell's text wraps onto further lines is a setting too, off by default, so a row is one line unless the person asks for more. A double-click on the divider after a column's heading fits the column to its widest cell, the heading included, as a spreadsheet does. Which library file the window opens is a setting too, so the window opens the library the person last chose, and the `library` commands and `mix add` open the same file. When the window opens a document and that library file is not there, the window makes it and, when the music folder exists, scans the music folder into it on a thread while the window stays responsive, so the first launch after a build or an install ends with a library. The music folder is a setting whose default is `~/Music/Undefunktis`, which is where `scripts/install-starter-tracks.sh` puts the five starter tracks, so a new library has something in it. **Library > Scan music folder** runs the same scan whenever the person asks, the status line shows each file as it is dealt with and the summary at the end, the panel gains the tracks as they land, and **Library > Stop scan** ends the scan after the file it is on.
- **Timeline that is also the playlist** (MixMeister's best UX decision). Reordering the playlist reorders the mix. A person drags the bottom edge of any lane of the timeline, a track's or the tempo curve's, to make the lane taller or shorter, so that a waveform is tall enough for frame-level work when the mix has many tracks and the view is zoomed all the way in, and double-clicks that edge to make the lane as tall as it goes, or as short, when it is already as tall. The wheel over the timeline scrolls the lanes up and down, the command or the alt key with the wheel zooms around the pointer, and the command or the alt key with shift and the wheel scrolls in time, as a trackpad's two-finger sideways swipe does on its own.
- **One tempo curve across the whole mix**: a single tempo curve runs the length of the timeline, and every track plays at whatever tempo that curve specifies at that point. The first track in the playlist sets the starting tempo. Each track also keeps the original BPM found during analysis, which never changes. The engine stretches a track by the ratio of the mix tempo to its original BPM, and the playlist shows both numbers, the original BPM alongside the applied stretch as a percentage. Because one curve governs every track, overlapping tracks cannot drift apart. Beat alignment is structural, not something the user has to maintain.
- **Editing the tempo curve**: click the curve beneath the timeline to add a node, then type an exact BPM or drag the node. A master BPM control above the timeline changes the tempo of the mix from the playhead on during playback and writes nodes onto the curve as it does so, which keeps every change visible and editable afterwards. What the playhead has already passed keeps the tempo it was heard at. To end a tempo excursion, place a second node at the tempo the mix should return to. Editing tempo never breaks beat alignment, because the engine re-stretches every affected track to keep the beats lined up. Removing the nodes over a stretch of the mix returns that stretch to the tempo the automatic transitions chose.
- **Intro and outro anchors** are how the app places overlaps. Every track gets one anchor near its start and one near its end, both worked out once during analysis and stored with the track rather than with the mix. Each mix keeps its own copy of the pair, which is what dragging changes, so moving an anchor in one mix leaves every other mix alone. Analysis finds where the music effectively begins and ends, so that lead-in silence and trailing tails fall outside, then places each anchor on a beat at a musically sensible point inside that span. The stretch of track a transition works across depends on the preset: the preset's bars from the anchor for the beatmix and the bass swap, eight unless another length is given, and for the blend the eight bars before the incoming track's intro anchor and the twenty-eight after it, together with the outgoing track's outro anchor to its last sample. Dermixen lines the outgoing track's outro anchor up with the incoming track's intro anchor, and the app derives the overlap from that single alignment. Dragging either anchor slides the overlap and leaves the pair still aligned: moving the intro anchor later, or the outro anchor earlier, lengthens the overlap, and moving them the other way shortens it. Anchors sit on beats, and the tempo, volume, and EQ nodes belonging to a transition move with them. Dragging the outro anchor past the end of its track leaves silence between the two songs.
- **Automatic beatmatched transitions**: across the overlap the anchors define, the transition generator writes tempo curve nodes that bring both tracks to a common tempo so their beats line up, then lets the curve settle at the incoming track's own tempo. Over a whole mix the curve therefore follows each song in turn rather than holding one number, and each track stretches only as far as its neighbors require. A transition is nothing more than those tempo nodes plus the two tracks' overlapping volume and EQ envelopes, with nodes that snap to beats, every one editable after the fact. The beatmix and bass-swap presets have an overlap length in bars, which is what the app uses to position the anchors when someone applies one of them. The default preset, the blend, has one fixed shape instead: the incoming track rises from silence eight bars before its intro anchor to full level twenty-eight bars after it, and the outgoing track plays to its last sample, easing down rather than fading out, so the incoming track is established before the outgoing one leaves and the outgoing track ends where its file does.
- **Per-track volume and EQ curves with one editing model**: every track has its own volume curve and its own low, mid, and high EQ curves on the timeline. Tempo is not among them: tempo belongs to the mix, not to the track. Editing works the same way for all four curve types: select the curve type, click the track to place the first node, click the curve's line to add more nodes, then drag a node horizontally to move it in time or vertically to change its level.
- **Per-track 3-band EQ envelopes**: a DJ-mixer-style channel strip (low, mid, and high bands at fixed frequencies, each with full-kill range) automated with the same node-based envelope facility as volume. This is what makes bass-swap transitions possible: cut the incoming track's lows until the swap point, then hand the bassline over. It is the only effect in the app (see non-goals).
- **Key detection and harmonic mixing**: every track gets a Camelot wheel code (8A, 8B). Selecting a track highlights harmonically compatible tracks in the library. Guiding principle: consistency beats correctness. If two tracks that mix well get compatible codes, the label being musicologically debatable doesn't matter.
- **Volume leveling** across tracks via EBU R128 loudness analysis, so quiet masters and loud masters sit at the same perceived level. The gain each track needs is written into the mix document as `gain_db` when the track joins the mix, so a render depends on the document alone. The target is minus fourteen LUFS, and no track is raised past a true peak of minus one dBTP.
- **Manual beatgrid correction** as a UI: drag the grid over the track's own waveform, down to the sample, with ×2 and ÷2 buttons for half/double-tempo errors and tap tempo. The grid is judged by ear against the track itself: the editor auditions the track on its own, at its own speed, with a metronome that clicks on every beat of the grid and that the user turns on and off at will, so a grid dragged while the track plays is heard to land on the kicks. Every adjustment takes effect on the track as it is made, the left and right arrow keys move the grid by one sample (ten with shift, a hundred with shift and the command or alt key), a click on the waveform plays the track from that point, Space plays and stops the track from the editor's playhead, and the whole correction is one undoable step in the mix's history. While the editor is open, the command key with Z takes back the last single adjustment and leaves the editor open, and the command key with shift and Z makes it again, so a nudge too far costs one key press rather than the whole correction. A person drags the bottom edge of the editor's strip, the waveform with the grid drawn over it, to make the strip taller or shorter, and collapses the strip to the row of controls and expands it again, with the collapsed state kept as a setting. No detector is perfect, and adjustment after analysis is the rule rather than the exception. The fix must be pleasant.
- **Per-track keylock toggle**: choose between pitch-preserving time-stretch and vinyl-style resampling per track.
- **Opening and saving**: the window opens with an untitled empty mix when it is given no file, and on the mix in the file when it is given one, whether from the command line, from `dermixen open`, or from a `.dmx` file opened on the desktop. A menu bar offers **File** with **New**, **Open**, **Save**, **Save as**, and **Quit**, **Edit** with **Undo**, **Redo**, and **Settings**, **View** with the zoom controls and the library panel, and **Library** with **Scan music folder** and **Stop scan**. Every item of **File**, **Edit**, and **View** has a keyboard shortcut, and the two **Library** items have none, since a scan is not something to start by a slip of the hand. **Open** and **Save as** use the operating system's own file dialogs. The title names the file, or `Untitled`, and says `(edited)` while the mix has changes that were never saved. Every menu item that would put another document under the window or close it first asks what should become of unsaved changes, with **Save**, **Don't save**, and **Cancel**, and a save that fails leaves the question standing.
- **Reliability plumbing**: autosave, crash recovery, and graceful relinking of moved/missing files. Project files are human-readable JSON, versioned, referencing audio by path plus content hash. An untitled mix is autosaved in the data folder and offered back the next time the window opens an untitled mix.

## Technology choices

Language is Rust, chosen over C++ for memory safety in service of "bulletproof." The architecture makes correctness observable from outside the code (see "Verification" below).

| Concern | Choice | Notes |
| --- | --- | --- |
| Decoding (WAV/MP3/MP4-AAC) | `symphonia` | Pure Rust, covers the whole import list |
| Audio output | `cpal` | CoreAudio on macOS, ALSA/PipeWire on Linux |
| WAV writing | `hound` | |
| MP3 encoding | LAME via Rust bindings | Patents expired |
| Loudness | `ebur128` | EBU R128 leveling |
| Time-stretch | Signalsmith Stretch via a small FFI shim | MIT-licensed, header-only C++, excellent at the small ratios beatmatching involves |
| Time-stretch fallback | Rubber Band R3 | GPL is acceptable (see "Licensing"), so this is freely available if Signalsmith shows artifacts on dense material |
| GUI | `egui` | Immediate-mode canvas suits a custom timeline/waveform UI |
| CLI | `clap` | |
| Tag metadata | `lofty` | ID3v2 and MP4 tags: artist, title, year |
| Library index | SQLite via `rusqlite` | One file, shared by CLI and GUI |
| Release identification | Discogs, through `tools/discogs_release.py` and `tools/discogs_year.py` | Outside the app. Both programs read the collection export first and ask the API only about what the export does not list |
| Analysis baselines | aubio and libkeyfinder via FFI | GPL, used as scoreboard baselines (see "Analysis strategy") |

The time-stretcher sits behind a trait so swapping implementations is a one-file change, not surgery.

## Architecture

The one non-negotiable rule: **the engine is a UI-free library and the GUI is a thin shell over it.** This is what makes the app testable, gives us a CLI renderer nearly for free, and lets the UI toolkit decision be revisited without touching audio code.

Cargo workspace:

- `crates/core`: the mix document model: track references, per-track intro and outro anchors, the mix tempo curve, per-track envelope math for volume and EQ, beat/bar arithmetic, and the settings file. Pure logic with no DSP dependencies. The settings file is the one file it reads and writes, so that the window and the CLI agree on every setting by sharing the code that reads it. It also has the two file helpers every other crate uses, a bounded read that refuses anything but a regular file and an atomic write, so that one piece of code decides how the app touches a file a person cares about.
- `crates/media`: decoding, encoding, and content hashing for file identity.
- `crates/analysis`: beatgrid, key, and loudness analysis, including confidence reporting. Hosts the scoreboard eval harness and the FFI-bound baseline libraries.
- `crates/library`: folder scanning, tag metadata, the SQLite index of analysis results, and the query interface used by both the CLI and the GUI.
- `crates/engine`: the render graph (per-track stretch driven by the ratio of the mix tempo to each track's original BPM, then EQ, then volume, then sum and leveling), both offline render and real-time preview through `cpal`. Preview and render share one code path. Preview is that path pulled in real time with generous lookahead buffering (a studio tool, so 100-200 ms of latency is fine, glitches are not). A preview that starts inside a track brings the track's stretcher and equalizer up from a run-in of half a second of the track's own audio rather than from the track's first frame, so the preview begins to sound within a fraction of a second at any depth into the mix. A track without keylock then previews exactly as it renders. A keylocked track previews as the same music (every beat within a millisecond and within a decibel of where the render puts it, every level within half a decibel of the render's, and every tone at its pitch), because the pitch-preserving stretcher accumulates phase from everything it has been fed and a run-in cannot reproduce that state sample for sample.
- `crates/cli`: a product surface, not scaffolding (see "The CLI is a product surface"). It doubles as the verification harness, and every engine capability ships with a CLI command in lockstep with (or ahead of) its GUI equivalent.
- `crates/app`: the egui shell.
- `crates/testkit`: test support shared by the other crates, like locating the reference library and generating synthetic audio. It is not part of the app.
- `crates/macos-documents-sys`: the documents macOS asks the app to open, like a `.dmx` file double-clicked in the Finder, received through the Objective-C runtime and handed to the window as paths. On other platforms it does nothing, and a `.dmx` file opened on the desktop reaches the window as its argument.

## The CLI is a product surface

The CLI is not scaffolding that gets discarded once the GUI exists. It is a way to use Dermixen, developed in lockstep with (or ahead of) the GUI. The workflow that motivates this:

> "Whip up a 90-minute mix of tracks in ~/audio/goa/comp from 1996, opening with Slinky Wizard - Lunar Juice (Hallucinogen Moon Strudel Remix)."

That sentence is addressed to an *agent* (Claude or similar), not to the CLI itself. The design principle that makes it work: **intelligence lives in the agent. The app provides deterministic primitives.** Dermixen does not ship a creative auto-DJ brain. It ships composable, boring, reliable operations (query the library, assemble a mix document, adjust it, render it) and the agent supplies the selection, ordering, and taste. This is also why the "no auto mode" non-goal survives intact: automation happens *through* the app, never *in* it, and the app stays deterministic.

Consequences:

- **Agents are a primary CLI persona.** Every command offers machine-readable JSON output behind a `--json` flag, meaningful exit codes, and progress on stderr. Human-friendly output is the default. Agent-friendly output is one flag away.
- **Checks over a proposed order are primitives too.** `mix plan` predicts the timeline `mix add` would build from a playlist and reports a tempo step over a limit, two keys that do not fit, and an artist on more than one track. It chooses nothing and writes nothing, so the selection and the ordering stay with the agent.
- **The project file is a public contract.** Versioned, human-readable JSON, described field by field in `docs/project-file.md`. Agents and humans may generate or edit it directly. The CLI validates it and says exactly what is wrong when it doesn't parse.
- **GUI handoff works both ways.** `dermixen open <mix>` launches the GUI on a CLI-built mix. Later (not version 1), the GUI notices external changes to the open file and offers to reload, so an agent can revise a mix while the user has it open.

An illustrative subset of the command surface, which `docs/cli.md` describes in full:

```text
dermixen library scan [~/audio/goa]
dermixen library query --path ~/audio/goa/comp --year 1996 --json
dermixen library query --bpm 142-143 --json
dermixen library find "slinky wizard lunar juice" --json
dermixen analyze <file> --json
dermixen mix new set.dmx
dermixen mix add set.dmx <track> [--position N]
dermixen render set.dmx set.wav
dermixen open set.dmx
```

An agent satisfying the request above would scan and query the library, choose an order using the BPM and Camelot data in the JSON results, build the mix file with `mix` commands (or write the project JSON directly), then either render straight to WAV or MP3, or hand off to the GUI with `open`.

A second supported workflow: the user hands the agent a **complete tracklisting** and the agent assembles that exact set. `library find` is the primitive that makes it work: fuzzy text matching over tags and parsed filenames, returning ranked candidates with confidence scores. The agent resolves each tracklist line to a file, reports any lines it couldn't resolve instead of guessing silently, and builds the mix in the given order.

## Settings

The choices a person makes once live in one settings file: `settings.toml` in `dermixen` below the user's configuration folder, or the file `DERMIXEN_SETTINGS_FILE` names. The window reads it when it opens and writes it when a setting changes, and the `dermixen` command reads the same file for the same settings, so the two front doors agree on every default and an agent driving the command gets what the person chose in the window. The file is plain text a person edits by hand. Every setting is optional and has a documented default, a missing file means every default, and a value that cannot be read or a name the app does not know is reported with its line and never replaced by a default or written over. `dermixen settings` shows and sets the same values the window's settings dialog does. The settings are the audio device's buffer size, which is the whole of the delay between a grid nudge and the ear, whether the grid editor's metronome is on, whether the grid editor's strip is collapsed, whether the library panel is hidden, whether the library panel's cells wrap their text, the music folder, whose default is `Music/Undefunktis` below the user's home folder, and the library file, whose default is `dermixen/library.sqlite` below the user's data folder (`~/Library/Application Support` on macOS and `~/.local/share` on Linux). A relative path in either of the two path settings is below the user's home folder, which is not where a relative `--library` option is looked for, and an empty path means the default. The library file setting is read after the `--library` option and the `DERMIXEN_LIBRARY_FILE` environment variable, and it takes effect in the window the next time the window opens a document. The music folder is what `dermixen library scan` scans when it is given no folder, and what the window scans, and a change to it takes effect at the next scan. `docs/settings.md` lists every setting.

## Limits

"Never crashes, never loses work" covers a file that was made to break the app, so every number the app reads from a file has a stated range, and the app refuses a value outside the range with a message that names the value. The limits are wide enough that no real mix or track reaches them.

- **A mix** lasts at most 24 hours from its first sample to its last. A 16-bit stereo WAV file reaches the 4 GiB a WAV file can describe at about 6 hours 45 minutes, so `render` refuses a longer mix as a WAV file before it writes anything, and writes it as an MP3 file.
- **A track** is at most 90 minutes of audio, so that a whole DJ set can be one track of a mix that presents a party's lineup. A decoded track is held in memory, and 90 minutes is about 1.9 GB. An audio file is at most 2 GiB, and its stated sample rate is from 8 kHz to 384 kHz.
- **A tempo**, of a grid or of a tempo node, is from 20 to 999 beats per minute, which is the range the grid editor accepts.
- **A beat** in a document (an anchor, a tempo node's position, or an envelope node's position) is at most 10,000,000 from zero in either direction, and a grid's first beat is within 90 minutes of the track's first sample in either direction. Whole beats of that size add without rounding: a mix would need more than four hundred million tracks before the layout's running sum of anchors lost a beat.
- **A gain or an envelope level** is from minus 144 to plus 24 decibels.
- **A sample** that a decoder produces is a finite number from minus 8 to plus 8, where 1 is full scale. The decoder replaces a sample that is not a number with silence and clamps a larger one, so no analyzer, stretcher, render, or audio device receives anything else. The loudness analyzer reports no loudness for audio that measures above 0 LUFS, which no real master does, and a track with no loudness joins a mix at unity gain.
- **A mix document or an autosave** is at most 16 MiB, and a settings file or a playlist is at most 1 MiB. The app reads all four only from a regular file, never from a device or a named pipe. A key appears at most once in any object of a mix document, and a track's path contains no NUL character.
- **A tag**: the tag reader and the file name parser each keep the first 1,024 characters of an artist and of a title, and the library stores what they kept.

No command and no window action writes a mix document that the app cannot open again: every writer checks the document against these limits, its size included, and lays it out before it touches the disk. `render`, `decode`, and `play --capture` never write over a file they read, which is the mix document, a track of the mix, or the file being decoded, however the output names it. The three commands do replace any other existing output file, since a render is made again from its document, and `mix new` refuses to replace an existing document, since a document is work.

## Analysis strategy

The plan is scoreboard-first, with the standard libraries as the floor and bespoke work as measured upgrades:

1. **Build the eval harness first.** Ground truth comes from public annotated corpora (the GiantSteps key and tempo datasets are annotated Beatport EDM, which matches this app's target music) plus the corrections that accumulate from a real library.
2. **Bind the incumbents as baselines.** aubio for beats and tempo, libkeyfinder for key. They run inside the harness from day one and are always available to ship. This caps the rabbit hole: bespoke work that doesn't beat the baseline is discarded, and the app is shippable at every moment.
3. **Go bespoke only where the scoreboard shows a win.**

Why bespoke can win here: the standard libraries solve a general problem (any music, often real-time, no assumptions). Dermixen solves a narrow one: whole-file offline analysis of machine-made, constant-tempo, mostly four-on-the-floor music that was rendered *from* a DAW grid. Recovering a grid that exists is a much easier problem than imposing one on a human performance, and offline whole-track analysis can use global consensus that real-time trackers structurally can't.

Where the novel effort goes: **downbeat and phrase-boundary detection.** The baselines find beats. To place a transition you need bar 1 and the 16/32-bar phrase structure that governs where a transition lands. None of the open-source analyzers attempt this. Kick patterns and energy shifts at bar and phrase periods are strong signals in this music. Finding each track's effective beginning and ending belongs to the same effort, because the app cannot place anchors without those two positions.

Version 1 detects phrase starts and section changes and shows them on the timeline. It does not move anchors onto them. On the labeled mixes, snapping the kick analyzer's anchors to detected phrases leaves the intro anchor where it is and moves the outro anchor away from its label, and the detector agrees with the labeled bar phase on fifty of sixty-two tracks, so snapping waits until anchor corrections made in the timeline have become labels that settle both questions. `docs/ground-truth.md` holds the phrase scoreboard and its caveats.

Two principles that apply regardless of which analyzer ships:

- **Honest confidence.** Run multiple cheap methods. Agreement means silent trust, disagreement means the track gets flagged for the manual grid editor instead of a confident guess.
- **The correction loop.** Every manual grid fix or key override the user makes becomes labeled ground truth from their real library, and analyzer changes are regression-tested against it forever after.

Known weak spot: lo-fi. Soft onsets and half-time feel cause octave errors (70 vs. 140 BPM) in classical detectors. Mitigations, in order: genre-informed tempo priors, the ×2/÷2 escape hatch, and (only if the scoreboard shows lo-fi lagging) an optional ONNX-based ML model as a bolt-on later.

### The reference corpus

The reference corpus is a Goa trance library of roughly 3,300 files: about 2,000 MP3, 770 WAV, and 58 FLAC. Facts observed there that shape the design:

- Folder conventions: `artist/Artist - Album [CATALOG]/` and `comp/VA - Compilation [CATALOG]/`, with track files named `NN Title` or `NN Artist - Title`. Some names contain non-ASCII characters. Unicode is normal input, not an edge case.
- Every sampled MP3 had ID3v2 tags, so tag coverage is excellent for MP3. The ~770 WAV files are where filename parsing earns its keep: WAV files rarely contain tags at all.
- A `mixes/` folder holds finished mixes and MixMeister project files. Library scans must support excluding folders like it, so finished mixes don't surface as source-track candidates.

Key detection uses profiles tuned for electronic music (published work on EDM-tuned key profiles shows they beat general-purpose profiles on Beatport data), reported per-window so stability can feed the confidence score.

## Verification

Work is verified by ear, by scoreboard, and by code review, and the first two need machinery. Consequences:

- **Golden render tests**: fixed project in, rendered audio compared against a stored reference (tolerance-based comparison, since floating point may differ slightly across platforms), run in CI on Linux and macOS for every push to `main` and every pull request, and locally by `scripts/check.sh`.
- **The scoreboard** makes analyzer quality a number ("bespoke 96.1% correct grids vs. aubio 91.4% on 300 tracks"), not an opinion.
- **The CLI harness lets every engine capability be heard**: "render this crossfade, play it."
- Property tests on the beat/tempo/envelope math in `crates/core`, which is pure and easy to test exhaustively.

## Licensing

The app is GPL-3.0-or-later (see `LICENSE.md`). The two analysis baselines decide that: aubio (GPL-3.0), vendored under `crates/aubio-sys/vendor/`, and libkeyfinder (GPL-3.0), vendored under `crates/keyfinder-sys/vendor/`. Linking a GPL library makes the combined work GPL. The MP3 encoder is LAME (LGPL), which the `mp3lame-sys` crate builds from source behind the media crate's `mp3` feature. The other two vendored libraries place no further condition on the app: Signalsmith Stretch (MIT) under `crates/signalsmith-sys/vendor/`, and the Ooura FFT that libkeyfinder uses, whose notice allows use, copying, and modification for any purpose.

## Stretch goals

Recorded so that nearer-term decisions don't accidentally preclude them. Not planned for version 1.

- **MixMeister project import.** Convert an old MixMeister file into a Dermixen project. The `.mmp` format is a standard RIFF container with the form type `MXMP`, holding FOURCC chunks (`TRKL`, `TRKI`, `TRKH`, `TRKF`, `TRSI`, `TRSO`, `TKTM`, `TKTH`, `TKLY`, `TRKM`, `TRKS`, `GLBL`, `MMVR`) and UTF-16LE strings. Positions throughout the file are 64-bit microsecond counts, and automation nodes are 32-byte records holding a lane identifier, a position, and a 32-bit float value. Source file paths and transition names ("Beatmix 32") are legible in plain sight, so the work is mapping chunk semantics, not cracking an opaque blob. `tools/mmp_dump.py` reads the format and prints it as text or JSON, which is how these claims stay checkable against real playlists. Six real playlists are fixtures under `tests/fixtures/mmp/`, described in `docs/fixtures.md`. Two known requirements: path remapping, since the files reference old Windows paths that span several roots (`D:\audio\goa`, `E:\audio\psytrance`, `C:\audio\samples`, and UNC paths beginning `\\media` all appear, sometimes within one project), so remapping has to handle a set of roots rather than a single prefix (the `mix relink` command, which searches for a moved file by its content hash under any number of folders, covers this), and lossy-conversion reporting, since MixMeister effects have no Dermixen equivalent and must be reported rather than silently dropped (MixMeister EQ envelopes, by contrast, map onto Dermixen's per-track EQ envelopes). The architecture protects this possibility: because the project file is a public, versioned contract, an importer is just another program that writes project JSON. The one standing obligation is to keep the project schema expressive enough to represent a MixMeister-style mix (track list, intro and outro anchors, a mix-wide tempo curve, and per-track volume and EQ envelopes), which it is by construction. MixMeister's anchors mean the same thing as Dermixen's and map across unchanged. MixMeister's tempo markers map straight onto the Dermixen mix tempo curve, since both describe one tempo timeline for the whole mix as sparse absolute BPM values.

## Open questions

- Exact export format confirmation (WAV and MP3 assumed).
