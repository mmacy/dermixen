# The dermixen command

`dermixen` is the command-line front door to Dermixen and the way every engine capability is verified. It is written for two readers at once: a person at a terminal, who gets plain text, and an agent driving the app, who gets JSON behind `--json` and meaningful exit codes.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | The command succeeded. |
| 1 | The command failed. One line beginning `error:` on standard error says why, naming the file or field concerned, and nothing is printed on standard output. |
| 2 | The command line could not be parsed: an unknown command or option, a missing argument, or a flag given twice. The usage text is on standard error. A value the command cannot read or refuses, such as `--bpm fast`, a tempo of zero, or a preset name it does not know, is a failure with exit code 1 and a message naming the option. |
| 3 | The command met a defect in dermixen itself. One line beginning `error:` on standard error names the file and line the defect happened at and what it said. The command stopped where it was, whatever it had finished writing is in place, and a file it was part way through writing was not replaced. A command that ends this way is worth reporting. |

Progress and warnings go to standard error, so standard output contains only the result. `dermixen --version` prints the name and the version and exits with code 0.

A reader that closes the pipe, as `head` does once it has read enough, stops the command writing on that stream. The command says nothing about the closed pipe and goes on to its end. A file it was writing is finished or removed as it would have been. The exit code is the one the command would have ended with, which is 0 for a command that did what it was asked, so `dermixen library query | head` prints the first lines and succeeds.

## What the commands refuse

Every number a command takes is held to the range a mix document gives it, and the message names the option. `--bpm` takes a tempo from 20 to 999 beats per minute. `--intro` and `--outro` take a whole beat within ten million of zero. `--first-beat` takes a time within ninety minutes of the track's own first sample (ninety minutes is the longest track Dermixen reads). `--gain`, and the value `mix set-gain` takes, run from minus 144 to plus 24 decibels. Each end of the `library query --bpm` range is a tempo in the same range. `DESIGN.md` states every limit under "Limits".

Every writer of a mix document checks the document against those limits and lays it out before it writes. No command leaves a document dermixen cannot open again. The text goes to a temporary file beside the document whose name nobody can work out in advance, and that file is moved onto the document in one step. A command that fails partway therefore leaves the document exactly as it was, an edit keeps the permissions the document had, and no run leaves a file at a name somebody else could have planted a link at. `render`, `decode`, and `play --capture` write their audio the same way.

`render`, `decode`, and `play --capture` never write over a file they read: `render` and `play --capture` refuse an output that is the mix document or a track of the mix, and `decode` refuses one that is the file being decoded. The refusal holds whether the output names the file directly, through `..`, through a symbolic link, or through a hard link, and it names the file. Each of the three replaces any other existing output file, since a render is made again from its document. `mix new` refuses to replace a document that is already there, since a document is work.

A mix document, a playlist, and the settings file are each read only from a regular file, and only up to 16 MiB for a mix document and 1 MiB for a playlist or the settings file. A path that names a folder, a device, or a named pipe is refused at once with a message naming it, rather than read. A mix document whose track names a folder, a device, or a named pipe is refused when the document is read, with the track's position and the path. A track whose file is gone is not refused, because finding it again is what `mix relink` is for.

`mix relink` is the exception, because it is the command that repairs such a document. It reads a document whose track names a folder and looks that track up by its bytes, as it looks up a track whose file is gone. A folder is what a person finds where a file was deleted, or where a volume is not mounted. A track that names a device or a named pipe stops `mix relink` too. A file system produces neither on its own, so a path like that was written into the document by hand, and rewriting the document would hide it rather than report it.

Text that came from a file, a tag, a path, or a library record passes through one rule on its way to the terminal. Every C0 and C1 control character, and the delete character, is written out as `\xNN`, where `NN` is the character's number in hexadecimal. A file name holding an escape sequence therefore reaches the terminal as the characters `\x1b` and the letters after it, rather than as an instruction to the terminal. A file name holding a newline stays on one line, so one track is always one line. The bidirectional controls, such as the right-to-left override U+202E, are left as they are, because writing them out would mangle every legitimate Arabic and Hebrew name. A name that holds one therefore prints in an order that differs from the order of its characters. JSON output is unchanged, because JSON writes a control character as an escape of its own.

## JSON output

