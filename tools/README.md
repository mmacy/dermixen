# Tools

Small programs that support the work but are not part of the app. They are written in Python so they can be thrown together quickly and read without knowing Rust. Nothing here is built, packaged, or shipped.

## check_prose.py

Scores Markdown files against a set of writing fingerprints and prints which are out of range: em dashes, semicolons, sentence length, contractions, marketing words, title-case headings, and openers that describe the document. `CLAUDE.md` states the rules under "Writing for humans". The score is a diagnostic: fix the prose and the numbers follow.

```
python3 tools/check_prose.py docs/*.md
```

It needs nothing beyond a Python 3 interpreter and always exits with status 0.

## mmp_dump.py

Reads a MixMeister Fusion `.mmp` playlist and prints what is inside it.

MixMeister project import is a stretch goal in `DESIGN.md`. This program exists so that the format can be checked against real playlists before any of that is written in Rust, and so the claims `DESIGN.md` makes about the format can be verified rather than trusted.

### Running it

```
python3 tools/mmp_dump.py "some playlist.mmp"
python3 tools/mmp_dump.py --json "some playlist.mmp"
```

It needs nothing beyond a Python 3 interpreter. The readable output gives one block per track. The JSON output gives the same information in full, which is the more useful form for feeding another program.

A damaged playlist, one whose chunks nest too deep or whose declared sizes do not match the bytes the file holds, is reported in one line on standard error naming the file, and the program exits with status 1 rather than crashing. A file over 64 MB is refused the same way before it is read, since the largest real playlist is under 1 MB. The readable output escapes a control character found in a path or a transition name, such as an escape or a bell, as `\xHH`, since a terminal interprets those characters as commands rather than printing them. The JSON output leaves the character as it is, since JSON already escapes it.

### What the format looks like

A `.mmp` file is a RIFF container whose form type is `MXMP`. Inside, a `LIST` of type `TRKL` contains the playlist, and each track is a `LIST` of type `TRKI`. Within a track:

| Chunk | Contains |
| --- | --- |
| `TRKH` | Sixteen bytes identifying the track, then the original BPM as a count of thousandths |
| `TRKF` | The source audio file path, as UTF-16LE text |
| `TRSI`, `TRSO` | Names of the transitions into and out of the track, as UTF-16LE text |
| `TKTM` | A list containing the beat grid, starting with a `TKTH` chunk giving the bar length |
| `TKLY` | A list containing the automation lanes |
| `TRKM` | One automation node: a lane identifier, a position, and a value |
| `TRKS` | A range within the track, given as three positions |

Every position anywhere in the file is a 64-bit count of microseconds. Automation nodes are fixed 32-byte records.

### What is confirmed and what is not

The identifications below are checked against twenty real playlists containing 245 tracks between them, not only against the single sample that ships with Fusion.

The microsecond unit is confirmed rather than assumed. One track's end position reads 66,560,000 and the audio file it points at is 66.560000 seconds long.

Two lanes contain tempo. The mix tempo lane gets roughly two nodes per track, and each node's value equals that track's own original BPM, so the tempo of a mix follows each song in turn instead of forcing every song to a single number. In one mix the curve steps from 135.9 through 138.8 and back to 137.8 across ten tracks, which keeps every stretch between 0.98x and 1.00x. A second lane contains one node per playlist, always on the first track.

Two lanes contain transition gain in decibels, and they correspond to the separate intro and outro marker lists in Fusion's own `transitions.xml`. Nodes in the intro lane sit near the start of a track and rise. Nodes in the outro lane sit near the end and fall to the silence floor of -50 dB. Measured across the same twenty playlists, intro nodes average 14% of the way into a track and rise on 51 tracks against 10, while outro nodes average 72% of the way in and fall on 46 tracks against 1.

Four more lanes occupy consecutive bits, all contain decibels, and a track can use several at once. Fusion gives each track four strips, named Volume, Bass, Midrange, and Treble, so the four lanes are almost certainly those four strips. Which bit belongs to which strip is not established, so the program numbers them rather than naming them.

