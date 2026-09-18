# The library

The library is everything Dermixen knows about the tracks on disk: where each file is, what its beat grid, key, and anchors are, and who made it. A scan walks a folder, analyzes each new audio file once, and writes the results to a SQLite file that the command line and the desktop shell share. The library is keyed by the content hash of each file, so a file that is moved or renamed keeps its record, and a file is never analyzed twice.

## The library file

The library file is one SQLite file. It records the version of its layout in SQLite's own user version field. The layout is version 5, and it is the only layout this build reads and writes. A file whose version field holds any other number is refused with a message that names the version found and the version this build reads. A SQLite file some other program wrote is refused the same way, because its version field does not hold 5. A file nobody has written to yet, which contains no tables and no version, is laid out as version 5. The remedy for a refused file is to scan again into a new one, since everything in the library can be recomputed from the audio.

A library file holds the one table and the one index Dermixen writes, plus the index SQLite makes for itself to hold the table's key. A file that holds any other table, index, view, or trigger is refused with a message that names the first such object, because a file some other program or person has shaped is theirs rather than Dermixen's, and the remedy is again a new file. Dermixen also reads a library file with SQLite's defensive mode on and its schema untrusted, so nothing a file names can run while Dermixen reads it. On Linux and macOS a library file Dermixen creates is readable and writable by its owner alone, since the library names every audio file in the person's music folder. A folder the `dermixen` command makes to hold that file is readable, writable, and enterable by its owner alone for the same reason. A file or a folder that already exists keeps the permissions it has. Several programs may open one new library file at the same moment, and each one waits its turn to lay the file out rather than failing.

One record per track contains:

