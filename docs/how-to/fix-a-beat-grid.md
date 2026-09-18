# Fix a beat grid

No detector is perfect. A grid at half or twice the true tempo, or one whose beats sit off the kicks, makes every transition on that track wrong, and the fix is to correct the grid by ear against the track itself.

## Recognize a wrong grid

- The metronome in the grid editor clicks off the kicks, or drifts away from them over the length of the track.
- A track plays at twice or half its speed in the mix, because the playlist shows its tempo as about half or double its neighbors' (70 BPM beside tracks at 140, say) and the engine stretches it to the mix tempo by that ratio.
- `dermixen library query` shows a tempo you know is wrong, or a grid confidence well under one in the record's JSON.

## Correct the grid in the window

1. Select the track and press **Edit grid**. The editor opens beneath the track's controls, with a strip showing the track's own waveform and the grid's beats over it: the beat that starts a bar in yellow and the other three in grey.
2. Press **Play track**. The track plays on its own, at its own speed, with a click on every beat of the grid. **Metronome** turns the click on and off at any moment.
3. Fix an octave error first. **Halve tempo** and **Double tempo** fix a grid at twice or half the true tempo, keeping beat zero where it is.
4. Put the beats on the kicks. Press on the strip and drag left or right, and every beat slides with the pointer. The click follows within a tenth of a second, so you hear it land. The wheel scrolls the strip, and the command key with the wheel zooms it, down to one frame of audio per pixel. **Nudge** slides the grid by a number of milliseconds instead.
5. Set the tempo if it's off. Type an exact tempo into **Tempo** and press return, or press **Tap** in time with the music four or more times and the editor takes the tempo from the taps.
6. Scroll to the end of the track and play it there. A grid whose tempo is a fraction off drifts off the kicks by the end of a long track, and that's where the drift shows.

A grid is one tempo and one beat zero for the whole track. A track whose own tempo drifts, like a recording played by hand, can't be pinned down at several points, and the app doesn't accommodate it: `DESIGN.md` lists variable-tempo source material among the non-goals.
7. Press **Apply**. The corrected grid goes onto the track as one undoable edit and the editor closes. **Cancel** closes it and changes nothing.

Starting the audition stops the mix, since the machine has one audio output, and starting the mix stops the audition. Selecting another track hides the editor and leaves the audition playing until you select the track again and press **Stop track**.

## Give the grid from the command

When you know a track's tempo and where its first beat falls, pass them to `mix add`:

```
dermixen mix add set.dmx track.mp3 --bpm 138 --first-beat 0.512
```

The grid is taken as given rather than analyzed. Because the anchors analysis placed belong to the analyzed grid, they aren't used: the intro anchor is beat zero and the outro anchor is the last whole beat of the track, unless you give `--intro` and `--outro`. `dermixen analyze track.mp3 --bpm 138 --first-beat 0.512` prints the record the given grid produces without writing anything.

## What happens to the correction

Every grid you apply in the window is written as an annotation file to `dermixen/corrections` in your data folder, beside the [library file](../library.md), in the format the scoreboard reads. Undoing the edit leaves the file, since the file records what you judged by ear. [Measure the analyzers on your own corrections](measure-the-analyzers.md) turns those files into a scoreboard, and [The `dermixen-app` window](../window.md#the-beat-grid-editor) is the reference for every control in the editor.
