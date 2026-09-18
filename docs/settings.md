# Settings

A setting is a choice a person makes once. Every run of the window and of the `dermixen` command reads the same settings file, so `dermixen-app` and `dermixen play` open the audio device the same way, and a metronome turned on in the window is on the next time the window opens.

## The file

The settings file is `settings.toml` in the `dermixen` folder below the user's configuration folder, which is `~/Library/Application Support` on macOS and `~/.config` on Linux. The `DERMIXEN_SETTINGS_FILE` environment variable names another file instead, and an empty value for the variable is the same as no variable. `dermixen settings show` prints the file's path.

The file is plain text in TOML, one setting per line as `name = value`:

```toml
audio_buffer_frames = 256
metronome = true
grid_strip_collapsed = false
library_collapsed = false
library_word_wrap = false
music_folder = "/media/goa"
library_file = "goa/library.sqlite"
```

Every setting is optional. A setting the file does not mention has its default, a missing file means every default, and an empty file reads the same as a missing one. A line that starts with `#` is a comment. A file from a version of the app with fewer settings reads as it did. The window reads the file when it opens, and the window and `dermixen settings` write the whole file when a setting changes, so a comment in the file does not survive a change made in the window or by the command. A reset of a setting the file does not set changes nothing. The file stays, empty, when the last setting is reset.

A value the setting does not take (`metronome = "yes"`, or a buffer of zero) and a name this version of the app does not know (`metronom = true`, or a setting a later version added) are errors. The window does not open, and every command that reads the file fails with exit code 1. The commands that read the file are `dermixen settings`, `dermixen play`, and the commands that open the library: `library scan`, `library query`, `library find`, `mix add`, `mix plan`, and `mix relink`. For a bad value the message names the file, the line, the setting, and what the setting takes. For an unknown name it names the file, the line, the name, and the settings there are. A file the operating system refuses to read is reported with the file and the reason, and has no line. Nothing is replaced by a default, and a file that cannot be read is never written over.

## The settings

| Setting | Takes | Default | What it changes |
| --- | --- | --- | --- |
| `audio_buffer_frames` | A whole number of frames from 1 to 4294967295 | The device's own size | How many frames the audio device takes each time it pulls from the mix's preview or the grid editor's audition. A change made while the track plays is heard from the next frame the device pulls, so the buffer is the delay between a nudge and the ear: 512 frames is about 12 milliseconds at 44.1 kHz, and 128 frames is about 3. A smaller buffer shortens that delay and risks dropouts on a slow machine. A size outside the range the device takes is reported when the preview or the audition starts, with the number of frames and the device's reason in the message. On macOS the reason gives the range the device takes. The size takes effect the next time the device is opened, which is the next press of **Play** or **Play track** in the window and the next `dermixen play`. On macOS the buffer size is a property of the device itself rather than of one app, so a size set here stays on the device after Dermixen closes, and another app that leaves the size alone plays with it. `dermixen settings reset audio_buffer_frames` goes back to the device's own size. |
| `metronome` | `true` or `false` | `false` | Whether the grid editor's metronome clicks on the beats of the grid. Turning the metronome on or off in the window changes this setting, so the click is as the person left it when the window opens again. |
| `grid_strip_collapsed` | `true` or `false` | `false` | Whether the grid editor's strip, the waveform with the grid drawn over it, is collapsed, so the editor shows its row of controls alone. Hiding or showing the strip in the window changes this setting, so the strip is as the person left it when the window opens again. |
| `library_collapsed` | `true` or `false` | `false` | Whether the library panel beside the timeline is hidden, so the timeline has the whole width of the window. The library panel's toggle in the first row of controls above the timeline, the **View** menu item that hides and shows the panel, and the **Library hidden** checkbox in the settings dialog change this setting, so the panel is as the person left it when the window opens again. |
| `library_word_wrap` | `true` or `false` | `false` | Whether the text in a library panel cell wraps onto further lines, so a row is more than one line when a value is too wide for its column. The **Wrap library cells** checkbox in the settings dialog changes this setting, so the table wraps a cell's text or cuts it off at the column's edge as the person left it when the window opens again. |
| `music_folder` | A path in quotes | `Music/Undefunktis` below your home folder | The folder your music is in. `dermixen library scan` with no folder scans this folder, and so does a scan you start from the window. A relative path is below your home folder, and an empty path means the default. The folder need not be there until a scan reads it, and a scan of the music folder when the folder is not there fails with the path and the name of this setting. A scan enters every folder below this one, so a mix you rendered into a folder below it joins the library as a track, and `dermixen library scan --exclude` is the way to leave a folder out. The window's own scan takes no exclusions. A change takes effect at the next scan. |
| `library_file` | A path in quotes | `dermixen/library.sqlite` below your data folder, which is `~/Library/Application Support` on macOS and `~/.local/share` on Linux | The library file the window and the `library` commands open, which `docs/library.md` describes. The `--library` option is read first, then the `DERMIXEN_LIBRARY_FILE` environment variable, then this setting. A relative path here is below your home folder rather than below your data folder, while a relative path you give to `--library` or to `DERMIXEN_LIBRARY_FILE` is below the folder you run the command in, so one path can name two different files. An empty path means the default. A library file that is not there yet is created empty, by the window as well as by the commands. A change takes effect in the window the next time the window opens a document, and in a command the next time you run it. The settings dialog's **Choose...** button for this setting opens a file picker that shows the files whose name ends in `.sqlite`, so a file that is not there yet is named by typing its path in the field beside the button. |

## The command

```text
dermixen settings show [--json]
dermixen settings set <NAME> <VALUE> [--json]
dermixen settings reset <NAME> [--json]
```

`show` prints the file's path and then one line per setting as `name = value`, with `(default)` after a setting the file does not set. An unset buffer is shown as `audio_buffer_frames = the device's own size (default)`. `show` prints `music_folder` and `library_file` as the path the setting gives, so a relative path is printed below your home folder, where the window and the commands look for it, and a setting the file does not set is printed as its default. The `--library` option and the `DERMIXEN_LIBRARY_FILE` variable come before the setting when a command chooses its library file, and `show` does not read either. The file keeps the path as you typed it, and a path whose text in the file is empty is printed as the default. `set` writes one setting and prints its line. `set` with empty text for `music_folder` or for `library_file` takes that setting out of the file, as `reset` does, since an empty path means the default. `reset` removes one setting from the file, so that it has its default again, and prints its line. A name the command does not know is a failure that names it and lists the settings there are. A value the setting does not take is a failure that names the setting, quotes the value, and says what the setting takes. In either case nothing is written. The JSON output names the file and gives every setting's value and whether the file sets it. The value of `music_folder` and of `library_file` is the path in use, which is the default when the file sets nothing, so neither of the two paths is ever null. `docs/cli.md` describes the command in full, and the `settings` definition in `docs/json/dermixen.schema.json` is the shape of the JSON output.

## The window

**Settings** above the timeline opens the settings dialog, which shows the file's path and every setting. A change is written to the file at once. `docs/window.md` describes the dialog.

The dialog has a row per setting, in this order: **Audio buffer**, **Metronome**, **Grid strip collapsed**, **Library hidden**, **Wrap library cells**, **Music folder**, and **Library file**. The last two are text fields with a **Choose...** button each, and each field shows the path in use in grey while the setting is unset. A path you type is read when you press return or click away, which clicking **Choose...** does as well, and an empty field leaves the setting unset, which means the default. A picker you cancel leaves the setting as it was. A music folder you choose takes effect at the next scan, which is **Library > Scan music folder**. A library file you choose takes effect the next time you open a mix or start a new one, and the window makes the file when it is not there yet.