Every command takes `--json` and then prints exactly one JSON document on standard output. `docs/json/dermixen.schema.json` describes every such document: the definition named after a command (`analyze`, `decode`, `library_scan`, `library_query`, `library_find`, `mix_new`, `mix_show`, `mix_plan`, `mix_relink`, `render`, `play`, `open`, `scoreboard`, `settings`) is the shape of that command's output. The tests in `crates/cli/tests/json.rs` hold every command but `decode`, `mix plan`, and `settings` to its definition. `crates/cli/tests/decode.rs` holds `decode` to the `decode` definition. `crates/cli/tests/plan.rs` holds `mix plan` to the `mix_plan` definition. `crates/cli/tests/settings.rs` holds the three `settings` commands to the `settings` definition. `mix add --json`, `mix move-anchor --json`, and `mix set-gain --json` print the same document as `mix show`. A failure under `--json` is reported the same way as without it: exit code 1 and one `error:` line on standard error.

## The library file

The library is everything Dermixen has learned about the tracks it scanned, kept in one SQLite file that `docs/library.md` describes. The `library` commands and `mix add` read and write that file. Which file is chosen in this order: the `--library PATH` option, which may be given either after `library` or after the subcommand, the `DERMIXEN_LIBRARY_FILE` environment variable, the `library_file` setting that `docs/settings.md` describes, and otherwise `dermixen/library.sqlite` in the user's data folder (`~/Library/Application Support` on macOS, `~/.local/share` on Linux). Every command that opens the library reads the settings file first, whether or not the setting is what names the file, so a settings file that cannot be read fails the command the way it fails `dermixen settings show`. An empty value for the variable is treated the same as the variable being unset. A library file that does not exist yet is created empty. A folder the command makes to hold that file is readable, writable, and enterable by its owner alone (mode 700 on Linux and macOS), because the library names every audio file the person owns, and a folder that is already there keeps the permissions it has.

## Commands

### analyze

```text
dermixen analyze <FILE> [--json] [--bpm <BPM>] [--first-beat <SECONDS>]
```

Reads one audio file and reports everything the library would store about it. The report gives the file's canonical path and its content hash. It gives the length in samples. It gives the beat grid, with the analyzer's name and confidence. It gives the key and the Camelot code, or null when no key analyzer is built in or the analyzer failed on the file. It gives where the music effectively begins and ends, and the intro and outro anchors analysis placed, with their analyzer and confidence. It gives the artist, the title, and the year, with their source. It says whether the year is an estimate, which the report names `year_is_approximate`, as `docs/library.md` describes under `metadata`. The library also stores which release the track came from, so the report gives the label, catalog number, release title, track number, and the lookup that found them, as `docs/library.md` describes under `release`. A fresh analysis leaves the release empty and the year unmarked. The two Discogs programs in `tools/` fill the release, correct the year, and mark an estimated year, as `tools/README.md` describes. The library also stores what the `shifts` phrase analyzer found, so the report gives the bar lines, phrase starts, and section changes, as `docs/library.md` describes under `phrases`. The library stores the file's loudness too, so the report gives the integrated loudness and true peak, as `docs/library.md` describes under `loudness`. With `--bpm` the grid is taken as given rather than analyzed, `--first-beat` gives the first beat's time in seconds (zero if omitted), and the grid's analyzer is reported as `given` with a confidence of one. The anchors, the key, and the phrases are still analyzed. Without `--bpm`, the `pulse` beat analyzer finds the grid, with beat zero on a downbeat, and the `kick` anchor analyzer places the anchors. Both analyzers are built in without any foreign library, as is the phrase analyzer. Nothing is written to the library.

The JSON output is a `track_record` in the schema. The text output lists the same fields one per line, each name padded so the values line up. It gives the phrase analysis as five lines named `phrase_analyzer`, `phrase_confidence`, `downbeat`, `phrase_starts` (the number of phrase starts), and `sections` (the number of section changes). It gives the loudness as one line named `loudness` with the integrated loudness in LUFS and the true peak in dBTP, or `not measurable` for a file the meter found nothing in. The meter finds nothing in a file that is quieter than its absolute gate of minus seventy LUFS, shorter than its four hundred millisecond block, silent, measuring above 0 LUFS, which no real master does, or containing a sample that is not a finite number.

### decode

```text
dermixen decode <FILE> <OUT> [--from <TIME>] [--for <LENGTH>] [--json]
```

Decodes `FILE` with the same call the library and `render` use, `dermixen_media::decode`, and writes the frames as a 16-bit stereo WAV file at 44.1 kHz to `OUT`, whatever sample rate `FILE` itself is stored at. A program that reads the WAV reads the same samples the library measured a track's beat grid against. `track_profile.py` and `grid_check.py` in `tools/` read a track's audio this way. Every decoded sample is held from minus 8 to plus 8, where 1 is full scale, whether the sample the file stores is smaller or larger than that.

`--from` and `--for` cut the file to a span, in the same words `render` reads them: a time written as minutes and seconds, like `7:30`, or as a number of seconds, like `450`. Without `--from` the WAV starts at the start of the file, and without `--for` it runs to the end. A value that is not a time is refused, and so is a length of zero, each with a message naming the flag concerned. A start at or past the end of the file is refused with a message giving the file's length. A span that runs past the end of the file is cut off there.