The `TRKS` ranges are most likely the segments a track can be cut into, but the role of each of their three positions is not established. The one in a track's `TKTM` list does give that track's full length, which is confirmed against the audio files themselves.

Positions inside a track are relative to that track, and two single-node lanes contain the anchors that set how far two tracks overlap. The cue lane has the intro anchor, where the track enters the mix, and the measure marker lane has the outro anchor, where the next track enters over it. Both sit on whole beats of the track's grid. The gain nodes sit near the anchors but do not mark them: predicting overlaps from the first gain node in each lane matched a known tracklisting on only one of nine transitions. How long a mix runs follows from the anchors. See "The anchor problem" below.

### Plot files, and where the anchors live

Fusion writes one analysis file per song, with the extension `.mxm`, containing roughly 50 to 100 KB of what it worked out about that track. Where those files go is a preference, so they are usually not beside the audio. The findings below are read from three plot files, all for tracks of one release.

A plot file is a RIFF container of form type `MXM6` with five chunks: `info` with the sample rate and length, `prof` with the analysis results, `fprs` and `fprl` which look like two lengths of acoustic fingerprint, and `plot`, the volume-over-time picture the timeline draws.

The `prof` chunk is 448 bytes and its first fields decode cleanly and identically across all three files. Positions are 32-bit sample counts rather than the microseconds the project files use.

| Offset | Contains |
| --- | --- |
| 8 | Detected tempo, as a float |
| 12 | A float between 0.75 and 0.85, most likely a confidence |
| 24 | Effective beginning of the music |
| 32 | Effective ending of the music |
| 40 | Total length in samples |
| 56 and 64 | Start and end of the intro range |
| 76 and 84 | Start and end of the outro range |

The intro and outro ranges are each **exactly 32 beats long** at the track's own detected tempo, in all three files, to within a hundredth of a beat. Thirty-two beats is eight bars, which is the length the Beatmix 8 transition uses.

This is where a song's default anchors come from: Fusion's analysis works them out once and keeps them in the song's plot file. The positions a particular mix actually used, after any dragging, are the cue and measure marker nodes in that mix's project file, described in the next section.

Two things cannot be read from three files. The intro range does not begin at the effective beginning, and how far after it varies from almost nothing to 63 beats, so the distance depends on something about the music. The outro range sits near the effective ending in two files and 21 beats past it in the third. Three tracks by one artist is far too small a sample to infer the rule, and more plot files would settle it quickly.

### The anchor problem

How far a mix runs can be computed from a project file. Each track has one node in the cue lane and one in the measure marker lane, both positions on the track's own timeline: the cue is where the track enters the mix, and the measure marker is where the next track enters over it. The mix advances from one track's entry to the next by the distance from its cue to its measure marker, and the last track plays from its cue to its end. Summing those distances predicts the length of the released recording of `goat-ranch-monthly-2015-03.mmp` to within a second (1:18:21 against 1:18:22), of `emergent-thisness.mmp` to within three seconds, and of `epcc-2.mmp` to within four. `slow-ascent.mmp` and `slow-ascent-2.mmp` come out eight and eighteen seconds short of the recordings, which is 0.4 percent at most. The sum treats every track as playing at its own tempo and leaves out the stretch each transition applies to the outgoing track, which the tempo lane describes. That stretch is the likeliest source of the residual. `global-goa-party-3.mmp` cannot be summed this way, because it contains sample files without measure markers and uses three tracks twice.

The gain lanes describe the fades, not the anchors. Taking the earliest node in each gain lane as a stand-in for the anchors predicts the same five lengths only to within 0.3 to 1.2 percent, because the intro fade often starts a little before the cue and the outro fade a little after the measure marker.

