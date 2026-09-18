# Editing a mix in the window

The window shows a mix as lanes on a timeline and plays it from any point. You move a transition by dragging an anchor, hear the result as you drag, and undo what you don't like. The mix from [Your first mix](your-first-mix.md) is the starting point.

## Prerequisites

- `set.dmx` from [Your first mix](your-first-mix.md), or any mix document with two or more tracks.
- [The library](../library.md) the tracks were scanned into, which is everything Dermixen has learned about your tracks, kept in one file in your data folder. The window reads the phrase analysis and the library panel from that file.

## Open the mix

```sh
dermixen open set.dmx
```

The command starts `dermixen-app` on the mix and returns. Before the window appears, the app reads and hashes every track's file to check that each is the file the document names, so a mix of lossless files takes a few seconds to show up.

The window opens showing the whole mix:

- One lane per track, in playlist order. Along the top of each lane are the track's position, its file name, its original tempo, whether keylock is on, and its gain. The waveform fills in as the app reads the file.
- A blue line marked `in` at the track's intro anchor and an orange line marked `out` at its outro anchor.
- A faint line at every bar, a green tick at the foot of the lane at every phrase start, and an amber line at every section change. The ticks and the amber lines come from the phrase analysis in the library, so a track that isn't in the library has none.
- The mix tempo lane beneath the tracks, with a dot at each tempo node and the tempo written beside it.
- A ruler above the lanes, marked in minutes and seconds.
- The library panel to the right, listing every track in the library.
- The status line at the foot of the window.

## Play the transition

1. Click the ruler at about 6:00. The playhead, a white line down every lane, moves there.
2. Press the space bar, or **Play**. The status line says `Buffering` while the app decodes the tracks that sound at that point and brings each of them up from the half second of its own audio that precedes the playhead, then `Playing`.
3. Press the space bar again to stop. The playhead returns to where you clicked, so the next play covers the same stretch.

What you hear is what `dermixen render` writes for the same span. The window and the command share one render path.

## Move the outro anchor

1. Move the pointer onto the orange `out` line of the first track. The pointer becomes a horizontal resize arrow.
2. Press and drag left, by a few bars. The second track slides earlier as you drag, because the overlap the two anchors define is what places it. The anchor snaps to whole beats.
3. Let go. The move is one edit in the history. When the preview is playing and the move changes frames it had already rendered ahead, the preview starts over from its position, and the status line says `Started over:` and the reason for five seconds.
4. Click the ruler before the new transition and play it. The handover now begins where you put the anchor.

Moving the outro anchor earlier lengthens the overlap, and moving it later shortens it. The tempo, volume, and EQ nodes that belong to the transition move with the anchor, so the fade still runs from the anchor.

## Undo the move

Press the command key with Z (the control key with Z on Linux). The anchor returns to where analysis placed it, and the second track slides back. Every drag, every added or removed node, every grid correction, every reorder, every keylock change, and every track added from the library is one undoable step.

## Add a track from the library

1. Click the second track's lane, anywhere away from its anchors and its curve. The track is selected, and every row in the library panel whose key mixes well with it turns green.
2. Type part of an artist or title into the panel's search field. The rows narrow to the tracks that match, best first.
3. Click a green row, then **Add selected to mix**. The track joins the mix after the selected track, with the default transition, the blend, on either side of it and the leveling gain its loudness gives. Its lane appears at once, and its waveform fills in a few seconds later.

The new track is selected, so the transitions on either side of it are the ones on screen. **Move up** and **Move down** move it through the playlist, and the delete key removes it.

## Save

Press the command key with S, or **Save**. The status line shows `Unsaved changes` from the first edit after a save until the next one. A save writes the document to a temporary file beside `set.dmx` and renames it into place, so a save that fails partway leaves the last complete document where it was.

Closing the window with unsaved changes brings up a dialog that offers to save and close, close without saving, or cancel. After every edit the window also writes the document to `set.dmx.autosave` beside the mix, so an exit the window never saw costs at most the edit in progress. Opening a mix beside an autosave file that differs from it offers those changes back.

## Next steps

- [Change a transition](../how-to/change-a-transition.md) covers the presets, the nodes on the volume and EQ curves, and the mix tempo.
- [Fix a beat grid](../how-to/fix-a-beat-grid.md) covers the grid editor and its metronome.
- [The `dermixen-app` window](../window.md) is the reference for everything the window shows and every control.