An output that is the file being decoded is refused before anything is written, as "What the commands refuse" above describes. On success one line on standard output names the file written, its length as `mix show` reports a length, and its frame count. The command writes to a temporary file beside `OUT` whose name nobody can work out in advance and moves that file onto `OUT` when the decode completes, so a failed decode leaves no file where `OUT` was asked for and no file of a predictable name beside it. The JSON output is a `decode` document: `file`, the canonical path of `FILE`, `path`, the WAV file written, `from_seconds`, where the WAV begins in `FILE`, and `length_samples` and `length_seconds`, the length written. Reading a file that is missing, or one that is not in a format Dermixen imports, fails with a message naming the file.

### library scan

```text
dermixen library scan [ROOT] [--exclude <DIR>]... [--library <PATH>] [--json]
```

Finds every audio file under `ROOT`, analyzes the ones the library does not contain yet, and stores them, as `docs/library.md` describes under "Scanning". With no `ROOT` the command scans your music folder, which the `music_folder` setting names and which is `Music/Undefunktis` below your home folder until you set `music_folder`. `docs/settings.md` describes the setting. A music folder that is not there is a failure naming the folder and the setting, and the command then creates no library file. `--exclude` names a folder not to enter, relative to the root or absolute, and may be repeated. The folder that contains your finished mixes is the usual one, so that a mix never surfaces as a source track. One line per file goes to standard error as the scan proceeds, saying what was done with it. On standard output the command prints a summary: how many files were added, moved, unchanged, and completed, and how many duplicates, then one line per duplicate in the form `duplicate PATH, already in the library as PATH`, then each file that failed with its reason, then one line per folder that could not be read in the form `unreadable PATH: REASON`. A completed file is one the library already contained with a record that had no loudness: the file is decoded and its loudness measured, and the rest of its record is left as it was. A file that fails does not stop the scan. Analyzing a new file takes seconds, so a first scan of a large folder of tracks takes hours. A second scan of an unchanged folder hashes every file and analyzes none.

### library query

```text
dermixen library query [--under <DIR>] [--bpm <RANGE>] [--year <RANGE>] [--length <RANGE>] [--key <CODE>] [--compatible-with <CODE>] [--artist <TEXT>] [--title <TEXT>] [--no-approximate-years] [--min-grid-confidence <CONFIDENCE>] [--min-anchor-confidence <CONFIDENCE>] [--library <PATH>] [--json]
```

Lists the tracks in the library that meet every condition given, in ascending order of path. With no conditions given, every track matches. A range is `LOW-HIGH` with both ends included, or a single number for exactly that value. A length range's ends are written as minutes and seconds, as in `7:00-9:00`, or as seconds, as in `420-540`. Both ends must be written the same way, and a single value is not accepted for a length. A code is a Camelot code such as `8A`. The conditions mean what `docs/library.md` says under "Queries". A condition that cannot be read, such as `--bpm fast`, is refused with a message naming the option. With `--no-approximate-years`, the command leaves out every track whose year is an estimate rather than one a source states. On its own it still leaves in a track with no year and no estimate mark, and with a `--year` range the command leaves out a track with no year either way. `--min-grid-confidence` and `--min-anchor-confidence` each take a number from 0 to 1, both ends included. Text that is not a number, a number outside that range, and `nan` are all refused with a message naming the option.

The text output is one line per track: the Camelot code or `--` when the key is unknown, the tempo, the year, the artist and title (with `?` after them when they were guessed from the file name), and the path. The year is written as four digits, as `~` and the year for an estimate, or as `-` for no year. The JSON output is a list of `track_record` objects.

A row of the library that matched and could not be read is left out of both outputs and counted, so one damaged row costs the person that track rather than the whole listing. When there were any, one line on standard error says how many, whether or not `--json` was asked for, and says that scanning the folder those tracks are in with `library scan` replaces a damaged row. `library find` reports the same count the same way.

### library find

```text
dermixen library find <TEXT> [--limit <N>] [--library <PATH>] [--json]
```

Ranks the tracks in the library that match `TEXT`, such as one line of a tracklisting, best first, as `docs/library.md` describes under "Finding a track from text". Only a track that scores above zero is a match, so text that names nothing in any track prints no lines at all and gives an empty list under `--json`. At most `N` matches are printed, five if omitted. The text output is one line per match: the score to two decimals, then the path. The JSON output is a list of objects, each with the `score` and the `record`. A top score of at least 0.8 is a resolved line. A top score below 0.5 is a guess to report rather than use.

### mix new