Span records, the `TRKS` chunks, are segments. A track cut into pieces has one record per piece, each giving the piece's start and end on the track's timeline and the source position it plays from, after a first record that gives the whole file's length. Two saved versions of one mix differ by 86 bytes that turn out to be span records with identical values in a different order. A cue or measure marker on an edited track is a position on the edited timeline, and `anchor_truth.py` maps it back through the segments to a position in the source file.

The one thing a project file does not contain is the default anchor a song had before any dragging. That anchor is in the plot file, and three plot files are available.

### Building anchor ground truth from these files

`anchor_truth.py` turns the two kinds of MixMeister file into annotation files for the anchor and phrase scoreboards, in the format `docs/ground-truth.md` describes. `from-mmp` reads playlists and writes one annotation per track, with the grid fitted to the section markers, the cue as the intro anchor, and the measure marker as the outro anchor. `from-mxm` reads plot files and writes the grid, fitted to the plot file's own section markers the same way, the effective beginning and ending, and the starts of the intro and outro ranges. `phrases` writes a `phrase` line for every intro and outro label that sits on a bar line outside the first bar, because each of those is a bar a transition was aligned to, and says which labels it left out and why. `link` gathers audio from a local copy of the collection beside copies of the annotations so the scoreboards can run. The committed annotations are in `tests/ground-truth/anchors/`.

```
python3 tools/anchor_truth.py from-mmp tests/fixtures/mmp/*.mmp --out tests/ground-truth/anchors
python3 tools/anchor_truth.py from-mxm plots/*.mxm --out tests/ground-truth/anchors --collection "psytrance/artist/Nervasystem - Phantasm Retrodelic EP 001"
python3 tools/anchor_truth.py phrases tests/ground-truth/anchors
python3 tools/anchor_truth.py link tests/ground-truth/anchors ~/audio --out ~/.cache/dermixen/anchors
```

An annotation that already exists is kept, so a label you have corrected by ear survives a rebuild. The `--force` option overwrites an annotation that already exists.

### Getting files to test against

The MixMeister Fusion installer includes a sample playlist, at `Contents/Resources/sample playlist.mmp` inside the application bundle. It is a short demo and exercises only a narrow part of the format, so it is a starting point rather than a proper test. Real playlists exercise far more of it.

## track_profile.py

Prints the level of a track window by window, so an anchor can be placed on the bar where the music does what the transition needs: the kick under the incoming track's intro anchor, and the outgoing track still at full energy where its outro anchor sits.

```
python3 tools/track_profile.py beats track.mp3 --bpm 148.55 --first-beat-sample 68616 --from 960 --to 1216 --per 4
python3 tools/track_profile.py sections track.mp3 --bpm 148.55 --first-beat-sample 68616 --from 0 --to 1024
python3 tools/track_profile.py seconds clip.mp3 --window 2
```

`beats` walks the track on its own grid and prints one line per `--per` beats: the beat the window starts on, the level in decibels below full scale, how hard the low band pulses on the beat, and a bar of `#` for scanning by eye. The pulse tells a kick from a pad. A window with a kick under it pulses by twelve decibels or more, a breakdown or a pad by less than ten, and a bass line playing between the kicks lowers the number, so the sections of one track are compared with each other rather than with another track. The tempo and the first beat are the `bpm` and `first_beat_sample` of the `grid` object that `dermixen library query --json` and `dermixen mix show --json` print.

`sections` prints one line per beat at which a section changes, which finds every beat an anchor could go on in one run instead of profiling at eight beats a line, then two, then one:

```
beat   128  kick leaves   (0 mod 32)
beat   160  kick returns  (0 mod 32)
beat   224  level +6.0 dB  (0 mod 32)
```

Each line names the beat the new state begins on, what changed, and that beat modulo 32, which is where the beat sits on a 32-beat phrase grid. A change at 0 modulo 32 is on a phrase line. A beat has a kick when the low band's pitch sweeps down through it, the same test `grid_check.py` makes. The test reads the average of the bar of beats around the beat being judged. One beat alone is not enough, because a single kick's sweep can sit under a bass note or a fill. The kick leaves at the first beat of a run of eight beats or more that have no kick, and returns at the first beat of a run of eight beats or more that have one. A single kick inside a break is a fill rather than the return. The level steps when the level of the eight beats after a beat differs from the level of the eight beats before it by two decibels or more. A level step within two beats of a kick change is left out, because losing the kick is what moved the level. The pulse number the `beats` command prints cannot stand in for the kick test: a bass line that stops between the beats pulses harder than a kick whose tail fills the beat, so a high pulse is not by itself a kick.

