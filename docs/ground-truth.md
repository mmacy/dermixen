# Ground truth for the scoreboard

The scoreboard measures every analyzer against tracks whose tempo, beats, or key are already known.

## Dermixen's own layout

A ground-truth directory contains audio files with annotation files beside them, named after the audio file without its extension. A track called `name.mp3` may have any of:

| File | Contains | Format |
| --- | --- | --- |
| `name.bpm` | The tempo | One number, in beats per minute. |
| `name.beats` | Every beat | One line per beat: the time in seconds from the start of the file, optionally followed by a space and the beat's position in its bar, from 1 to 4. |
| `name.key` | The key | A tonic and a mode, such as `F minor`, `C# major`, `Bb min`, or `Fm`. A bare tonic such as `A` is major. |

Audio is recognized by its extension: `wav`, `mp3`, `flac`, `m4a`, or `mp4`, in either case. A file with any other extension beside the audio is ignored unless it is one of the three annotation files. An audio file with no annotation beside it is skipped. This layout is where corrections made by hand accumulate: every beat grid or key corrected by hand becomes a set of these files, and the analyzers are measured against them from then on.

## The GiantSteps datasets

The GiantSteps tempo and key datasets are annotated Beatport previews of electronic dance music, which matches the music Dermixen is for. A checkout has audio under `audio/` and annotations under `annotations/tempo/`, `annotations/key/`, and, where present, `annotations/beats/`, each named after the audio file's stem. `load_giantsteps` reads that layout into the same structure as Dermixen's own. The audio is not in this repository. Point the loader at a local checkout.

## Metrics

- **Tempo accuracy 1** is the share of tracks whose estimated tempo is within four percent of the annotated tempo.
- **Tempo accuracy 2** is the same, but an estimate that is double, half, triple, or a third of the annotated tempo also counts. The gap between the two accuracies is the octave-error rate, which is the known weak spot on lo-fi material.
- **Tempo error** is the median, over the tempo-annotated tracks, of the distance between the estimated and annotated tempos as a fraction of the annotated tempo. With an even number of tracks it is the mean of the two middle values. The four-percent accuracies say whether an analyzer is roughly right. This says how precisely, which is what decides whether a grid built from its tempo stays on the drums to the end of a six-minute track. An error of one percent walks a grid a whole beat off the drums within a hundred beats. The table shows it as a percentage with two decimals.
- **Beat F-measure** is the harmonic mean of precision and recall of the estimated beats against the annotated ones, where a beat counts as found if it lies within seventy milliseconds of an annotated beat that no other estimate has already claimed.
- **Key exact** is the share of key-annotated tracks whose estimated key is the annotated key.
- **Key weighted** is the mean credit over key-annotated tracks. The annotated key earns one point, a key a perfect fifth away in either direction with the same mode earns half a point, the relative major or minor earns three tenths, the parallel major or minor earns two tenths, and any other key earns nothing. On the Camelot wheel those are the same code, a neighboring number with the same letter, the same number with the other letter, and the same tonic with the other letter.

The accuracies, the F-measure, and the key weights are the definitions the music information retrieval community uses, so a number on this scoreboard can be set beside a published one, with one difference. The community's `mir_eval` program credits only the fifth above the annotated key, so a weighted key score from here can sit slightly above one computed there on the same estimates.

## Anchors: where the music begins and ends and where the anchors go

Anchor ground truth is a separate annotation file, `name.anchors`, beside the audio file `name.ext`, in the same directory layout as the tempo, beat, and key annotations. The timeline window writes its corrections in this format into the corrections folder that `docs/window.md` names, as `name-XXXXXXXX.anchors` with eight hexadecimal digits of the audio file's content hash after the name. The hash is in the name so that two tracks that share a file name in different folders keep separate files. `tools/anchor_truth.py link` pairs the audio with an annotation under the annotation's own name, so such a file works in a linked directory as any other does, and a person who copies one beside the audio by hand must rename the audio to match. The committed set lives in `tests/ground-truth/anchors/`, without the audio. `tools/anchor_truth.py link` gathers the audio beside copies of the annotations from a local audio collection. `dermixen-analysis`'s `anchors` example, run as `cargo run --release -p dermixen-analysis --example anchors -- <directory>`, prints the scoreboard for that directory.

An annotation file contains one `key value` line per fact, with `#` lines as comments:

```
# The grid fitted to the section markers and the measure marker as the outro anchor.
file goa/comp/VA - Analog Dreams [DATCD005]/2 Space Tribe - The Great Spirit (Original Mix) - mastered.mp3
bpm 136.6689
first_beat 0.863555
intro 55.304127 ear
outro 413.549048 mixmeister
```