| Field | Meaning |
| --- | --- |
| `hash` | The BLAKE3 hash of the file's bytes, as 64 hexadecimal digits. This is the key. |
| `path` | Where the file was last seen. |
| `length_samples` | The length of the decoded audio at 44.1 kHz. |
| `grid` | The beat grid: `first_beat_sample` and `bpm`, as in the project file. |
| `grid_confidence`, `grid_analyzer` | How sure the beat analyzer was, from zero to one, and which analyzer answered. |
| `key` | The key as text such as `A minor`, its Camelot code such as `8A`, the analyzer's confidence, and the analyzer's name. Null when no key analyzer is built in or the key analyzer failed on the track. |
| `extent` | Where the music effectively begins and ends: `begins_sample` and `ends_sample`. |
| `anchors` | The intro and outro anchors analysis placed, `intro_beat` and `outro_beat`, as whole beats of the grid. A mix takes its own copy of these when the track is added. |
| `anchor_confidence`, `anchor_analyzer` | How sure the anchor analyzer was, and which analyzer answered. |
| `metadata` | The `artist`, `title`, and `year`, their `source` (`tags` when read from the file's tags, `filename` when guessed from its name), and `year_is_approximate`. A year is approximate when it is an estimate taken from the years of a release's other tracks, or of other work by the same artists, rather than a year some source states. The estimate places a track in an era without claiming to be the year itself, so a query that needs a year it can rely on can leave those tracks out. The library file stores the mark as the column `year_is_approximate`. The `library query` listing writes an estimated year as `~1996`. The window writes the same estimate as `about 1996`. The `analyze` report gives the year on one line and `year_is_approximate` on another. |
| `release` | What Discogs says about the release the track came from: the `label`, the `catalog_number`, the release `title`, the `track_number`, and the `data_source` naming which lookup answered, `discogs_export` for the collection export and `discogs_api` for the Discogs API. A scan fills none of these. A tag names the pressing the file was ripped from, and for most of this catalog that pressing is a reissue, so a tag names the wrong release. Every field is null until a Discogs lookup records a match. The library file stores them as the columns `label`, `catalog_number`, `release_title`, `track_number`, and `release_data_source`. |
| `loudness` | The EBU R 128 measurement volume leveling uses: `integrated_lufs`, the integrated loudness of the whole track, and `true_peak_db`, its highest true peak in decibels relative to full scale. It is null when the meter found nothing in the track, which is a track quieter than the meter's absolute gate of minus seventy LUFS, shorter than its four hundred millisecond block, silent, measuring above 0 LUFS, which no real master does, or containing a sample that is not a finite number. A track with no loudness joins a mix at unity gain. The library file stores the two as the columns `loudness_lufs` and `true_peak_db`. |
| `phrases` | What the phrase analyzer found, or null when the analyzer failed on the track: the `analyzer`, its `confidence`, the `downbeat` (which beat of every four starts a bar, from zero to three), the phrase `starts` (each a whole `beat` of the grid and the longest phrase in `bars`, eight, sixteen, or thirty-two, that begins there), and the `sections` (the first beat of every bar at which the arrangement changes). The grid is not moved to put beat zero on a downbeat, and the anchors are not moved onto the phrases. The confidence is the analyzer's own figure, and `docs/ground-truth.md` records that on the labeled tracks the confidence does not separate the tracks the analyzer gets right from the ones it gets wrong. Nothing acts on the confidence. |

## Scanning

A scan lists every file under the root whose extension is `wav`, `mp3`, `flac`, `m4a`, or `mp4`, in either case, in ascending order of path. The decoder reads all five: WAV, MP3, FLAC, and the AAC audio in M4A and MP4 files. Paths are stored as the scan was given them. The `dermixen` command scans the folder it is given, and with no folder it scans the music folder the `music_folder` setting names. The command makes the folder it is given absolute with the links along that folder resolved, and it does the same with the music folder, before the scan starts. The library file therefore contains absolute paths, and scanning a folder by name stores the same paths as scanning that folder as the music folder. Folders named in the exclusion list are not entered. An exclusion is a path relative to the root or an absolute one, and a folder is left out when it is that path or lies below it. An exclusion is resolved the way the scan root is, with `.` and `..` in it worked out and every link followed, so `comp/../mixes` leaves out the same folder as `mixes`. The file system does that resolving, so an exclusion matches a folder the way the file system matches a name: on a file system that ignores letter case, which is what macOS formats a disk with unless the person chooses otherwise, `MIXES` leaves out the folder named `mixes`. An exclusion that names no folder on disk leaves nothing out. A folder of finished mixes is the usual exclusion, so a finished mix never surfaces as a source track. Symbolic links are neither followed nor listed, whether they point at folders or at files. A folder the scan cannot read is reported and passed over, and the scan goes on.

Each file found is hashed. A hash already in the library costs nothing more, with one exception: when the record's path still exists the file is left alone, and when the recorded path is gone the record is pointed at the file, which is how a moved file keeps its analysis. The exception is a record at the same path whose loudness is absent. The scan decodes that file, measures its loudness, and stores the completed record, leaving the rest of the record as it was. It reports the file as completed and counts it in the summary's completed total. A record whose loudness the meter cannot measure keeps no loudness, because a null loudness cannot say that the meter answered with nothing, so every later scan decodes that file again and reports it unchanged. A second file with the same bytes as a file already in the library whose path still exists is a duplicate: it is reported and not stored, and the record keeps the path it had. A new hash means the file is decoded and analyzed, then stored. The decoder enforces three limits on every file it reads: an audio file of at most 2 GiB, a stated sample rate from 8 kHz to 384 kHz, and audio of at most 90 minutes. A scan reports a file outside any of these limits as failed with the reason and goes on. A file that cannot otherwise be decoded or analyzed is reported with the reason and the scan goes on too, and a file whose decoding or analysis panics is reported the same way with the panic's message as the reason, so one file that breaks an analyzer costs that file alone rather than every scan from then on. Every scan hashes every file it finds, so a scan of an unchanged folder whose records all have loudness reads every file once, decodes none, and analyzes none.

## Tags and file names

A tag has no length limit of its own, so the tag reader and the name parser each keep the first 1,024 characters of the artist and the title and drop the rest. The cut falls between characters, so a value of characters that take several bytes each is stored as whole characters. The limit covers what those two readers produce. It does not cover the path, which is the file's own name on disk, and it does not cover the release columns, which the Discogs tools write from Discogs.

Reading a record back from a library file applies the same limit of 1,024 characters. A library file is a file another person can hand over, and a text value in one can be of any size, so a read cuts the artist, the title, the analyzer names, the key names, the metadata source, and the release columns to their first 1,024 characters. SQLite does the cutting, so a value of megabytes never reaches the rest of Dermixen, and SQLite counts characters rather than bytes, so the cut falls between characters as the tag reader's cut does. Two columns are read differently. The path is never cut, because a cut path names another file, and a row whose path is longer than 4,096 characters, which is the longest path Linux accepts and four times the longest macOS accepts, is a row the read leaves out. The phrase column holds the JSON the phrase analysis serializes to rather than a tag, and JSON cut short parses as nothing, so a read keeps its first two megabytes, which is above the longest analysis Dermixen writes for the longest and fastest track there can be.

Metadata comes from the file's tags when they contain an artist or a title: ID3 tags in MP3 files, Vorbis comments in FLAC files, iTunes-style tags in MP4 files, and an ID3 chunk in a WAV file when one is present. Values are trimmed of surrounding whitespace, and an empty value counts as absent. The year is the first four digits of the date tag when the date begins with at least four digits, so `1996`, `1996-05-02`, and `19960502` all give 1996 and `05/02/1996` gives none. A file with no usable tags, or tags that cannot be read, gets its metadata from its name, marked with the source `filename` so that queries and the user interface can treat guessed metadata differently from real tags. The reference corpus is where this matters: its MP3 files nearly all have tags, and its WAV files nearly never do.

The name parser reads the file name and, when the name alone does not give an artist, the folders above it. Its rules, in order:

1. The extension is dropped. A name that has underscores and no spaces is read with each underscore as a space.
2. A leading track number is dropped: one to three digits followed by a space, a dot, or a hyphen with a space on each side, or two digits in parentheses followed by a hyphen, with or without spaces between. A three-digit number is a disc number and a track number, as in `203`. A leading catalog number is dropped too: two or more capital letters, then digits, then any run of capitals, digits, and hyphens, followed by a space, as in `AFR010` or `KR005-B1`. Both are dropped again for as long as the name still begins with one, so a name that opens with two numbers loses both. The parser cannot tell a track number from an artist whose name begins with digits, such as `2 Unlimited`. Those names come out wrong, and the `filename` source is the warning.
3. The rest is split on ` - `, a hyphen with a space on each side. If any part is one to three digits on its own, or begins with a track number, that part loses its number and every part before it is dropped, because the number marks where the track's own name starts after an album or label prefix. When several parts qualify, the last one is the marker, since it is the one nearest the title. Then, when more than one part remains, a first part reading `VA`, `Various`, or `Various Artists`, in any case, is dropped.
4. With two or more parts left, the artist is the first part and the title is the last. Parts between them are ignored.
5. With one part left, the title is that part. The artist is taken from the nearest enclosing folder whose name has the form `Artist - Album`, optionally followed by a catalog number in square brackets. Folders named like `CD1`, `CD2`, or `wav` are skipped on the way up. When that nearest folder names `VA`, `Various`, or `Various Artists`, the track is on a compilation and the artist is absent. The search does not go on to folders above it. When no such folder is found the artist is absent.
6. A trailing catalog number in square brackets is dropped from the title, and every value is trimmed of whitespace.

The parser never gives a year. `crates/library/tests/metadata.rs` contains a table of thirty real names from the reference corpus with the artist and title each must give, which is the contract these rules serve. Names outside the table's patterns get whatever the rules give, and the `filename` source is what tells a reader not to trust it.

## Queries

A query is a set of conditions that must all hold, and a query with no conditions matches every track. Results come back in ascending order of path, compared one path component at a time, which is the order a scan lists files in.

A query leaves out a row it cannot read and counts it. A row is unreadable when one of its values has a type the library never writes, when its text is not valid UTF-8, when its path is longer than the 4,096 characters a path may have, or when one of its numbers lies outside the range a mix document accepts: a tempo from 20 to 999 beats per minute, a length from nothing to 90 minutes, a first beat within 90 minutes of the track's first sample either way, an anchor on a whole beat within 10,000,000 beats of beat zero either way, and every other number finite. One unreadable row therefore costs that track rather than the whole library. Asking for one track by its hash or by its path is different: an unreadable row there is an error that names the file, since the caller asked for that track and nothing else can answer.

| Condition | Matches tracks whose |
| --- | --- |
| under a folder | file lies in that folder or one below it |
| tempo range | grid tempo lies in the range, both ends included |
| year range | year lies in the range, both ends included. A track with no year does not match |
| length range | decoded length lies in the range, both ends included |
| key | Camelot code is the one given. A track with no key does not match |
| compatible with a key | Camelot code is compatible with the one given, as defined below. A track with no key does not match |
| artist contains | artist contains the text, without regard to case. A track with no artist does not match |
| title contains | title contains the text, without regard to case. A track with no title does not match |
| leave estimated years out | year is not an estimate. A track whose year is an estimate does not match, whether or not it has a year. A track with no year and no estimate mark matches this condition on its own |
| least grid confidence | grid confidence is at least the value given, the bound included |
| least anchor confidence | anchor confidence is at least the value given, the bound included |

## Finding a track from text

`find` takes free text, such as one line of a tracklisting, and ranks the tracks that match it. The text and each track's artist and title are broken into words, compared without regard to case or punctuation, and two words still match when one letter is wrong, missing, or extra, provided the longer of the two has five letters or more. A leading track number or timestamp on the text, such as `03.` or `12:34`, is ignored.

The score is three quarters the share of the text's words found in the track plus one quarter the share of the track's words found in the text. A line that names the artist and title exactly scores one. A line that names only the title scores high, but below a line that names both, so the fuller name wins among tracks that share a title. A line that names nothing in the track scores zero and the track is left out, as is a track with neither an artist nor a title. Tracks are sorted by descending score, then ascending path.

An agent resolving a tracklisting should take a top score of at least 0.8 as a resolved line and a top score below 0.5 as a guess to report rather than use. `crates/library/tests/find.rs` contains a table of tracklisting lines and the file each must rank first, which is the contract the scoring serves.

## The Camelot wheel

Every key has one Camelot code: a number from 1 to 12 and a letter, `A` for minor and `B` for major. Two codes are compatible when they are the same, when they share a number, or when they share a letter and their numbers are one step apart around the wheel, where 12 and 1 are one step apart. Keys are written with sharps.

| Code | Key | Code | Key |
| --- | --- | --- | --- |
| 1A | G# minor | 1B | B major |
| 2A | D# minor | 2B | F# major |
| 3A | A# minor | 3B | C# major |
| 4A | F minor | 4B | G# major |
| 5A | C minor | 5B | D# major |
| 6A | G minor | 6B | A# major |
| 7A | D minor | 7B | F major |
| 8A | A minor | 8B | C major |
| 9A | E minor | 9B | G major |
| 10A | B minor | 10B | D major |
| 11A | F# minor | 11B | A major |
| 12A | C# minor | 12B | E major |