`seconds` prints the level on a fixed clock instead, which is how a rendered clip of a transition is read for a dip at the handover, since the mix has no single grid where two tracks overlap.

It needs the built `dermixen` command, which decodes the audio, and the `numpy` package. `DERMIXEN` names the executable to run in place of the one on the path or in the build folder.

## grid_check.py

Checks a track's beat grid against the kicks in its audio, for a track whose grid confidence is low or whose transition sounds off although its anchors sit on the right beats.

```
python3 tools/grid_check.py track.mp3 --bpm 148.55 --first-beat-sample 68616 --from 0 --to 1024
```

The kick is found by its pitch, not by its loudness. A kick drum's low band sweeps down over its first hundred milliseconds, from 150-175 Hz to around 50 Hz, and a bass note holds one pitch. The program cuts the audio to 30-200 Hz, reads the pitch from the zero crossings, and calls a moment a sweep start when the pitch falls by 25 Hz or more over the following 60 milliseconds while the band stays at a third of the beat's loudest moment or above. It averages sixteen beats into one profile in ten-millisecond bins before looking, which makes the sweep plain even on a track whose bass note is as loud as its kick. A check that only tracked the loudest moment of the low band would sit on that bass note and call the grid good.

Every `--every` beats (eight if omitted) the program finds where the sweep starts near the beat the grid predicts and reports how far from the grid that moment sits. Each check measures its own offset against the offset the check before it found, folds that difference into one beat, and adds the differences up, the way a phase is unwrapped. A check that re-found the kick a whole beat away after a breakdown does not read as drift, and a steady drift of any size adds up however far the offsets walk from the grid. Whole beats come back out of the offset the report gives, because a kick sits on every beat and an offset a whole beat away names the same grid.

The first line of the report gives the offset at the first beat, the drift along the track, how many checks sit within twenty milliseconds of the fitted line out of every check the program made, and the tempo that would remove the drift. A grid that holds shows no drift, a fitted tempo within a few thousandths of a BPM of the given one, and about three quarters or more of the checks on the line, with the rest in stretches that have no kick. An offset near a quarter or a half of a beat is a first beat that is off by that much, and a drift is a tempo that is off, in which case the fitted tempo is the one to give `mix add --bpm`. A check that finds no kick is counted and left off the line, with no entry in the listing. A check whose kick sits further from where the previous check predicts it than the search reaches, or whose offset misses the fitted line by more than twenty milliseconds, is off the line. Those checks are beats with no kick under them, as in a breakdown, unless there are many along the whole stretch, in which case the grid does not describe the track. The program follows the kick from one check to the next, so a tempo that is off by up to about three percent is still measured rather than lost once the drift passes the search window. Further off than that, the sweep smears out of the averaged profile and the report says that fewer than two beats have a kick. After three checks in a row that find no kick within reach, the search widens to half a beat each side until it finds one. The program finds the kick in a stretch that opens before the kick arrives, once the kick starts.

The second line says where the kick starts. When the beats have one kick each it reads `kick starts -181 ms after the grid beat, one kick onset per beat. --first-beat 0.127 would put it at +20 ms`. The program takes twenty milliseconds after the grid beat as where the sweep should start, so every grid that passes this check lines up with every other, and the `--first-beat` in that line is the one to give `mix add`. The time that line suggests is never before the start of the file: when the kick sits further before the grid beat than the grid's first beat sits after the start of the file, the suggestion is moved a whole beat later, which names the same grid. When the beats have two kicks each, which is what a rendered handover contains when its two tracks' grids disagree, the line reads `two kick onsets per beat: -4 ms and +210 ms, 214 ms apart` instead, so a clip with two kicks in it is not read as one kick that drifts.