```text
dermixen mix new <MIX> [--json]
```

Writes an empty mix document to `MIX`, which by convention ends in `.dmx`. It refuses to replace a file that already exists. The document is written to a temporary file beside `MIX` first. `MIX` is then linked to that file in one step, which fails when anything already has the name. The name therefore appears with the whole document behind it or not at all.

### mix add

```text
dermixen mix add <MIX> <FILE> [--position <N>] [--preset <NAME>] [--bars <N>] [--intro <BEAT>] [--outro <BEAT>] [--bpm <BPM>] [--first-beat <SECONDS>] [--no-keylock] [--gain <DB>] [--library <PATH>] [--json]
```

Adds `FILE` to the playlist in `MIX`, at position `N` counting from one, or at the end if `--position` is omitted. The file's record is taken from the library when the library contains its hash, and otherwise the file is analyzed as `analyze` analyzes it and the record is stored, so a file is analyzed once however many mixes it joins.

The track's beat grid and anchors come from that record: the intro anchor as analysis placed it, and the outro anchor moved for the transition length as `docs/project-file.md` describes under "The overlap length and the anchors". `--intro` and `--outro` replace either anchor with a whole beat of the track's grid. With `--bpm`, the grid is the given one instead, and since the analyzed anchors belong to the analyzed grid they are not used: the intro anchor is beat zero and the outro anchor is the last whole beat of the track unless the person gives `--intro` or `--outro`. Keylock is on unless `--no-keylock` is given. The track's gain is the leveling gain that `docs/project-file.md` describes under "The gain", from the loudness in the file's record. `--gain` writes the given number of decibels instead. A record with no loudness gives a gain of `+0.0 dB`, with a warning on standard error that names the file and gives both remedies. A scan of the file's folder measures a track whose record has no loudness, and a track the meter finds nothing in keeps that gain however often it is scanned.

The track is joined to the track before it, and to the track after it when inserted in the middle, with the preset `--preset` names: `blend` (the default), `beatmix`, `bass-swap`, or `cut`, written exactly so. `beatmix` and `bass-swap` run across `--bars` bars (eight if omitted, and at least one), and the blend has one fixed shape, which `--bars` does not change. `docs/project-file.md` describes the nodes each preset writes. A preset name that is not one of the four is refused with a message naming all four. Appending a track writes nodes and removes none, so a node a person placed by hand on the previous track survives unless it sits at one of the exact beats the preset writes. Inserting a track in the middle replaces the transition that joined its two neighbors: on the track before, every node at or after a quarter beat before its outro anchor is removed, and on the track after, every node before a quarter beat before its outro anchor is removed, before the two new transitions are written. A transition longer than the span between the outgoing track's intro and outro anchors is refused, with the span and the length named, because the track's fade in and fade out would otherwise overlap. The blend counts as one hundred and twelve beats for this rule, so it needs twenty-eight bars between the outgoing track's anchors, and the message says to choose another preset or move the anchors rather than to ask for fewer bars. `--bars` moves the analyzed outro anchor for every preset, `cut` included, so a cut falls where a transition of that length would have begun. The cut itself is always a quarter beat.

The resolved anchors must leave the outro after the intro. An add whose anchors do not, whether given or defaulted, is refused with both beat values named. The document is rewritten in place. On any failure the document is left as it was, though a file analyzed during a failed add stays in the library, since its record is true whatever became of the mix. On success the command prints the mix's timeline as `mix show` does.

### mix show

```text
dermixen mix show <MIX> [--json]
```

Lays the mix out on the timeline and prints one line per track: its position, its file name, its tempo, its gain to one decimal place with its sign, a space, and `dB` (as in `-6.0 dB`), its anchors, and the times at which it starts and ends counted from the start of the rendered mix. Then comes the length of the whole mix in minutes, seconds, and samples. The JSON output is a `mix_show` document, which gives the same facts for each track, the gain as `gain_db`, together with the number of tempo and volume nodes it has.

### mix plan

```text
dermixen mix plan <PLAYLIST> [--max-step <BPM>] [--allow-repeats] [--library <PATH>] [--json]
```

Predicts the timeline `mix add` would build from a playlist, and writes nothing at all: no mix document, and no record in the library. `PLAYLIST` is a file of audio paths, one per line, in playlist order. A blank line is skipped, and a relative path is resolved against the folder the command runs in, as every path a command takes is.

The prediction is the mix that `mix new`, and then `mix add PATH` for each path with no options, would build, laid out as `mix show` lays a document out. Each track's record comes from the library by its file's bytes, the grid and the anchors and the leveling gain come from that record as the `mix add` section above describes, keylock is on, and each track is joined to the track before it by a blend of eight bars. The command builds that mix with the code `mix add` uses and lays it out with the code `mix show` uses, so a change to either command changes the plan with it. For a track whose record has no loudness, the command writes the same warning on standard error that `mix add` writes, which is not one of the plan's own warnings and so is not in the JSON list.

