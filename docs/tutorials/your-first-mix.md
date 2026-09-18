# Your first mix

The smallest mix is two tracks and one transition. Building one takes a folder of tracks, the `dermixen` command, and a few minutes, most of them the scan. At the end you have a mix document, a rendered MP3, and the transition on its own as a WAV to listen to.

## Prerequisites

- The `dermixen` and `dermixen-app` executables, built as [Build from source](../how-to/build-from-source.md) describes and on your `PATH`.
- A folder of tracks in WAV, MP3, FLAC, or M4A format, with at least two at about the same tempo.

The steps below use two tracks from a folder named `comp` under `~/audio/goa`. Substitute your own folder and your own two tracks.

## Scan the folder

```
cd ~/audio/goa
dermixen library scan comp
```

The scan finds every audio file under `comp`, analyzes each one, and stores the results in [the library](../library.md), which is everything Dermixen has learned about your tracks, kept in one file in your data folder. One line per file goes to your terminal as the scan proceeds, and a summary follows:

```
added 12
moved 0
unchanged 0
completed 0
duplicates 0
```

Analysis takes about three seconds per track, so a folder of twelve tracks takes under a minute and a thousand tracks take about an hour. The library file is one SQLite file in your data folder (the commands create it on first use): `~/Library/Application Support/dermixen/library.sqlite` on macOS and `~/.local/share/dermixen/library.sqlite` on Linux. A second scan of the same folder hashes every file, finds every hash already in the library, and finishes in a fraction of a second.

## Look at what the scan found

```
dermixen library query --under comp --bpm 139-146
```

One line per track: the Camelot code, the tempo, the artist and title, and the path.

```
12A   139.9 bpm  Total Eclipse & Simon Posford  Sound Is Solid (Remix)            /Users/dermixenuser/audio/goa/comp/VA - Mind Rewind [DMRCD01]/202 Total Eclipse & Simon Posford - Sound Is Solid (Remix).mp3
```

Pick two tracks within a few beats per minute of each other. Two tracks that share a Camelot code, or whose codes sit next to each other on the wheel, mix well harmonically. `--compatible-with 12A` lists every track that mixes well with a track in 12A.

## Create the mix

```
dermixen mix new set.dmx
```

The command writes an empty mix document to `set.dmx` and prints `wrote set.dmx`. A mix document is a JSON file, and you can open it in a text editor at any point to see what the commands have written.

## Add the first track

```
dermixen mix add set.dmx "comp/VA - Fill Your Head with Phantasm Vol 2/VARIOUS ARTISTS - FILL YOUR HEAD WITH PHANTASM VOL.2 - 07 L.S.C. - Big Brain.mp3"
```

The command takes the track's record from the library and prints the timeline:

```
  1  VARIOUS ARTISTS - FILL YOUR HEAD WITH PHANTASM VOL.2 - 07 L.S.C. - Big Brain.mp3   145.00 bpm    +3.7 dB  intro       32  outro      960  0:00.0 to 7:51.1
1 track, 7:51.1 long, 20774040 samples
```

Each line gives the track's position, its file name, its original tempo, the gain that levels it, its two anchors, and where it starts and ends in the mix. The intro anchor is the beat that lines up with the previous track's outro anchor, and the outro anchor is the beat where the next track's intro anchor lands. Analysis placed both anchors: the intro anchor on the bar where the track's kick is established, so the track arrives with its kick running, and the outro anchor sixteen bars before the kick stops, so the track leaves while its kick still runs. The gain brings the track to minus fourteen loudness units relative to full scale (LUFS) without raising its true peak past minus one decibel (dBTP), so a quiet master and a loud one sit at one level.

## Add the second track

```
dermixen mix add set.dmx "comp/VA - Mind Rewind [DMRCD01]/202 Total Eclipse & Simon Posford - Sound Is Solid (Remix).mp3"
```

```
  1  VARIOUS ARTISTS - FILL YOUR HEAD WITH PHANTASM VOL.2 - 07 L.S.C. - Big Brain.mp3   145.00 bpm    +3.7 dB  intro       32  outro      960  0:00.0 to 7:53.5
  2  202 Total Eclipse & Simon Posford - Sound Is Solid (Remix).mp3   139.85 bpm    -6.7 dB  intro       64  outro      816  6:10.5 to 12:55.8
2 tracks, 12:55.8 long, 34214245 samples
```

The times at the end of each line are where the track's file begins and ends on the timeline. The second track's file begins at 6:10.5, but its volume curve keeps it silent until eight bars before its intro anchor, beat 64, which the app has lined up with the first track's outro anchor, beat 960. At 145 beats per minute (BPM), beat 960 falls 397 seconds after the track's beat zero, which is 6:37. The second track rises from silence from about 6:24, is twelve decibels down when the anchors meet at 6:37, and reaches full level twenty-eight bars later, at about 7:25. Over the first eight bars after the anchors the mix tempo ramps from 145 to 139.85 so that the beats of both stay lined up. The first track plays on underneath, easing down to seven decibels below full level at its last sample, 7:53.5, and from there the second track plays alone. That's the default transition, named `blend`. `dermixen mix show set.dmx` prints this same timeline whenever you want it.

## Render the transition and listen

```
dermixen render set.dmx transition.wav --from 6:00 --for 2:00
```

The command renders two minutes of the mix from 6:00 as a 16-bit WAV file, decoding each track as the render reaches it. Progress lines go to standard error as it runs, and the last line names the file:

```
wrote transition.wav: 2:00.0 long, 5292000 samples
```

Open `transition.wav` in any player. The first track runs alone for the first twenty-four seconds, then the second rises under it from 0:24, is twelve decibels down when the anchors meet at 0:37, and is at full level by 1:25, while the first eases down beneath it until its file ends at 1:53. Listen for the kicks: they land together throughout, because both tracks follow the one mix tempo curve. The two minutes you hear are what a render of the whole mix contains at those positions, the same beats at the same levels, so what you check here is what the finished mix has. [What you hear is what renders](../explanation/what-you-hear-is-what-renders.md) says how close the samples are.

## Render the whole mix

```
dermixen render set.dmx set.mp3
```

A name that ends in `.mp3` gives a 320 kbps MP3, and any other name gives a 16-bit WAV at 44.1 kHz. The render streams, so the memory it uses is for the tracks that overlap at a given moment and not for the whole mix. A thirteen-minute mix renders to MP3 in about forty seconds on a laptop.

## Open the mix in the window

```
dermixen open set.dmx
```

The window shows the two tracks as lanes on a timeline, with the mix tempo curve beneath them and the library beside them. [Editing a mix in the window](editing-a-mix-in-the-window.md) continues from here.

## Next steps

- [Find tracks for a mix](../how-to/find-tracks-for-a-mix.md) covers every query the library answers.
- [Change a transition](../how-to/change-a-transition.md) covers the other three presets, the transition length, and moving the anchors.
- [How a mix fits together](../explanation/how-a-mix-fits-together.md) says what the anchors and the tempo curve are and why one curve serves the whole mix.
- [The `dermixen` command](../cli.md) is the reference for every command used above.