| Key | Contains |
| --- | --- |
| `bpm` | The tempo of the grid the labels sit on, in beats per minute. Required. |
| `first_beat` | The time of beat zero of that grid, in seconds from the start of the file. Required. Beat zero starts a bar of MixMeister's grid: it is the first of the section markers, which sit whole bars apart, in the playlists and the plot files alike. Whether MixMeister's bars are the audio's bars is what the phrase scoreboard's downbeat column measures. |
| `begins` | Where the music effectively begins, in seconds, then the label's source. |
| `ends` | Where the music effectively ends, in seconds, then the label's source. |
| `intro` | Where the intro anchor belongs, in seconds, then the label's source. |
| `outro` | Where the outro anchor belongs, in seconds, then the label's source. |
| `phrase` | A bar a transition was aligned to, in seconds, then the label's source. May appear any number of times. |
| `section` | A bar at which the arrangement changes, in seconds, then the label's source. May appear any number of times. |
| `file` | The audio file's path below the audio root the `link` command is given. The loader ignores it. The linking tool uses it. |

Labels are given in seconds rather than beats so that they stay true if the grid is refined. Every label names its source, because the labels come from three places of different standing:

- `ear`: a position confirmed by listening. This is the standard the other two are measured against.
- `mixmeister`: a position read from a MixMeister project file with `tools/mmp_dump.py`. The intro anchor is the cue node, where the track enters the mix. The outro anchor is the measure marker, where its fade out begins. The grid is a straight line fitted through the project's section markers, which sit on whole beats of MixMeister's grid and pin its tempo down more precisely than the three decimals the project stores.
- `plot`: the default MixMeister's own analysis wrote into the track's plot file: its effective beginning and ending, and the starts of its thirty-two-beat intro and outro ranges. The grid is fitted to the plot file's own section markers the same way: "Seeing Is Believing" is 131.9993 beats per minute with beat zero at 0.045037 seconds and "Snaafu" 140.0002 with beat zero at 0.045712 seconds. "Instenso", whose markers do not describe one grid, keeps the stored tempo of 139.9968 with beat zero on the first marker at 1.478299 seconds. Taking beat zero as the first beat at or after the start of the file instead, which is what the intro range start alone gives, puts the three grids at 131.9973, 139.9968, and 140.0046 beats per minute with beat zero at 0.041429, 0.192572, and 0.042601 seconds. On "Instenso" that is three beats earlier, a different bar phase, and the difference moves the `kick` row's intro median on the `plot` labels from 29.0 beats under the intro-range grids to 28.0 under the marker grids.

The `mixmeister` labels describe where a track enters and leaves one mix under MixMeister's transitions, which fade a track in over sixteen to twenty-four bars. Under a fade that long, a track can enter at its first sound and still have its kick running by the time the fade completes, and many of the labels bring a track in eight to sixteen bars before its kick or at its very first beat. The one intro label confirmed by ear sits on the bar where the kick is established, which is where the intro anchor belongs under an eight-bar overlap. So a `mixmeister` intro label is where a track enters a mix under a long fade, not where an eight-bar intro anchor belongs, and an analyzer that finds the kick will sit eight to sixteen bars after many of those labels. The `mixmeister` outro labels are closer to the eight-bar meaning: the fade out starts at a musical boundary in the last sixteen bars of the kick, most often at the bar the kick stops, eight bars before it, or sixteen bars before it. The three `plot` files show three different intros. On "Seeing Is Believing" the intro range starts on the first beat of the file, where the kick already runs. On "Instenso" it starts eight bars before the kick, so MixMeister's thirty-two-beat fade completes as the kick lands. On "Snaafu" it starts sixteen bars before the full kick, with a quieter kick running through the last eight of those bars. MixMeister's effective beginning is the first sound rather than the first bar of the body. The scoreboard therefore reports every source on its own row, and only the `ear` rows measure the meaning the app is built for.

Tracks whose MixMeister section markers do not fit one grid, which is what a section edit that shifts the beats looks like, are left out of the `mixmeister` set, as are tracks a mix uses more than once and samples shorter than a minute.

### Phrase labels: the bars a transition was aligned to

A transition should land on the start of a phrase, not merely on a bar, and it should not straddle a section change. The labels that bear on that come from the same files as the anchors, and each says less than it might seem to:

