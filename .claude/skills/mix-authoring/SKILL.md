---
name: mix-authoring
description: Builds, extends, and tunes a dermixen mix document (.dmx) from the analyzed library, placing each transition by number with the profiling tools in tools/ and checking it with a rendered clip. Use for any request to assemble a mix or set from analyzed tracks, to choose the next track for one, or to tune or audition a transition or an anchor.
---

# Authoring a mix

You turn a brief into a `.dmx` file, or add to one, and every transition you write is tuned before anyone hears it: profiled to the beat, placed on the phrase grid, rendered as a clip, and measured. The deliverable is a mix, or an audition of one handover, that holds together, not a proposal for someone else to finish.

`docs/project-file.md` says what a mix document contains and `docs/cli.md` documents every command. Every command that reads the library takes it from `DERMIXEN_LIBRARY_FILE` or `--library`, as `docs/cli.md` says under "The library file". This file covers what those do not: how to choose tracks, place anchors, check the result, and present it.

Do not re-encode or re-tag a track's file between building a document and rendering it. `render` hashes each track's file again before it decodes it, and it stops with a message that names the file and points at `mix relink` when the bytes no longer match what the document names. Exit status 3 means `dermixen` met a defect in itself, as `docs/cli.md` describes under "Exit codes", and is always worth reporting. It differs from exit status 1, which is an ordinary failure such as a missing file or a bad option.

## Terms

- Intro anchor and outro anchor: the beat of a track where the previous track hands over to it, and the beat where it hands over to the next one. Both count from beat zero of the track's own grid.
- Handover: the stretch where two tracks play together, from the incoming track's rise to the outgoing track's last sample.
- Pre-roll: the eight bars before the intro anchor, in which the blend brings the incoming track up from silence.
- Phrase grid: the beats where a track's sections change. Trance and Goa phrase in blocks of 32 beats (eight bars), so a track's phrase grid is a residue modulo 32. A track whose sections change at 1104 and 1136 phrases at 16 mod 32.
- Audition: a rendered clip of one handover, given to the person to judge before the track goes into the mix.

## The blend

`mix add` writes the `blend` preset unless another is named, and it is the transition to author with. `docs/project-file.md` gives its nodes under "blend". What the shape means for placing anchors:

- The incoming track rises from silence 32 beats before its intro anchor, sits at -12 dB on the anchor, and reaches -2.5 dB 64 beats after it, -0.5 dB at 96, and 0 dB at 112.
- The outgoing track stays at full level to its outro anchor, eases to -7 dB at its last sample, and is never faded out. It ends on its own last bar.
- The outgoing track needs 112 beats between its intro and outro anchors or `mix add` refuses.

So the outgoing track has to keep going, kick and all, for 96 beats after its outro anchor, which is when the incoming track reaches -0.5 dB. Sixty-four beats, at -2.5 dB, is the least that holds without an audible hole. And the incoming track has to have its kick under it from the intro anchor onward with no break in the next 112 beats, because a break there sags the mix just as the outgoing track leaves.

Use `--preset beatmix`, `bass-swap`, or `cut` when the person asks for one, or when the outgoing track cannot give the blend its 112 beats and no earlier outro anchor is musical. Do not change a transition the person shaped by hand. When a transition you did not write is in the way, say so and ask.

## Default parameters

Every brief inherits these unless it says otherwise. They are standing rules, so they need no confirmation and no restating back.

- **No step over 1 BPM between neighbouring tracks.** The limit is on each step, not on the distance from the opening tempo, so a set may climb 1 BPM per transition for four transitions and finish 4 BPM up.
- **The tempo may move down as well as up.** A set does not have to climb from end to end. A dip in the first third and a climb through the last is a shape the rules allow.
- **Each credit appears once.** The rule compares the artist the release credits, whole, against the artists already in the set. It does not read the names inside a credit or in a title. A remixer takes no slot, so `Fred - Awesomeness (Bob mix)` and `Bob - I Like Paste (Fred mix)` can both be in one set. A collaborator takes none either, so `Nervasystem & Aether` is a different credit from `Nervasystem` and both can play. Two tracks credited to Fred cannot.
- **Each release appears once.** Two volumes of one compilation series are two releases, and two pressings of one album are one.
- **Pair keys where the codes allow it.** When the library's key confidence is near zero, the codes are noise and `mix plan` refuses to check them. Walk them anyway as a tiebreaker between records that are otherwise equal, report how many neighbouring pairs fit by `Camelot::is_compatible_with`, and say plainly in the same breath that the codes are not trustworthy. Never keep a record the person dislikes for the sake of a code, and never reorder a set that works musically to gain one more pair.
- **Offer alternates with every proposed set.** Two per slot, each inside the step limit against that slot's two neighbours and clear of every artist and release already in the set. Name the alternates that share an artist or a release with each other, because those are one-at-a-time swaps. Workflow 2 asks for one track rather than a set, and its own step 4 says how many candidates that takes. Give each alternate's Camelot code beside it and say what swapping it in does to the neighbouring-pair count from the "Pair keys" rule below, the same way the proposed set's own count is reported. An alternate that turns a fitting pair into a non-fitting one is still worth offering when it wins on tempo, obscurity, or confidence, but say so plainly rather than leaving the person to work it out from the codes themselves.
- **Aim for 85 to 95 minutes when the brief gives no length.**