The text output is one line per track: its position, its Camelot code or `--` when the library records no key, its tempo to two decimals, the tempo step into it signed to two decimals and blank for the first track, when the track's first sample is heard as `enters M:SS.s`, and the artist and title the library records, each written as `-` when the library records none. The last line gives the number of tracks, the length of the mix, and the tempo the mix opens and closes at. The JSON output is a `mix_plan` document, which gives each track what `mix show` gives it together with the artist, the title, the Camelot code, and the tempo step, and gives the whole plan the playlist's path, the length of the mix, whether the keys were checked, and the warnings.

Three kinds of warning go to standard error, one sentence each, and into the JSON `warnings` list in the same order. A warning is not a failure: the timeline is printed and the exit code is 0. For each transition in turn, counting from one, so that transition 1 is the transition into track 2, the command warns when the tempo moves further than `--max-step` beats per minute (one if omitted) and when the two tracks' Camelot codes do not fit by the rule `docs/library.md` gives under "The Camelot wheel". Then, unless `--allow-repeats` is given, it warns once for each artist on more than one track, in the order the artists first appear, naming every track that artist appears on. An artist is every name in a track's artist field, split on ` & `, ` feat. `, ` feat `, ` featuring `, ` vs. `, and ` vs ` in any case, along with a remixer named in the title inside brackets that contain `remix` or `rmx`, so `Mahadeva (Man With No Name Remix Edit)` credits Man With No Name. Two spellings that differ only in case are one artist, reported with the spelling of the earliest track's record.

Whether the keys are checked at all is decided by the median of the key confidences of every record in the library that has a key, taken over the library rather than over the playlist, because whether the analyzer found keys is a fact about the library. Under a tenth, the Camelot codes are noise, a clash between two of them means nothing, and the command checks no keys: it says so in one line on standard error, which is not one of the warnings, and the JSON `keys_checked` is false. A library in which no record has a key has no median, and the command says `keys are not checked: no track in the library has a key` in the same place.

A failure prints one `error:` line and nothing on standard output, and everything is checked before anything is printed. A playlist that cannot be read is named with the reason, and so is a playlist that names no tracks. A path that is not a file on disk is named. A file the library has no record of is named, along with the `dermixen library scan` command that adds its folder, since a plan analyzes nothing. A library path that names no file is refused with the path named and the same command given, and the command creates neither the library file nor the folder above it, because a plan writes nothing at all. A `--max-step` that is not a finite number of zero or more is refused with the option and the value named. A blend that does not fit between a track's anchors is refused in the words `mix add` refuses it in, since the rule is the same one.

### mix move-anchor

```text
dermixen mix move-anchor <MIX> <N> (--intro <BEAT> | --outro <BEAT>)... [--json]
```

Moves an anchor of track `N`, counting from one, to a whole beat of the track's own grid, as dragging the anchor in the window does. At least one of `--intro` and `--outro` is required, and both may be given. The nodes of that anchor's transition move with it, and a node in the body of the track stays where it sits in the music: for the outro anchor, every volume, EQ, and tempo node from a quarter beat before the anchor on, and for the intro anchor, every node from eight bars before the anchor up to twenty-eight bars after it, stopping short of the transition out, which begins a quarter beat before the outro anchor. Eight bars before the anchor and twenty-eight bars after it are where the blend's rise starts and ends, so every node of a blend moves with the intro anchor, as it does in the window, when the rise ends before the transition out. The fit rule under "mix add" keeps that true for every track that a track follows. The last track of a mix is not held to it, so a last track whose anchors are under twenty-eight bars apart keeps the rise nodes past its outro anchor where they are. The tracks after the moved anchor slide along the timeline, so moving the last track's outro anchor before adding the next track is how the handover into that track is placed, and moving an intro anchor later or an outro anchor earlier lengthens an overlap.

The beat must be whole, and the pair of anchors the track ends up with must leave the outro anchor after the intro anchor, judged on the pair when both are given. A move that is refused, and a track number the playlist does not have, leave the document as it was. Whether the track's transitions still fit between its new anchors is not checked, as the window does not check it, so `mix show` is the place to confirm the anchors afterwards. On success the command rewrites the document and prints the mix's timeline as `mix show` does.

### mix set-gain

```text
dermixen mix set-gain <MIX> <N> <DB> [--json]
```

