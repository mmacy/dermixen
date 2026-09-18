# The project file

A Dermixen project file contains one mix. It is JSON, written for people and programs alike, and its extension is `.dmx`. The file is a public contract: an agent or a person may write one by hand, and `dermixen` validates it and says exactly which field is wrong when it does not.

Version 1 is the format `dermixen` reads and writes.

## Shape

```json
{
  "version": 1,
  "tracks": [
    {
      "path": "/Users/dermixenuser/audio/goa/comp/VA - Goa Vibes [GV001]/03 Slinky Wizard - Lunar Juice.mp3",
      "hash": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
      "length_samples": 18522000,
      "grid": { "first_beat_sample": 4410, "bpm": 138.0 },
      "anchors": { "intro_beat": 64, "outro_beat": 896 },
      "keylock": true,
      "gain_db": -2.5,
      "volume": [ { "beat": 896, "db": 0.0 }, { "beat": 928, "db": -50.0 } ],
      "eq": {
        "low": [ { "beat": 900, "db": 0.0 }, { "beat": 912, "db": -90.0 } ],
        "mid": [],
        "high": []
      },
      "tempo": [ { "beat": 896, "bpm": 138.0 } ]
    }
  ]
}
```

Every field shown is required except `gain_db`, which may be omitted and then means zero. A field that is not listed here is an error, so that a misspelt name is caught rather than ignored. A key may appear once in an object: a file that gives one key twice is refused rather than read with one of the two values.

Every number has a stated range, given with its field below. The ranges are wide enough that no real mix meets them, and they are what lets `dermixen` promise that a file it accepts lays out, renders, and can be written back.

The file contains what a render needs and nothing else. Tags and musical key belong to [the library](library.md), where the app looks them up by `hash`.

## Top level

| Field | Type | Meaning |
| --- | --- | --- |
| `version` | integer | The format version. The current version is `1`, and a file of any other version is refused. |
| `tracks` | list | The playlist, in order. The playlist is also the timeline: the order of this list is the order of the mix. It may be empty. The tracks together must lay out as a mix of at most 24 hours, measured from the first sample heard to the last. |

## A track

| Field | Type | Meaning |
| --- | --- | --- |
| `path` | string | Where the audio file was last seen. It must not contain a NUL character, since no file system call accepts one. |
| `hash` | string | The BLAKE3 hash of the audio file's bytes, as 64 hexadecimal digits. Dermixen writes lowercase and reads either case. The hash identifies the file when it has moved, so `path` is a hint and `hash` is the identity. |
| `length_samples` | integer | The length of the audio in samples at 44.1 kHz. From zero to 238140000, which is the 90 minutes a track may last. |
| `grid` | object | The track's beat grid. See below. |
| `anchors` | object | Where the track joins its neighboring tracks. See below. |
| `keylock` | boolean | `true` keeps the track's pitch when its speed changes. `false` lets the pitch move with the speed, as a record does. |
| `gain_db` | number | A gain in decibels applied to the whole track on top of its volume envelope, which is how volume leveling brings every track to one loudness. From `-144.0` to `24.0`. See "The gain" below. May be omitted, which means `0.0`. Dermixen always writes it. |
| `volume` | envelope | The volume envelope. See below. |
| `eq` | object | Three envelopes, `low`, `mid`, and `high`, one per EQ band. |
| `tempo` | list | This track's nodes on the mix tempo curve. See below. |

### Positions and units

Positions within a track are either samples from the first sample of the audio file or beats of the track's own grid. Which unit a field uses is part of its name: `first_beat_sample` and `length_samples` are samples, and `intro_beat`, `outro_beat`, and every `beat` field are beats. Beats may be fractional except where this page says they must be whole. Beat zero is the first beat of the grid, and beats before it are negative. Every beat in a file, whether an anchor, a tempo node's position, or an envelope node's position, is from -10000000 to 10000000. Whole beats of that size add without rounding over any number of tracks, which is what keeps the layout exact.

Levels are in decibels, where `0.0` leaves the signal unchanged. Every level, whether a `gain_db` or an envelope node's `db`, is from `-144.0` to `24.0`. Any level at or below `-90.0` is silence: the engine contributes nothing from a track at that level, rather than an inaudible residue.

### The grid

| Field | Type | Meaning |
| --- | --- | --- |
| `first_beat_sample` | integer | The position of beat zero, in samples. Beat zero is treated as the start of a bar. It sits within 238140000 samples of the track's first sample in either direction, which is the 90 minutes a track may last. |
| `bpm` | number | The track's original tempo, which analysis finds once and which never changes. From 20 to 999 beats per minute, which is the range the grid editor accepts. Every track has one constant tempo. |

The engine stretches a track by the ratio between the mix tempo curve and this `bpm`. The playlist shows both numbers.

### The anchors

| Field | Type | Meaning |
| --- | --- | --- |
| `intro_beat` | number | The beat that the previous track's outro anchor lines up with. Must be a whole beat from -10000000 to 10000000. |
| `outro_beat` | number | The beat that the next track's intro anchor lines up with. Must be a whole beat from -10000000 to 10000000. |