- MixMeister's section markers, the list a playlist and a plot file both contain for each track, are bar lines and nothing more. Of 2,951 markers on 69 playlist tracks, 2,917 sit a whole number of bars from the first, and the gaps between them are most often three bars, then four, two, five, and six. Within a typical track no bar of an eight-bar cycle contains more than a fifth of them, and only one pair in 2,882 sits one bar apart. They mark where MixMeister's beat tracker confirmed the beat, spaced a few seconds apart wherever the beat is clear, and they say nothing about phrases or sections. They do fix the bar phase, which is why beat zero of every annotation is the first of them.
- MixMeister's cue and measure marker, the `mixmeister` intro and outro labels, are the bars a transition was aligned to in a mix. Counted from beat zero, the cues fall on an eight-bar boundary on 58 percent of the tracks and the measure markers on 37 percent, and a sixteen-bar boundary on 42 and 11 percent, so the transition bars are not a count of sixteen from the first beat. The phrases of these tracks start where the arrangement starts them.
- The plot files' intro and outro ranges are MixMeister's default transition bars for the three plot tracks, of the same kind.
- The two `ear` anchors are the only labels confirmed by listening. The ear intro on "The Great Spirit" is the bar the kick is established at, which is also a section change. It is the one `section` label.

Every intro and outro label that sits on a bar line of its grid outside the first bar is therefore also a `phrase` label, written by `tools/anchor_truth.py phrases` with the same time and source: a bar a transition was aligned to. A label in the first bar is left out because the first bar starts the count by definition and says nothing about where the phrases fall, and a label off the bar lines is left out because the scoreboard measures bars. The committed set contains 115 phrase labels on 65 tracks, 109 of them `mixmeister`, 4 `plot`, and 2 `ear`. Another 17 labels are left out, 14 in the first bar and 3 off the bar lines, the latter on "Die sonne - The Rip", where both `mixmeister` labels sit one beat before MixMeister's bar lines, and on "Instenso", whose plot markers do not describe one grid. The `link` command leaves a track out only when no file under the audio root matches the annotation's `file` line, by path or by name. A linked directory that lacks a track lacks it for that reason. The scoreboard aborts on a track whose audio does not decode, so such a track is left out of the linked directory by hand.

### Anchor metrics

Every analyzer is handed the labeled grid, so these metrics measure anchor placement alone and not the beat tracker. An analyzer's anchor is a whole beat of that grid, which becomes a time through the grid, and the error is the distance from the label in beats at the labeled tempo, positive when the estimate is late. For each of the four positions the report gives:

- **n**, how many tracks label the position and got a result.
- **bar %**, the share of those within one bar, four beats either side of the label: the anchor landed on the right bar or the bar next to it.
- **4 bars %**, the share within four bars, sixteen beats: half of an eight-bar overlap, so the transition still works even though the anchor is off.
- **med beats**, the median absolute error in beats, which says how far off the misses are.

The effective beginning and ending are measured the same way, in beats of the labeled grid, and the last column is the mean seconds each analyzer took per track. The `edges` row is the analyzer that trims silence and nothing more. It is the floor a bespoke anchor analyzer must beat.

The only extent labels are the three `plot` ones, and the two analyzers define the extent differently. `edges` trims silence, which is close to what MixMeister's effective beginning is: on those three tracks its beginning is within four bars of the label on every one, with a median error of 4.2 beats. `kick` takes the extent to be the span of the kick, as the module description in `crates/analysis/src/kick_anchors.rs` states, so a quiet build before the kick falls outside it. Against MixMeister's beginnings that puts `kick` a median of 83.9 beats late, far worse than `edges`. On the ending the two are closer, `kick` at a median of 18.9 beats against 56.9 for `edges`, because MixMeister's ending sits near where the beat stops rather than where the sound does. The extent columns show both analyzers against MixMeister's numbers, so a reader can weigh either meaning.

### Grid metrics on the same tracks

Each annotation's grid, with beat zero on a downbeat, also serves as ground truth for beat analyzers, which is what the `grids` example prints, run as `cargo run --release -p dermixen-analysis --features aubio --example grids -- <directory>`. Beside the two tempo accuracies and the median tempo error defined above, it reports:

- **on beat %**, the share of tracks whose first beat lies within seventy milliseconds of a beat of the labeled grid.
- **downbeat %**, the share whose first beat lies within seventy milliseconds of a labeled beat that starts a bar. The transition generator assumes beat zero starts a bar, so this is the figure that says whether the bars of two tracks will line up. Because the labeled beat zero is only assumed to start a bar, the figure compares analyzers with one another under that assumption. It is not an absolute measure of downbeat placement.
- **sure hits**, **sure misses**, **unsure hits**, and **unsure misses**: how many tracks the analyzer reported a confidence of at least one half on and got within four percent or not, and the same for the tracks it was less sure of. An honest confidence puts the misses among the unsure tracks.
- **s/track**, the mean seconds the analyzer took per track.