Sets the gain of track `N`, counting from one, to `DB` decibels, in place of the leveling gain `mix add` wrote or whatever gain the track has now. The number may be negative, as in `dermixen mix set-gain set.dmx 3 -2.5`, and must be finite. Nothing else about the track changes, so this is how a level judged by ear goes into the document without building the mix again. A value that is not a finite number and a track number the playlist does not have are both refused, and a refused command leaves the document as it was. On success the command rewrites the document and prints the mix's timeline as `mix show` does.

### mix relink

```text
dermixen mix relink <MIX> [--under <DIR>]... [--library <PATH>] [--json]
```

Finds the files of a mix again after they have moved. Every `--under` folder is listed first, so a folder that cannot be read at all is refused with a message naming it even when every file is in place. Then every track's file is read and hashed. A track whose file is where the document says, with the bytes the document names, is kept. Every other track is looked for by its hash, first in the library when the library file is already there. The library's record for the hash is trusted only when the path that record names contains those bytes. The search then goes under each `--under` folder in the order given, hashing every audio file there in the order `library scan` lists them until one matches. The command opens no library file that is not already there and creates none, so on a machine with no library file the search covers only the folders given. A file found is written into the document as the track's path, made absolute with symbolic links resolved as `mix add` writes a path, and the command points the library's record for that track at the file too. A file found nowhere leaves the track as it was. A track whose path names a folder is looked for the same way, since a folder where a file was is a file that is gone. This is the one command that reads such a document rather than refusing it.

One line per track on standard output says which it was: `kept`, `relinked` with the new path, or `missing` with where the file was looked for. A track whose file is at the path the document names and could not be read, as an unmounted volume or a permission leaves it, has that path and the reason the operating system gave at the front of its reason, because mounting the volume or changing the permission is the remedy for that track rather than relinking. The document is rewritten, as `mix add` rewrites it, only when a track was relinked. A track still missing is not a failure of the command, since the report is the point. `render` and `play` refuse the mix until the file is found. Progress goes to standard error as files under the folders are hashed. The JSON output is a `mix_relink` document: the path of the mix, how many tracks were `relinked` and how many are `missing`, and one entry per track with its `position`, the `path` the document names now, its `outcome` (`kept`, `relinked`, or `missing`), the path it named before as `from` (null unless relinked), and the `reason` a missing file could not be found (null otherwise).

### render

```text
dermixen render <MIX> <OUT> [--from <TIME>] [--for <LENGTH>] [--handover <N>] [--json]
```

Renders the mix in `MIX`, or a span of it, to `OUT` as a 16-bit stereo WAV file at 44.1 kHz, or as a 320 kbps constant bit rate MP3 when the name of `OUT` ends in `.mp3`, in either case. A build without the `mp3` feature refuses an MP3 output with a message saying so.

`--from` says where to start, counted from the start of the mix on the same clock `mix show` reports, and `--for` how much to render. Without `--from` the render starts at the start of the mix, and without `--for` it runs to the end. Both take a time written as minutes and seconds, such as `7:30` or `7:30.5`, or as a number of seconds, such as `450`. A span that runs past the end of the mix is cut off there. A start at or past the end of the mix is refused with a message giving the mix's length, a length of zero is refused, and a value that is not a time is refused. Each such message names the flag concerned. For a track without keylock the span written contains exactly the frames a render of the whole mix contains at those positions. For a track with keylock it contains the same music rather than the same samples. The pitch-preserving stretcher builds its output from everything it has been fed, and a span feeds it half a second of the track before the span rather than the whole track. The beats land within a millisecond of where the whole render puts them, and each quarter second is at the whole render's level. Either way an excerpt around a transition sounds as the transition sounds in the whole mix. The tracks that have ended before the span begins, or that begin after it ends, are never decoded.

`--handover N` gives the span instead, and the command line may not contain `--from` or `--for` alongside it. The clip written runs from a minute before the incoming track's rise begins to a minute after the outgoing track, track `N - 1`, ends, cut off at either end of the mix. The rise begins where the incoming track's volume envelope begins, at its first node. For the blend that is thirty-two beats before the intro anchor, for a beatmix or a bass swap it is the anchor itself, and for a cut it is a quarter beat before the anchor. A track whose envelope has no node before its intro anchor rises at the anchor. The two anchors meet at the incoming track's intro anchor, and the outgoing track ends at its last sample. All three moments come off the timeline on the clock `mix show` reports, and the frames written are the frames `--from` and `--for` write for that span. A moment that falls before the mix's first sample, which only the rise can when the incoming track is the first to sound, is reported as the start of the clip. `--handover 1` is refused with a message saying that no track comes before track 1, and a track number the playlist does not have is refused as `mix move-anchor` refuses one. On success the line naming the file written is followed by a line saying where the rise, the anchors, and the outgoing track's end fall in the clip, as minutes, seconds, and tenths counted from the clip's start. Under `--json` the `render` document gains a `handover` object: the track the handover goes into, where the clip starts and how much of the mix it covers on the mix's clock, and those three moments counted from the start of the clip.