The first track's beat zero is at mix beat zero. Every later track is placed so that its `intro_beat` falls on the same mix beat as the previous track's `outro_beat`. That single alignment is what places every track, and the overlap between two tracks follows from it: a track plays in full, so the audio before its intro anchor is heard under the previous track and the audio after its outro anchor is heard under the next one. What is actually audible is up to the envelopes. Moving the intro anchor later, or the outro anchor earlier, lengthens the overlap.

An anchor may lie before the track's first beat or after its last. An outro anchor past the end of the track leaves silence before the next track begins. The outro anchor belongs after the intro anchor: `dermixen mix add` and every edit in the timeline refuse a pair in the other order, though a file containing one is still read.

The anchors in the file belong to this mix. The library keeps the positions analysis found, which is where a track's anchors start from when it is added to a mix, and dragging an anchor in one mix changes only that mix.

### The gain

`gain_db` is added to the volume envelope's level at every moment of the track, so a track with an empty envelope and a gain of `-2.5` plays 2.5 decibels below its file throughout, and a fade still reaches silence where the envelope reaches it. Whether the track is silent depends on the envelope alone, and a level at or below `-90.0` in the envelope contributes nothing whatever the gain.

Volume leveling writes the gain when a track joins a mix, from the loudness the library measured for the file. The rule is the smaller of two differences: minus fourteen LUFS minus the track's integrated loudness, which brings the track to the target, and minus one dBTP minus the track's true peak, which keeps a quiet track with sharp peaks from being raised into clipping. A loud track is therefore turned down by exactly what brings it to minus fourteen, and a quiet one is turned up by that much or by as much as its peaks allow, whichever is less. The result stops at `-144.0` or `24.0` if the two differences reach past the range a level may take. A file whose record has no loudness gets a gain of zero, and so does a file whose measurement is not a finite number. `dermixen mix add --gain` writes a given number instead, and the field can be edited by hand like any other. The mix tempo and the envelopes are unaffected either way.

### Envelopes

An envelope is a list of nodes, in any order, each with a `beat` from -10000000 to 10000000 and a `db` from `-144.0` to `24.0`. Between two nodes the level changes in a straight line in decibels. Before the first node the level is the first node's level, and after the last node it is the last node's level. An empty list means the level is `0.0` throughout. Two nodes at the same beat are an error.

### Tempo nodes

The mix has one tempo curve, and every track plays at whatever tempo the curve gives at each moment, so overlapping tracks cannot drift apart. The curve is built from the `tempo` lists of all the tracks: each node's `beat` is a beat of its own track, and placing the track on the timeline places the node. Nodes therefore travel with a track when the playlist is reordered.

Each node is `{ "beat": number, "bpm": number }`, with `beat` from -10000000 to 10000000 and `bpm` from 20 to 999 beats per minute, the same range a grid's tempo has. A node's `beat` need not lie within its track. The node lands wherever the arithmetic below puts it.

The curve on the timeline is assembled like this:

1. The first track's beat zero is mix beat zero, and the curve always starts with a node there set to the first track's original `bpm`. That is how the first track sets the starting tempo, and it is not written in the file.
2. Each track's origin is the mix beat of its beat zero. The first track's origin is zero, and each later track's origin is the previous track's origin plus the previous track's `outro_beat` minus this track's `intro_beat`.
3. Every node in every track's `tempo` list is placed at its track's origin plus its `beat`, and the whole set is sorted by mix beat. Nodes that share a mix beat keep playlist order, with the starting node first, and the later of two nodes at one beat is the tempo at that beat. A node that would land before mix beat zero is placed at mix beat zero, since the mix cannot change tempo before it begins.

Between two consecutive nodes the tempo changes in a straight line over time, so a ramp from tempo `a` to tempo `b` across `n` beats lasts `120 * n / (a + b)` seconds. After the last node the curve stays at the last node's tempo. A mix with no tempo nodes of its own therefore plays at the first track's original tempo throughout.

Worked through for the example file above, extended with the second track shown in `tests/fixtures/mix/valid/two-tracks.dmx`: the first track has `outro_beat` 896 and a node `{ "beat": 896, "bpm": 138.0 }`. The second track has `intro_beat` 32 and a node `{ "beat": 64, "bpm": 140.0 }`. The second track's origin is `0 + 896 - 32 = 864`, so its node lands at mix beat `864 + 64 = 928`. The curve is then: 138 BPM from the start, still 138 at mix beat 896 where the transition begins, a ramp to 140 by mix beat 928, and 140 from there on. The ramp spans 32 beats and lasts `120 * 32 / 278` seconds, a little under fourteen seconds.

## Transitions

When `dermixen mix add` appends a track to a playlist that already contains one, it joins the two with a transition preset, which writes plain nodes into both tracks. A transition is nothing beyond the nodes it writes, so it can be edited freely afterwards. A node already at one of the exact beats a preset writes is replaced, and every other node is left alone. Four presets exist.

### blend