## What to ask

Ask for whatever the brief leaves out, in one question rather than several. Every item is optional, so "your call" is an answer and the choice is then yours.

- The tempo range.
- The year, or the range of years.
- The title. The person names their mixes, so ask rather than invent one.
- Whether to draw a cover image.
- Anything else that narrows the pool, like avoiding tracks already used in a published mix.

Never ask which tracks to use. Choosing the tracks is the job.

## Workflow 1: build a set from a brief

1. Read the brief against "Default parameters" and "What to ask" above, and ask once for whatever is missing. A brief that gives a tempo with a spread, like 140 BPM +/-1, has already set the step limit twice over: every track sits inside that window, and no step exceeds the spread.
2. Read the tempo clusters: `dermixen library query --year LO-HI --bpm LO-HI --min-anchor-confidence 0.5` lists each matching track's Camelot code, tempo, year, artist, title, and path in path order, and `--json` gives the whole record. `library query` leaves out a row it cannot read and says how many on standard error, and a scan of the folder that row's file is in replaces the row. Add `--no-approximate-years` when the set is chosen by era and an estimated year will not do. The switch leaves out every track whose year is an estimate and keeps the rest, including a track with no year at all. Tracks of one genre and era cluster at a handful of tempos rather than spreading evenly, and which cluster a set sits in decides which artists can be in it at all. Check the cluster before promising an artist. A set that climbs a step per transition walks from one cluster to the next, which is how it reaches material one cluster does not contain.
3. Choose and order the tracks, writing their paths one per line in playlist order to a file. Keep that file, `mix plan` reads it. The artist rule and the release rule under "Default parameters" decide which of two candidates can stay. A track that exists in the library as two pressings is one track, so check a candidate's artist and title against the set, not only its path.
4. Predict the timeline: `dermixen mix plan playlist.txt --max-step 1`. It prints where each track enters, the step at each transition, and the finished length, and writes a warning on standard error for a step over the limit, two keys that do not fit, or an artist who appears on more than one track. The exit status is 0 either way, and `--json` lists the warnings. `--allow-repeats` turns the last warning off when the person has said an artist may repeat. When the library's keys are noise it says so once and checks none of them. Adjust and run again until the length and the steps are right. A track adds the time between its two anchors, not its full length, which is why the length is predicted rather than summed from track lengths.

   The repeated-artist warning is stricter than the rule above and reports cases the rule allows. `credits` in `crates/cli/src/plan.rs` splits the artist field on ` & `, ` feat. `, ` feat `, ` featuring `, ` vs. ` and ` vs `, and it adds the name in front of `remix` or `rmx` inside brackets in the title. So it counts each half of `Nervasystem & Aether` and every remixer as an appearance. Read a warning that names a remixer or one half of a collaboration credit, confirm that the two credits differ, and carry on. A warning that names the same whole credit twice is the rule being broken.
5. Place the anchors before building. Profile each track's ending and opening with workflow 3, steps 1 to 4, and write the chosen beats beside its path. `mix plan` predicts with the analyzed anchors, so the built length differs from the prediction by the distance the anchors moved, and it always comes out shorter, because tuning moves intro anchors later to clear the incoming track's breaks. Budget about 45 seconds a track for that: a twelve-track set that `mix plan` puts at 87 minutes builds at about 78. Size the set from the corrected estimate rather than from the prediction, and check the built length before rendering.
6. Build the document: `dermixen mix new set.dmx`, then `dermixen mix add set.dmx PATH --intro BEAT --outro BEAT` for each path in order. Giving both anchors to `mix add` writes the blend around them. `mix add` takes a file path, not a search term, and `dermixen library find "artist - title"` turns a line of text into a path. `mix add` also writes each track's leveling gain from the loudness in the library. `mix new` refuses to replace a document that exists, so delete the file before rebuilding. Then check every handover with workflow 4, starting with the ones whose profile was least clear, and fix one by moving the outgoing track's outro anchor with `mix move-anchor` or by rebuilding from the adjusted list.
7. Report the track list with each track's key and tempo, the step at each transition, the finished length, why the set is ordered as it is, and what each transition's profile showed, then the two alternates for every slot. When the brief named an artist the tempo plan rules out, say so, and say what tempo would bring that artist back.

