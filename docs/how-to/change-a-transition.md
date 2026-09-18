# Change a transition

A transition is nothing more than nodes: two tempo nodes that bring the tracks to one tempo, and the volume and EQ nodes that fade one track out and the other in. You choose what `mix add` writes when a track joins the mix, and you edit the nodes afterwards in the window or in the file.

## Choose a preset when adding a track

```
dermixen mix add set.dmx track.mp3 --preset bass-swap --bars 16
```

Four presets exist:

- `blend`, the default: the incoming track rises from silence eight bars before its intro anchor, is twelve decibels down at the anchor, and reaches full level twenty-eight bars after it, while the outgoing track plays on to its last sample, easing down to seven decibels below full level there rather than fading out. The mix tempo ramps from the outgoing track's tempo to the incoming track's over the first eight bars after the anchors. EQ is left alone. The shape is fixed, so `--bars` does not change it, and it needs twenty-eight bars between the outgoing track's anchors.
- `beatmix`: the mix tempo ramps from the outgoing track's tempo to the incoming track's across the span, the outgoing track fades out over the span, and the incoming track fades in over it. EQ is left alone.
- `bass-swap`: everything `beatmix` writes, and the low band is handed from the outgoing track to the incoming one at the middle of the span. Only the outgoing bassline is heard before the swap and only the incoming one after it.
- `cut`: no overlap. The outgoing track goes silent at its outro anchor over a quarter beat, and the incoming track starts at its intro anchor over a quarter beat. The tempo changes at that beat rather than ramping.

`--bars` sets the span of `beatmix` and `bass-swap` in whole bars (four beats each), eight if omitted. The span runs from the outgoing track's outro anchor, so a longer span ends later: with `--bars 16` the fade that starts at beat 960 of the outgoing track ends at beat 1024 instead of 992. The option also moves the outro anchor of the track being added, whichever preset is chosen, earlier by four beats for every bar beyond eight and later by four beats for every bar short of it, so that the transition out of this track, when the next track is added with the same length, still ends where analysis chose.

## Set the anchors by hand

```
dermixen mix add set.dmx track.mp3 --intro 64 --outro 896
```

`--intro` and `--outro` are whole beats of the track's own grid, counted from beat zero. The intro anchor is where the track lines up with the previous track's outro anchor, and the outro anchor is where the next track's intro anchor lands. The outro anchor must be a later beat than the intro anchor. `dermixen analyze track.mp3` prints the anchors analysis placed, and `mix show` prints the anchors every track in a mix has now.

## Move an anchor of a track already in the mix

```
dermixen mix move-anchor set.dmx 3 --outro 960
```

The track is named by its position, counting from one, and the nodes of the anchor's transition move with it, as they do when the anchor is dragged in the window. Moving the last track's outro anchor before adding the next track is how the handover into that track is placed on the bar the outgoing track's structure calls for. Every node of a blend's rise moves with the intro anchor, so `mix move-anchor --intro` places a blend's intro anchor as well as `--intro` on `mix add` does. `docs/cli.md` gives the rules under "mix move-anchor".

## Insert a track between two others

```
dermixen mix add set.dmx track.mp3 --position 2 --preset beatmix
```

Inserting a track replaces the transition that joined its two neighbors with two new ones, using the preset and the span given. The nodes of the old transition are removed from both neighbors first.

## Drag the anchors in the window

Press on the orange `out` line or the blue `in` line and drag. The anchor snaps to whole beats, and the tracks after it slide along the timeline as you drag. The tempo, volume, and EQ nodes that belong to the transition move with the anchor. Moving the intro anchor later, or the outro anchor earlier, lengthens the overlap. Dragging the outro anchor past the end of its track leaves silence before the next track.

## Edit the fades and the EQ in the window

1. Choose **Volume**, **Low**, **Mid**, or **High** above the timeline. Every lane shows that curve, with a dot at each node.
2. Click the curve's line where there's no node to add one there. On a track whose curve has no nodes yet, click anywhere on the track to place the first.
3. Drag a node left or right to move it to another beat, or up or down to change its level. The selected node shows its level in decibels.
4. Press delete or backspace to remove the selected node.

Each band of the EQ has full-kill range, so a low node at minus ninety decibels removes the bass entirely. That's how a bass swap is drawn by hand: cut the incoming track's lows until the swap point, then hand the bassline over.

## Change the tempo in the window

**Master BPM** shows the mix tempo at the playhead. Type a tempo into it and press return, and the window writes two nodes just ahead of the point the preview has reached, or of the playhead when nothing is playing: one that pins the curve at the tempo it had, and one a beat later at the new tempo. Everything before them, including everything you've heard, keeps its tempo. To end the excursion, move the playhead further on and type the tempo the mix should return to. To set one node exactly, click it and type into the **Tempo** field that appears beside the master field. To add a node anywhere, click the curve's line in the tempo lane, and it lands on the nearest whole mix beat. Drag a node to move it. Delete or backspace removes the selected tempo node, and removing the nodes **Master BPM** wrote over a stretch returns that stretch to the tempo the transition nodes give.

## Let the pitch move with the tempo

Every track has keylock on unless you turn it off, so the pitch-preserving stretcher keeps the track's pitch wherever the mix tempo takes its speed. `dermixen mix add track.mp3 --no-keylock` adds a track with keylock off, and **Keylock** in the window turns it on or off for the selected track. With keylock off, the track is resampled and its pitch moves with its speed, as a record does.

## Edit the file directly

A mix document is JSON, and every node is a line in it. [The project file](../project-file.md) gives every field and the nodes each preset writes, so a script can write a transition the presets don't offer. `dermixen mix show` validates the file and names the field that's wrong when it doesn't parse.