The default. A long rise into the incoming track over the outgoing track's last minutes, with the outgoing track playing to its own last sample rather than fading out. Its shape is fixed, and `--bars` does not change it:

- The outgoing track gets a tempo node at its outro anchor set to its own original tempo, and the incoming track gets a tempo node eight bars after its intro anchor set to its own, so the mix tempo ramps from one to the other across the first eight bars after the aligned anchors.
- The incoming track's volume gets eight nodes placed relative to its intro anchor: `-90.0` thirty-two beats before it, `-24.0` twenty-four beats before, `-16.0` twelve beats before, `-12.0` at the anchor, `-5.5` thirty-two beats after, `-2.5` sixty-four beats after, `-0.5` ninety-six beats after, and `0.0` one hundred and twelve beats after. The track is heard from eight bars before its anchor and reaches full level twenty-eight bars after it. A node that lands before the track's first sample is valid and does nothing, because the envelope's value before its first node is that node's value.
- The outgoing track's volume gets five nodes across the stretch from its outro anchor to its last sample: `0.0` at the anchor, `-1.0` a quarter of the way along, `-2.5` halfway, `-4.5` three quarters of the way, and `-7.0` at the last sample. The three middle nodes are rounded to whole beats, and the last one sits on the last sample whether or not that is a whole beat. The track is never faded to silence. It ends when its file does, seven decibels down.
- How much of the track is left after its outro anchor decides how many of those five nodes are written. A tail of four beats or more gets all five, because rounding then gives each of the three middle nodes a whole beat of its own strictly between the anchor and the last sample. A tail under four beats gets two nodes instead, `0.0` at the anchor and `-7.0` at the last sample, so the track plays at full level up to its anchor. An outro anchor at or past the last sample gets a single `0.0` node at the anchor. `mix add --bpm` without `--outro` puts the outro anchor on the track's last whole beat, which is the short-tail case.
- EQ envelopes are left alone.

The blend counts as one hundred and twelve beats long, the rise after the anchor, for the rule that a transition must fit between the outgoing track's anchors. The eight bars before the anchor and the ease afterwards are not counted, because a node before a track's first sample is harmless and the ease belongs to the outgoing track alone.

### beatmix

Across a span of whole bars from the aligned anchors, eight bars unless the person gives `--bars`:

- The outgoing track gets a tempo node at its outro anchor set to its own original tempo, and the incoming track gets a tempo node at the end of the span set to its own, so the mix tempo ramps from one to the other across exactly the fade.
- Each track gets a six-node volume fade that approximates a fade that is straight in amplitude. The fade in has nodes at fractions 0, 1/8, 1/4, 1/2, and 3/4 of the span and at its end, with `-90.0` and then the decibel equivalents of amplitudes 1/8, 1/4, 1/2, 3/4, and 1. The fade out is its mirror in time: nodes at fractions 0, 1/4, 1/2, 3/4, and 7/8 of the span and at its end, with `0.0`, then the decibel equivalents of amplitudes 3/4, 1/2, 1/4, and 1/8, and then `-90.0`.
- EQ envelopes are left alone.

### bass-swap

Everything `beatmix` writes, and in addition the low band is handed from the outgoing track to the incoming one at the middle of the span, which is a whole beat because the span is whole bars. The incoming track's `low` envelope gets `-90.0` one beat before the swap beat and `0.0` at it. The outgoing track's `low` envelope gets `0.0` one beat before the swap beat and `-90.0` at it. Only the outgoing bassline is heard before the swap, and only the incoming one after it.

### cut

No overlap to speak of. The outgoing track's volume gets `0.0` a quarter beat before its outro anchor and `-90.0` at it. The incoming track's volume gets `-90.0` a quarter beat before its intro anchor and `0.0` at it. Each track gets a tempo node at its anchor set to its own original tempo, so the tempo changes at the shared beat rather than ramping: the later of the two nodes at that beat is the incoming track's, and its tempo applies from there.

### The overlap length and the anchors

Analysis places each track's anchors for an eight-bar overlap, choosing where the outgoing track should be gone. When `--bars` asks for another length and the outro anchor was not given by hand, the anchor moves so an overlap of that length still ends at that same point: earlier by four beats for every bar beyond eight, later by four beats for every bar short of it. The anchor moves this way for every preset, the blend included, though only `beatmix` and `bass-swap` take their span from the length. The intro anchor does not move with the length.

The example file above was written by hand rather than by a preset, which is why its fades have fewer nodes and stop at `-50.0` rather than the silence floor. Both are valid files.

## Errors

When a file is refused, the error names the field as a path into the document, then says what is wrong there, in the form `tracks[2].grid.bpm: must be a tempo from 20 to 999 beats per minute, not 1e12`. A file that is not JSON at all has an empty field path, and a mix that lays out longer than 24 hours is named by the `tracks` field, since no single track is at fault. The fixtures under `tests/fixtures/mix/invalid/` and `tests/fixtures/mix/out-of-range/` show one example of each kind of refusal, and the sidecar `.expected.json` next to each one says what the error must name.