The `link` command of `tools/anchor_truth.py` also writes each annotation's tempo as a `name.bpm` file, so the tempo scoreboard above runs on the same directory.

### Phrase metrics on the same tracks

The phrase scoreboard hands each phrase analyzer the decoded audio and the labeled grid and scores what it finds against the `phrase` and `section` labels. A phrase analyzer trusts the grid's beats and tempo but not the assumption that beat zero starts a bar. It answers with the beat of the grid that starts a bar, every bar that starts a phrase with the longest phrase starting there, eight, sixteen, or thirty-two bars, and every bar at which the arrangement changes. The table gives, for every analyzer over all labels and then per source:

- **downbeat %**, the share of tracks whose bar lines are the labeled ones, that is, whose downbeat is beat zero of the labeled grid. It appears on the `all` row only, because the grid belongs to the track rather than to a label.
- **phrase n**, how many phrase labels were scored, and **bar %**, the share of those on one of the analyzer's bar lines, which is the downbeat figure weighted by labels.
- **8 bars %**, **16 bars %**, and **32 bars %**, the share of phrase labels that lie, within the seventy-millisecond beat tolerance, on a phrase start of at least that length.
- **med bars**, the median distance in bars from a phrase label to the nearest sixteen-bar phrase start.
- **section n**, how many section labels were scored, and **bar %**, the share within one bar, four beats either side, of a section change the analyzer found.
- **sure hits**, **sure misses**, **unsure hits**, and **unsure misses**: the tracks split at a confidence of one half, where a hit is a track whose downbeat is right and whose phrase labels all lie on sixteen-bar phrase starts.
- **s/track**, the mean seconds per track.

Two programs print this table. `dermixen scoreboard` prints it as its fifth table, with the grids as labeled. The `phrases` example of `dermixen-analysis`, run as `cargo run --release -p dermixen-analysis --example phrases -- <directory>`, prints it twice. It prints once with the grids as labeled, and once with every grid's beat zero moved later by as many beats as the track's index in the run, modulo four, so that beat zero starts a bar on only every fourth track. An analyzer that reads the bar lines off the audio scores the same on both. One that assumes beat zero starts a bar drops to a quarter on the downbeat. The example then prints each analyzer's result on every track.

Two analyzers are on the board. `counted` is the floor: it assumes beat zero starts a bar and a thirty-two-bar phrase and counts phrases from there, reports no section changes, and has a confidence of zero. `shifts`, in `crates/analysis/src/shift_phrases.rs`, measures the low, middle, and high bands one beat at a time. It takes the downbeat to be the beat that cuts the track into bars whose levels differ most from one bar to the next. It takes as section changes the bars where the kick starts or stops and where the levels shift most over four bars. It counts phrases eight bars apart from every section change, with every second one a sixteen-bar phrase and every fourth a thirty-two-bar one, so that after a breakdown of an odd length the phrases are counted from the bar the breakdown ends on. On the 62 linked tracks and their 109 phrase labels, with the grids as labeled:

| analyzer | downbeat % | 8 bars % | 16 bars % | 32 bars % | med bars | sure hits | sure misses | unsure hits | unsure misses |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `counted` | 100.0 | 44.0 | 20.2 | 9.2 | 4.0 | 0 | 0 | 6 | 55 |
| `shifts` | 80.6 | 45.0 | 33.0 | 30.3 | 1.0 | 0 | 0 | 7 | 54 |

With the grids shifted, `counted` falls to 25.8 percent on the downbeat and 13.8, 7.3, and 5.5 percent on the three phrase lengths, and `shifts` keeps every figure. On the `mixmeister` labels alone `shifts` scores 45.6, 33.0, and 30.1 percent against 42.7, 19.4, and 9.7 for `counted`. The `ear` and `plot` rows have two and four labels and settle nothing. The one section label, the kick's entry on "The Great Spirit", is found by `shifts` to the bar.

Three things a reader needs in order to weigh the table:

