# How a mix fits together

A mix is a playlist with the transitions worked out. Dermixen keeps it as a document with four kinds of thing in it: the tracks in order, two anchors on each track, one tempo curve for the whole mix, and a volume curve and three EQ curves on each track. Every transition you hear is made of those parts and nothing else, and every edit you make, in the window or with the command, changes one of them.

## The playlist is the timeline

Put the tracks in order and that's the mix. There's no arrangement separate from the playlist to keep in step with it. Move a track up or down the playlist and its anchors and curves move with it, so the transitions on either side of it are whatever its new neighbors' anchors and curves make together.

## The anchors are the beatmatch

Every track has two anchors, each on a beat of the track's own beat grid. The intro anchor is the beat where the track lines up with the track before it. The outro anchor is the beat where the track lines up with the track after it. Placing a track is one rule: its intro anchor lands on the same mix beat as the previous track's outro anchor. That's the beatmatch, and the app does it for every pair.

The overlap follows from where the anchors sit. Whatever a track has before its intro anchor plays under the previous track, and whatever it has after its outro anchor plays under the next one. Drag the intro anchor later, or the outro anchor earlier, and the overlap gets longer. Drag either one the other way and the overlap gets shorter. Drag the outro anchor past the end of the track and you get silence before the next one.

The blue `in` line and the orange `out` line on each track's lane are the anchors. Analysis places them when a track is scanned: it finds where the music begins and ends, so the lead-in and the tail fall outside, and puts each anchor on a beat inside that span. [The library](../library.md) keeps those positions. A mix gets its own copy when the track joins it, so moving an anchor in one mix doesn't move it in another, and the next time you add the track to a mix, its anchors start where analysis put them.

## One tempo for the whole mix

The mix has one tempo curve, shown in the mix tempo lane under the tracks, and every track follows it. Each track keeps the tempo analysis found for it, one number for the whole track, and the engine stretches the track from that number to whatever the curve gives. Because every track follows the same curve, two tracks that overlap can't drift apart. Their beats line up because of how they're placed, not because of anything you do to keep them there.

The curve is drawn through nodes, each a tempo at a beat, and between two nodes the tempo changes in a straight line. The mix starts at the first track's own tempo. When a track joins the mix, the preset writes a node on the outgoing track at its outro anchor, at that track's tempo, and a node on the incoming track at the end of the fade, at the incoming track's tempo. The mix ramps from one tempo to the other across the fade and stays there until the next transition. The exception is a cut, which puts a node on each track at its anchor so the tempo changes on the beat.

Tempo nodes belong to tracks, so they move with a track when you reorder the playlist, and you can add your own anywhere. Type a tempo into **Master BPM** and the mix ramps to it from the playhead on, while everything before the playhead keeps the tempo it had. Nothing you do to the tempo breaks the beatmatch, because the engine re-stretches every track the change touches.

The engine picks between two ways of changing a track's speed, by keylock. With keylock on, the pitch-preserving stretcher keeps the pitch where the track was mastered, whatever the mix tempo does. With keylock off, the engine resamples the track and the pitch moves with the speed, the way a record does when you push it. Keylock is on for every track unless you turn it off, with **Keylock** in the window or with `--no-keylock` on `dermixen mix add`.

## The curves are the mixer

Every track has a volume curve and three EQ curves, **Low**, **Mid**, and **High**. Together they're the channel strip of a DJ mixer: a fader and a three-band EQ with full kill on each band. That's the whole strip, and there's no other effect in the app.

A curve is drawn through nodes the same way, each a level in decibels at a beat. Between two nodes the level changes in a straight line. Before the first node the level is the first node's, and after the last node it's the last node's. A curve with no nodes is zero decibels all the way through, which is the track at its own level. Minus ninety decibels is silence: a volume node there mutes the track, and a low node there removes the bass entirely, which is what a bass swap is made of.

Each track also has a gain, one number in decibels that the app sets from the track's measured loudness when the track joins the mix, so that your tracks sit at one level however they were mastered. The gain is added to the volume curve at every moment, and it shows along the top of the lane. [Level the tracks](../how-to/level-the-tracks.md) gives the rule and how to set your own.

Tempo isn't one of a track's curves. Tempo belongs to the mix.

## A transition is nothing but nodes

When a track joins the mix, a preset writes nodes into the outgoing track and the incoming one: tempo nodes that bring the two to one tempo, volume nodes that fade one out and the other in, and, for a bass swap, low-band nodes that move the bassline across at the middle of the fade. That's all a transition is. There's no transition object behind the nodes, so anything a preset wrote you can drag, add to, or delete in the window like a node you drew yourself.

Four presets exist. The blend, which is the default, brings the incoming track up from silence eight bars before its intro anchor, has it twelve decibels down at the anchor, and takes it to full level twenty-eight bars after, while the outgoing track plays on to its last sample. The beatmix fades one track out and the other in across a span of whole bars, eight unless you give another. The bass swap is a beatmix that hands the low band across at the middle of the span. The cut has no overlap. [Change a transition](../how-to/change-a-transition.md) shows how to choose one and edit the result, and [the project file](../project-file.md#transitions) lists every node each preset writes.

## What the mix file doesn't contain

The mix document contains what a render needs and nothing else. Tags, key, phrase analysis, and loudness live in [the library](../library.md), and the app looks them up by the hash of the track's file. The document names each track by its path and by that hash. The path is where the file was last seen, and the hash is how the app finds a file again after you move it. [Relink moved files](../how-to/relink-moved-files.md) covers what to do when a file moves.

The document has no audio in it either. If you send someone the mix file, they need the tracks too.
