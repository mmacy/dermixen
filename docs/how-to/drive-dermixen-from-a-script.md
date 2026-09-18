# Drive Dermixen from a script

Every command is written for a program as much as for a person: JSON output behind `--json`, exit codes that mean something, and progress kept off standard output.

## Ask for JSON

```sh
dermixen library query --bpm 140-144 --json
dermixen mix add set.dmx track.mp3 --json
dermixen render set.dmx set.wav --json
```

With `--json` a command prints exactly one JSON document (a single object or a single list) on standard output. `docs/json/dermixen.schema.json` describes every document: the definition named after the command (`analyze`, `library_scan`, `library_query`, `library_find`, `mix_new`, `mix_show`, `mix_relink`, `render`, `play`, `open`, `scoreboard`) is the shape of that command's output, and `mix add --json` prints the same document as `mix show`. The tests hold each command to the schema.

## Read the exit code

| Code | Meaning |
| --- | --- |
| 0 | The command succeeded. A `mix relink` that leaves a track missing is a success, since the report is the result. |
| 1 | The command failed. One line beginning `error:` on standard error says why, naming the file or field, and nothing is printed on standard output. |
| 2 | The command line couldn't be parsed: an unknown command or option, a missing argument, or a flag given twice. |

A value a command can't read, like `--bpm fast` or a preset name that isn't one of the four, is a failure with exit code 1 and a message that names the option. A failure under `--json` is reported the same way as without it, so a script reads standard output only after checking the code.

## Keep the streams apart

Progress lines, warnings, and the scan's per-file lines go to standard error. Standard output contains only the result, in text or in JSON. Send standard error to a log or to `/dev/null`, and parse standard output.

## Point every command at one library file

```sh
export DERMIXEN_LIBRARY_FILE=/path/to/library.sqlite
```

The `library` commands, `mix add`, `mix relink`, and the window all read the [library file](../library.md) the variable names, so set the variable once and every command you run in the script, and the window you open afterwards, reads the same file. `--library` on a command overrides the variable. Without either, the commands use `dermixen/library.sqlite` in the user's data folder.

## Write the project file directly

A mix document is versioned JSON with every field described in [The project file](../project-file.md). You can write one from a program without going through `mix add`, and `dermixen mix show` validates it, naming the field that's wrong in the form `tracks[2].grid.bpm: must be a positive number` when it isn't. Content hashes and grids come from `dermixen analyze FILE --json`, which prints the record the library would store without writing anything.

## Hand the mix to a person

```sh
dermixen open set.dmx --json
```

`open` starts the window on the mix and returns at once, printing the mix's path, the executable it started, and the window's process ID. The window runs on after the script exits.

## An example

The script in [Build a mix from a tracklisting](build-a-mix-from-a-tracklisting.md) resolves each line of a tracklisting with `library find --json`, stops on the first line it can't resolve, and builds the mix with `mix new` and `mix add`. [The `dermixen` command](../cli.md) is the reference for every command and option.