Swapping an alternate in after the anchors are placed moves the two handovers either side of it, so profile the new track and re-measure both before the set is called finished. An alternate that passed the step limit against the slot's original neighbours can break it against a neighbour that was itself swapped, so re-run `mix plan` after any pair of adjacent swaps.

### How the tempo behaves

Every track plays at its own tempo through its body. Across a transition the mix tempo ramps from the outgoing track's tempo to the incoming track's over eight bars after the aligned anchors. `crates/core/src/transition.rs` states this rule and `crates/core/src/tempo.rs` computes the ramp. Two consequences: nothing is stretched to the opening tempo, and the number that matters for each pair of neighbors is the difference between their two tempos. Sorting the whole set into a narrow window around the first track throws away most of the library for no reason.

### The fields that decide

| Field | What it decides |
| --- | --- |
| `grid.bpm` | Where the track sits in the tempo plan, and the size of the step at each of its two transitions |
| `grid_confidence` | Whether that tempo is trustworthy. It says nothing about whether the grid sits on the kick or on a bass note, so run `tools/grid_check.py` on every track before placing anchors on its grid |
| `key.camelot` | Which tracks can follow it |
| `anchors.intro_beat`, `anchors.outro_beat` | Where analysis thinks the track enters and where the next track enters over it. A starting point for workflow 3, not the answer |
| `anchor_confidence` | How far from the analyzed anchors the tuned ones are likely to end up |
| `loudness.integrated_lufs` | The leveling gain the track gets |
| `metadata.year`, `metadata.artist`, `metadata.title` | Era and identity |
| `release.label`, `release.title`, `release.catalog_number`, `release.track_number` | The release the track came from, as Discogs identifies it, filled by `tools/discogs_release.py` for a track a Discogs lookup has matched and null for the rest, with `release.data_source` naming which lookup answered: `discogs_export` for the collection export, `discogs_api` for the Discogs API, or null when no lookup has |

Two Camelot keys fit when they share a number, or share a letter and sit one step apart on the wheel. `Camelot::is_compatible_with` in `crates/analysis/src/camelot.rs` is the authority and `mix plan` checks each pair with it. Stepping the number by one at every transition gives a set a direction, and holding a number for two tracks adds length without breaking the walk. When `key.confidence` is near zero across the library, the analyzer did not find keys and the code is noise. The key rule under "Default parameters" says what to do with the codes then.

The path says something the metadata does not. A file under `VA - Trip Through Sound [BR65001-2]` came off a Blue Room compilation, which says more about how a track sounds than its title does.

## Workflow 2: extend a mix by one track

1. Read the mix: `dermixen mix show MIX --json` gives every track's path, hash, grid, anchors, `start_seconds`, and `end_seconds`. The last track is the outgoing one.
2. Read the outgoing track's record from the library (`dermixen library query --json`, matched by hash in code, since the query has no hash condition) for its key, and take the step limit from the mix itself: the largest tempo difference between neighbors so far, unless the person names one.
3. Search the library's records in code, not with `--bpm` alone: keep the tracks whose tempo is within the step of the outgoing track's exact tempo, whose key is compatible when keys are trustworthy, whose era fits, whose hash is not in the mix, and whose artist is not already in the mix unless the person has said an artist may repeat. Note the nearest misses, within a tenth of a BPM outside the step, and any track whose file exists in two pressings with different tempos.
4. Present ten candidates as a table unless asked for another form or number: artist, title, year, tempo with its difference from the outgoing track, key, grid confidence, the minutes between its anchors (what it adds to the mix), and a one-line reason. Show an estimated year as an estimate, like `~1996`, rather than as a plain year. Mark the nearest misses as such. When the pool is thin, say so and say what would widen it: tracks shown before and not used, the step limit, or the era. A track shown before and not chosen is a candidate again whenever the person says so.
5. The person picks one or more to audition. This is a question of taste, so do not pick for them. For each pick, copy the mix (`cp MIX auditions/NAME.dmx`), tune the handover on the copy with workflow 3, render and measure it with workflow 4, and present the clip.
6. When the person names the winner, keep a backup (`cp MIX MIX-before-NAME.dmx`) and copy the audition document over the mix. Then run `dermixen mix show MIX` and report the new length.

## Workflow 3: tune a transition

The tools need the built `dermixen` command and the `numpy` package. `reading-profiles.md` shows what their output looks like and how to read it. Both take the tempo and the first beat from the `grid` object of the track's record, as `--bpm` and `--first-beat-sample`.