It needs the built `dermixen` command, which decodes the audio, and the `numpy` package. `DERMIXEN` names the executable to run in place of the one on the path or in the build folder.

## decoding.py

Reads a span of an audio file for `track_profile.py` and `grid_check.py`, both of which import it. `decode` runs `dermixen decode` to write the span as a WAV file and reads it back as mono samples, so the two programs measure the same frames the library and a render read. `find_dermixen` locates the executable: the `DERMIXEN` environment variable when it is set, else `dermixen` on the path, else the release or debug build under the repository's `target/` folder.

## discogs_release.py

Fills the library's `label`, `catalog_number`, `release_title`, and `track_number` columns from Discogs. None of them come from the file's tags: a tag names the pressing a file was ripped from, and when that pressing is a reissue the tag names the wrong release.

It runs in two steps, so nothing writes to the library until you have read what it would write.

```
python3 tools/discogs_release.py match --export collection.csv --out proposed.csv
python3 tools/discogs_release.py apply proposed.csv
```

`match` reads the Discogs collection export CSV that `--export` names, which lists the releases you own and costs no requests. It groups the library by release folder, walking up past a `CD1` or `CD2` folder to the one named `Artist - Album`, and resolves each folder to a Discogs release. A folder whose name states a catalog number in square brackets is matched on that catalog number, compared with its spacing and punctuation dropped, so `TO3 CD 002` and `TO3CD002` are the same number. A folder that states none is matched on its release title, and then only when the artist agrees or the release is a compilation.

A folder that states a catalog number the export does not list can still match on its title, but only when the two catalog numbers name the one release. They do when their numbers agree once leading zeros are dropped and neither says CD where the other says LP. `SZ051` and `SPIRIT ZONE 051` are the one release that way, as are `TRANR604CD` and `TRANRCD604`, and `BMPHQCD01` and `bmphqcd001`.

Where they do not agree the folder is left for the API. The Doof folder `[TIPCD10]` matches the LP `TIP LP 10` on title alone, and `[PHNKL2080-2]` matches `BALLLP01` on another label, and neither is the pressing on disk. The API can search for the exact catalog number instead.

`match --api` asks the Discogs API about the folders the export cannot resolve. It reads the consumer key and secret from the repository's `.env` file, spaces requests a second apart, which stays inside the authenticated limit of sixty a minute, and stops the run when Discogs answers that the limit is reached or when fewer than five requests remain in the window. `--api-limit N` stops asking after N requests, so a trial run over a few folders shows what the search comes back with before the rest of them spend anything. Without `--api` it makes no requests at all.

`apply` reads the CSV and writes the release columns of every track under each resolved folder. The track number comes from the leading number in the file's name rather than from Discogs, so on a release spread over two discs it is the position on the disc: both `CD1/01` and `CD2/01` store 1.

`discogs_release.py` and `discogs_year.py` both refuse a library file whose layout version is not the one `crates/library/src/index.rs` writes, and name the version they found.

Both tools build every request against the address in the module constant `API` and never follow a redirect that changes the host or the port, so the consumer key and secret never reach anywhere Discogs did not name. Neither crashes on an answer of the wrong shape, such as a rate-limit header that is not a number or a `results` field that is not a list. A text value that begins with `=`, `+`, `-`, `@`, a tab, or a carriage return, and so could open as a formula in a spreadsheet, is written to the CSV with a leading single quote instead.

## discogs_year.py

Corrects the library's `year` column, which many tags get wrong. The column means the year the music was first released or produced, not the year of the pressing on disk. A reissue's tag names the year of the pressing, so a track on `Digitalis - The Early Years 1995-2000` is tagged 2020, and a 1996 recording on a 2013 DAT Records pressing is tagged 2013.

It runs in two steps, like `discogs_release.py`.