An output that is the mix document or one of its tracks is refused first, as "What the commands refuse" above describes. So is a mix longer than a WAV file can hold. A WAV file describes at most 4 GiB, which at sixteen bits and two channels is about six hours and forty-five minutes, and a mix may last twenty-four hours. A mix over that limit is refused before anything is written, with a message that gives both lengths and points at an output name ending in `.mp3`, which has no such limit. Every track's file is then hashed and checked against the document. A file that is missing or whose hash differs stops the render before anything is written, with a message naming the file and pointing at `mix relink`. The render then streams: each track is decoded when it is first heard and released when it has finished, so the memory a render needs depends on how many tracks overlap at once and not on the length of the mix. Each track is hashed again as it is decoded, and the render stops when those bytes are no longer the bytes the document names, so a file replaced between the first check and the read never reaches the output. The frames go to a temporary file beside `OUT` whose name nobody can work out in advance, and that file is moved onto `OUT` only when the render completes. A failed render therefore leaves no partial file, and a run stopped from outside, as by a keyboard interrupt, leaves no file at a name a person can predict. Progress goes to standard error as the render proceeds. Tracks with keylock use the pitch-preserving stretcher when it is built in and plain resampling otherwise, with a warning on standard error in the second case. On success one line on standard output names the file written and the length of what was written, as minutes, seconds, and tenths. The JSON output is a `render` document, whose length is that of the span written.

### play

```text
dermixen play <MIX> [--from <TIME>] [--for <LENGTH>] [--capture <WAV>] [--json]
```

Plays the mix in `MIX`, or the span of it that `--from` and `--for` select exactly as they do for `render`, through the machine's default audio output device, and returns when the last frame has been played. The span is checked and every track's file is hashed, as `render` does, before the device is opened, so a mistaken span is refused without touching the device. `play` reads the settings file before it checks the span or hashes a track. When the file sets `audio_buffer_frames`, the device takes that many frames each time it pulls, as `docs/settings.md` describes. The frames the device plays are the frames `render` would write for the same span, produced by the same code as the device plays, so what is heard is what a render of that span contains. The engine keeps up to two seconds of the mix rendered ahead of the device, so when the machine cannot keep up the shortfall is heard as a gap, never as different audio. Before sound starts, the tracks that are sounding at the start of the span are decoded, and each is brought up by playing the half second of its own audio that precedes the span through its stretcher and its EQ. The wait is the decoding, about a third of a second for a seven-minute MP3, and it is the same wherever in the mix the span begins. As each track is decoded, a line on standard error names it. While playing, one line per ten seconds on standard error says where in the mix the sound has reached, on the same clock as `mix show`.

`--capture WAV` writes the frames the device would have played to that file instead of playing them, as fast as they can be rendered, so that the rule that the device plays exactly what a render writes can be checked from the command line. `play --capture` and `render` over the same span write files with identical samples, and a capture is refused for the same reasons a render is: a capture file that is the mix document or one of its tracks, and a span too long for a WAV file, are both refused before anything is written. The capture goes to a temporary file beside the name given and is moved onto it when the span has been played, the way `render` writes its output. A capture has no device to fall behind, so its underrun count is always zero and says nothing about whether the machine could have kept up. A device that cannot be opened, or that cannot take stereo audio at 44.1 kHz, is a failure with a message saying so. A build without the `playback` feature refuses to play and says why, though `--capture` still works in it. Tracks with keylock use the pitch-preserving stretcher when it is built in, as for `render`.

On success one line on standard output says how much was played, from where, and the length of the whole mix, each as minutes, seconds, and tenths, then either `no underruns` or the number of underruns, meaning how many times the device needed frames that were not ready. With `--capture` the line also names the file written. The JSON output is a `play` document with the same facts: the path of the mix document, the path of the capture file or null, where playing began, how much was played, the mix's length, and the underrun count.

### open

```text
dermixen open <MIX> [--json]
```

Opens the mix in `MIX` in the window, and returns as soon as the window has been started. The window runs on by itself, and closing the terminal does not close it. The mix is read first, as `mix show` reads it, so a file that is missing or is not a mix document is refused with a message saying what is wrong and no window is started.

The window's executable is `dermixen-app` in the folder that contains the `dermixen` executable, which is where both executables land when the workspace is built or installed. The `DERMIXEN_APP` environment variable names another executable to start instead, and an empty value is treated the same as the variable being unset. An executable that does not exist is a failure naming the path looked at and the variable, one that cannot be started is a failure giving the operating system's reason, and a system that cannot say where the running `dermixen` executable is is a failure with that reason too. The window is given the mix's path made absolute, with symbolic links resolved, so it opens the same file from any working directory.