1. Profile the outgoing track's ending. Its last whole beat is `(length_samples - first_beat_sample) / 44100 / (60 / bpm)` rounded down. Run `python3 tools/track_profile.py sections OUT.mp3 --bpm BPM --first-beat-sample N --from LAST-320 --to LAST`, which lists the beats where the kick leaves or returns and where the level steps, each with its residue modulo 32, then `python3 tools/track_profile.py beats OUT.mp3 --bpm BPM --first-beat-sample N --from LAST-320 --to LAST --per 8` to see the shape between them. Find three things: the beat where the kick leaves (the pulse drops from the track's kick level to its no-kick level and stays there), the beat where the level falls (a fade or a stop), and the phrase grid (the beats where either changes, modulo 32).
2. Choose the outro anchor on the phrase grid, at least 96 beats before the kick leaves or the fade begins, 64 at the least, and at least 112 beats after the intro anchor. Take the latest beat that meets all three, so the outgoing track plays as long as it can.
3. Profile the incoming track's opening the same way, with `--from 0 --to 320`. Find the first phrase where the kick is established, with the pulse at the track's kick level and the level at its body level, and check that the 112 beats after it contain no break. The intro anchor goes on that phrase boundary. It needs 32 beats of audio before it, because a pre-roll that starts before the file's first sample cuts in part way up the rise. When the kick is established at beat 16, the anchor is 48 at the earliest, or later if the music before 48 is not a build.
4. Check every track's grid against its kick before placing anchors on it: `python3 tools/grid_check.py FILE --bpm BPM --first-beat-sample N --from 8 --to 1024`. The second line of the report says where the kick's pitch sweep starts relative to the grid beat and which `--first-beat` would put it at +20 ms, which is where it starts on a grid that sits on the kick. A sweep that starts more than about 40 ms from there is a grid on a bass note: about a quarter beat late when the kick's sub-bass swells after its click, half a beat late when the offbeat bass is as loud as the kick. The fix is `mix add --bpm BPM --first-beat SECONDS` with the anchors given by hand, since the analyzed anchors belong to the analyzed grid. A drift is a wrong tempo, and the fitted tempo goes to `--bpm` the same way. `reading-profiles.md` shows both reports. The pulse cannot make this call: a stretch that profiles as kick-only can be a bass pulse, and the low band's peak on a track like that is the bass, so the sweep is what decides where the kick is.
5. Apply the placement to the mix, or to the audition copy: `dermixen mix move-anchor MIX N --outro BEAT` moves the outgoing track's outro anchor and takes its nodes along, and `dermixen mix add MIX IN.mp3 --intro BEAT` adds the incoming track with its intro anchor where you chose. `dermixen mix move-anchor MIX N --intro BEAT` moves an intro anchor already in the mix and takes every node of the blend's rise along, so either way of placing it gives the same nodes.
6. Render and measure the handover with workflow 4. When it dips, move the outro anchor a phrase earlier with `mix move-anchor` if the outgoing track left before the incoming one was up, or start again from a fresh copy with `--intro` a phrase later if the incoming track had a break, and render again. Repeat until the handover holds.

## Workflow 4: render and check a handover

1. `dermixen render MIX clip.mp3 --handover N` renders from 60 seconds before the incoming track's rise to 60 seconds after the outgoing track ends, so the clip contains both tracks alone as well as the handover, and prints where the rise, the anchors, and the outgoing track's end fall in the clip. Keep those three times.
2. `python3 tools/track_profile.py seconds clip.mp3 --window 2` prints the level every two seconds. From the rise to the outgoing track's end, a level that stays within about 1 dB of its neighbors is a handover that holds. A dip of more than about 2 dB is a hole, and its cause is in workflow 3 step 6. A step of 2 dB or more between the two tracks' steady levels either side of the handover is a leveling difference, which `dermixen mix set-gain MIX N DB` on the quieter track corrects.
3. `python3 tools/grid_check.py clip.mp3 --bpm BPM --first-beat ANCHOR --from 48 --to 112`, with the incoming track's tempo and the anchor time the render printed, checks that the two kicks land together where both are near full level. One kick onset per beat is a handover that holds. Two kick onsets per beat are two grids that disagree, and the distance between them says by how much: half a beat is a grid on an offbeat bass note, and the fix is workflow 3 step 4 on the track whose grid is wrong. The level profile cannot see this.
4. Present the clip only once it measures right. A clip you know is flawed is not an audition, and fixing it afterwards costs a second listen. Name what you checked: the two anchors, where the outgoing track's kick leaves relative to the outro anchor, where the incoming track's kick is established relative to the intro anchor, the largest dip in the clip, and that the clip has one kick onset per beat.

## What this skill does not decide

Which records suit a brief. Whether a track is dark, whether it belongs at midnight or at sunrise, and whether two records belong next to each other are questions of taste and genre knowledge. The library gives artist, title, year, key, tempo, and, for a track a Discogs lookup has matched, the release the track came from with its label and catalog number. The judgment is yours, and the person you are working for may be a DJ who hears a wrong call at once, so give your reason for a choice rather than presenting it as self-evident.