```
python3 tools/discogs_year.py match --export collection.csv --out years.csv --api
python3 tools/discogs_year.py apply years.csv
```

`match` reads the same collection export as `discogs_release.py`, named by `--export`. It treats every row dated 2005 or later as a suspect, which `--suspect-from` changes. `--include-undated` adds the rows that have no year at all, which need the same lookups. A row with no year takes any year a source gives. A row that has one keeps it unless the year found is earlier, since the later of two is the reissue.

Six sources answer, in order. A track that already appears elsewhere in the library with an earlier year takes that year. So does another cut of a track the library dates: `Sunshrine (Mix 2)` is the same recording session as `Sunshrine`. A remix is not, because a remix is its own piece of work and can come years later, so `Boundless (Funkygong Remix)` from 2015 takes nothing from `Boundless` from 1996. So do a year the collection export gives the release and a year a title states outright. None of those cost a request. Then the catalog number in the folder name is looked up, which dates every track on the release in one request: `BR013CD` is Koxbox's `Stratosfear` from 1996. A track on a release with no catalog number goes to the Discogs track search, which answers with the releases containing that track, and the earliest of those dates the music. A track Discogs does not index takes the year its own release settled on, when two or more of that release's other tracks agree.

The collection export dates the pressing that is owned, so it says nothing about a release on an archival label or a retrospective, and it is passed over for those. The earliest release is otherwise the answer whatever its year. A self-released album from 2011 is from 2011, so `Battle of the Future Buddhas - Digging Mud` keeps the year it has. Two kinds of release are not evidence, though. One is a pressing on a label that presses older material and dates it to the pressing, which is DAT Records, DAT Mafia, memo604, Digital Reprints, and Unreleased Goa. The other is a release whose title says it collects older music, like `The Early Years`, `Single Collection`, or `Remastered`.

An archival pressing that names its recording year is still evidence of that year. `Etnica - Live In Athens 1996` on DAT Records is the recording, whoever pressed it later.

Suntrip, Zion 604 and Anjuna press reissues and new music both, so the label alone cannot date one of their releases. Their reissues are caught by the release title, or decided one release at a time from the notes on the Discogs release page.

Where no source answers, `apply` clears the year rather than guessing. An empty year is better than a wrong one, because a mix that selects on an era takes the wrong tracks from a wrong year and takes none from an empty one.

### Estimating a year for what is left

An estimate is better than nothing for era selection, as long as you can tell it from a year that was researched.

```
python3 tools/discogs_year.py approximate --out approx.csv
python3 tools/discogs_year.py apply-approximate approx.csv
```

`approximate` writes one row per release that still has undated tracks, with the evidence beside it: how many of that release's own tracks are dated and what years they span, and the same for other work by the artists on it. It proposes the middle year of whichever of those two spans is narrow enough to describe one era, which is four years or fewer. A release spanning 1994 to 2011 collects several eras, so it gets no proposal and the `basis` column says why.

Read the CSV and change the `proposed_year` column before applying it. A row with an empty `proposed_year` is skipped.

`apply-approximate` writes only over a track that has no year, so an estimate never replaces a researched year, and it marks every year it writes with `year_is_approximate`. Running `apply` over a track later replaces the estimate with the researched year and clears the mark.

## Running the tests

`tools/tests/` holds the acceptance tests for `grid_check.py`, `track_profile.py`, `discogs_release.py`, and `discogs_year.py`. They run the programs as a person runs them, on audio that `tools/tests/synth.py` writes: a kick whose pitch sweeps, a bass note that holds one pitch, and a grid put on whichever of the two the test is about. `test_reading_through_dermixen.py` checks how `grid_check.py` and `track_profile.py` find and call the `dermixen` command.

```
python3 -m unittest discover -s tools/tests -v
```

They need the `numpy` package and the release build of the command, `cargo build --release -p dermixen-cli`, and they take about ten seconds. They are not part of continuous integration or `scripts/check.sh`, which run the Rust workspace.