- The `counted` row's downbeat and `bar %` figures are 100 percent by construction. `counted` returns beat zero as its downbeat, the scoreboard counts a downbeat as right when it is beat zero of the labeled grid, and the phrase labels are the anchor labels that sit on a bar line of MixMeister's grid. The downbeat column therefore measures agreement with MixMeister's bar phase, not accuracy against known truth. `shifts` agrees with MixMeister on 50 of the 62 tracks. On the other twelve it puts the downbeat a beat late on seven, a beat early on three, and two beats off on two, and which phase is right on those twelve is an open question.
- The two analyzers declare phrase starts at different densities, and a label has a higher chance of landing on a `shifts` start than on a `counted` one. Over the 17,827 bars of the 62 tracks, `shifts` declares a start of eight bars or longer every 7.10 bars, one of sixteen or longer every 11.69 bars, and one of thirty-two every 16.43 bars. The share of bars that are starts is therefore 14.1, 8.6, and 6.1 percent. For `counted` the shares are 12.5, 6.25, and 3.1 percent. Against those chance rates, `counted` scores 3.5, 3.2, and 2.9 times chance on the three lengths and `shifts` 3.2, 3.9, and 5.0 times: on eight-bar starts `shifts` is about level with the floor once density is accounted for, and on thirty-two-bar starts it still wins clearly.
- Every constant in `crates/analysis/src/shift_phrases.rs` is tuned on these 62 tracks, and there is no held-out set, so these are training-set figures.

On the 50 tracks where the bars agree, which have 88 of the labels, the labeled transition bars sit on an eight-bar start of `shifts` 55.7 percent of the time and on a sixteen-bar start 40.9 percent of the time. `counted` scores 39.8 and 17.0 percent on the same 88 labels. The labels that miss are spread from one to eight bars off, and the single commonest distance is eight bars, the half phrase. `shifts` reports a confidence below one half on every track that has phrase labels, and the confidence does not separate the tracks it gets right from the ones it gets wrong.

MixMeister's beat lines sit on the kick's peak rather than its attack on about a quarter of the tracks, so that the attack leads the labeled beat by up to a tenth of a beat. On the rest the beat line sits on or before the attack. `shifts` starts each beat's window an eighth of a beat early for that reason, and any analyzer that measures these grids one beat at a time needs the same allowance.

Snapping anchors to phrases, which the `anchors` example measures as the rows `kick+8` and `kick+16`, does not improve them. Those rows move `kick`'s intro and outro anchors to the nearest eight-bar and sixteen-bar phrase start `shifts` finds. On the `mixmeister` labels the intro anchor stays at 27.6 percent within a bar under `kick+8` and rises to 31.0 under `kick+16`, with the share within four bars at 32.8, 31.0, and 36.2 percent for `kick`, `kick+8`, and `kick+16` and the median error near 32 beats for all three. The outro anchor worsens from 15.5 percent within a bar and 22.4 within four bars under `kick` to 13.8 and 20.7 under `kick+8` and 13.8 and 17.2 under `kick+16`, with the median error growing from 32 beats to 36 and 46. On the two `ear` labels the intro stays on the bar under both and the outro moves from 4 beats off to 19 beats off. A labeled outro is a musical boundary within the last sixteen bars of the kick, which the phrases counted from the kick's stop do not reproduce.

## Running it

```
dermixen scoreboard path/to/ground-truth
dermixen scoreboard --giantsteps path/to/giantsteps-tempo-dataset
dermixen scoreboard tests/ground-truth/anchors
```

The third form runs the anchor, grid, and phrase scoreboards as well, because that directory contains anchor annotations. The audio those annotations refer to must be beside them, which `tools/anchor_truth.py link` arranges. On the phrase scoreboard the command hands every analyzer each track's labeled grid as it is. The `phrases` example of `dermixen-analysis` is the way to see the same analyzers over grids whose beat zero has been moved, and each analyzer's result on every track.

Each track is named by its audio file's name without the extension, in the tables and in the JSON output alike. The command prints two tables for every directory, and three more when the directory contains anchor annotations. The sections "Anchor metrics", "Grid metrics on the same tracks", and "Phrase metrics on the same tracks" above define the columns of those three. In the first, each beat analyzer that is built in gets one row: its name, how many tracks it ran on and failed on, the two tempo accuracies as percentages, the tempo error as a percentage, the beat F-measure as a percentage, and the seconds it took per track. In the second, each key analyzer gets one row: its name, tracks and failures, the exact and weighted key scores as percentages, and the seconds per track. The `keyfinder` row is the libkeyfinder baseline, and it appears when the `keyfinder` feature is on. The `edm` row is the analyzer with profiles tuned for electronic dance music, and it always appears. On the 604 tracks of the GiantSteps key set `edm` scores 51.7 percent exact and 61.1 percent weighted against the baseline's 60.4 and 69.0, so the baseline is the key detector the library uses and `edm` stays on the board as the challenger to beat. Analyzers that need a foreign library are built only when their Cargo feature is on, so a row appears when its feature is on and not otherwise.
