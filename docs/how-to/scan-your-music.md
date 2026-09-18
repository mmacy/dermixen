# Scan your music

A scan walks a folder, analyzes every audio file that [the library](../library.md) doesn't contain yet, and adds each result to the library, which is everything Dermixen has learned about your tracks, kept in one file in your data folder. Run a scan once over all your music, and again whenever you add tracks.

## Scan a folder

```sh
dermixen library scan ~/audio/goa
```

The scan finds every file under the folder whose extension is `wav`, `mp3`, `flac`, `m4a`, or `mp4`, and analyzes each one: its beat grid, its key, its loudness, where its music begins and ends, its intro and outro anchors, its phrase starts, and its artist, title, and year from its tags or, when the tags are missing, from its file name. One line per file goes to standard error as the scan proceeds. The summary on standard output counts what was added, moved, unchanged, and completed, and how many duplicates were found. Then come a line for each duplicate, `duplicate PATH, already in the library as PATH`, a line for each file that failed with its reason, and a line for each folder the scan couldn't read, `unreadable PATH: REASON`.

Analysis takes about three seconds per track, so a thousand tracks take about an hour. A file that fails to decode or analyze is reported and skipped, and the scan goes on. Symbolic links are neither followed nor listed.

## Leave folders out

```sh
dermixen library scan ~/audio/goa --exclude mixes --exclude samples
```

`--exclude` names a folder not to enter, relative to the root or absolute, and may be repeated. Use it for the folder that contains your finished mixes, so that a mix never surfaces as a source track, and for anything else that isn't a track.

## Choose the library file

The commands pick the library file in this order:

1. The `--library PATH` option, given after `library` or after the subcommand.
2. The `DERMIXEN_LIBRARY_FILE` environment variable. An empty value is treated the same as the variable being unset.
3. `dermixen/library.sqlite` in your data folder: `~/Library/Application Support` on macOS, `~/.local/share` on Linux.

A library file that doesn't exist yet is created empty, by the window as well as by the commands. The window reads the same file, through `DERMIXEN_LIBRARY_FILE`, the `library_file` setting, or the default. To keep a separate library for a second collection, point `--library` at another file when you scan it and set `DERMIXEN_LIBRARY_FILE` to that file when you open the window.

## Scan again after adding tracks

Run the same scan again. Every file is hashed, and a hash the library already contains costs nothing more, so a scan of an unchanged folder reads every file once and analyzes none. A file with a new hash is analyzed and added. A file that was moved or renamed keeps its record, which the scan points at the new path, and the summary counts it as moved. A second file with the same bytes as a file already in the library is a duplicate: the scan reports it and doesn't store it.

## Read the summary from a script

```sh
dermixen library scan ~/audio/goa --json
```

The command prints one JSON document with the counts, the failed files with their reasons, and the folders it couldn't read. [Drive Dermixen from a script](drive-dermixen-from-a-script.md) covers the JSON output of every command.

[The library](../library.md) is the reference for what a record contains, how tags and file names are read, and what a query matches.