The line printed on success names the mix, the executable, and the process ID of the window. With `--json` the same three are printed as the `open` document.

### scoreboard

```text
dermixen scoreboard <DIR> [--giantsteps] [--json]
```

Runs every built-in analyzer over the ground truth in `DIR` and prints the tables described in `docs/ground-truth.md`: the beat scoreboard, then the key scoreboard, and, when the directory contains anchor annotations, the anchor scoreboard, the grid scoreboard, and the phrase scoreboard, the last with every analyzer handed each track's labeled grid as it is. Any warnings the annotations raise go to standard error. With `--giantsteps` the directory is read as a GiantSteps dataset checkout, which contains no anchor annotations. The JSON output is a `scoreboard` document with one row per analyzer for beats, keys, anchors, grids, and phrases, each including its per-track scores. The anchor, grid, and phrase lists are empty when the directory contains no anchor annotations. A phrase row's `on_bar`, `on_8_bars`, `on_16_bars`, `on_32_bars`, `median_bars`, and `sections_within_bar` are the `bar %`, `8 bars %`, `16 bars %`, `32 bars %`, `med bars`, and section `bar %` columns that `docs/ground-truth.md` defines, as shares from zero to one rather than percentages.

### settings

```text
dermixen settings show [--json]
dermixen settings set <NAME> <VALUE> [--json]
dermixen settings reset <NAME> [--json]
```

Shows and changes the settings file that the window and this command share, which is the file the `DERMIXEN_SETTINGS_FILE` environment variable names, else `dermixen/settings.toml` below the user's configuration folder. An empty value for the variable counts as no variable. `docs/settings.md` lists every setting, what it takes, its default, and what it changes.

`show` prints the file's path on the first line and then one line per setting as `name = value`, with ` (default)` after a setting the file does not set. A buffer the file does not set is printed as `audio_buffer_frames = the device's own size (default)`, since the size is then the device's own. `music_folder` and `library_file` are printed as the path the setting gives, so a relative path in the file is printed below your home folder, where the window and the commands look for it, while the file keeps the path as you typed it. `show` reads neither the `--library` option nor the `DERMIXEN_LIBRARY_FILE` variable, which come before the setting when a command chooses its library file. `show` writes nothing, and it prints the path whether or not the file exists. `set` puts one setting in the file and prints that setting's line. `set` with empty text for `music_folder` or for `library_file` takes that setting out of the file instead, as `reset` does, since an empty path means the default. `reset` takes one setting out of the file, so that the setting has its default again, and prints that setting's line. Resetting a setting the file does not set changes nothing and is not a failure. `set`, and a `reset` that has a setting to remove, write the file whole, so a comment in the file does not survive a change either command makes. The file stays, empty, when the last setting is reset.

All three read the file first, and a file that stops one of the three is left as it is. A file the operating system refuses to read is a failure whose message names the file and gives the operating system's reason, which has no line in the file to point at. A file that is not TOML, that gives a value a setting does not take, or that names a setting this version of the app does not know, is a failure whose message names the file, the line, and what is wrong. A name `set` or `reset` does not know is a failure that names it and lists the settings there are. A value the setting does not take is a failure that names the setting, quotes the value, and says what the setting takes. Nothing is written in either case.

The JSON output of all three is a `settings` document: the file's path, and for each setting the value in force and whether the file sets it. The value of an `audio_buffer_frames` the file does not set is null. The value of `music_folder` and of `library_file` is the path in use and is never null, since each of the two settings has a default path.

## Features

The pitch-preserving stretcher, the aubio beat tracker, the libkeyfinder key detector, the audio output device, and the MP3 encoder are foreign libraries built behind the Cargo features `signalsmith`, `aubio`, `keyfinder`, `playback`, and `mp3`, all on by default for this crate. `cargo build -p dermixen-cli --no-default-features` builds a `dermixen` with none of them. That build still renders to WAV (by resampling) but refuses an MP3 output, still plays into a file with `play --capture` but not through a device, still finds grids and anchors with the built-in `pulse` and `kick` analyzers, and stores no key, since the library's key detector is libkeyfinder. The built-in `edm` key analyzer appears only on the scoreboard, as does the aubio beat tracker.

## From two tracks to a mix

`scripts/render-two-tracks.sh` goes from two audio files and their tempos and anchors to a rendered WAV in one command, which is the first thing to listen to:

```sh
scripts/render-two-tracks.sh a.wav 138 896 b.wav 140 32 mix.wav
```
