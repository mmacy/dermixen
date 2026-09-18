# Level the tracks

Masters differ in loudness by ten decibels or more across your tracks, so a mix built from files at their own levels jumps at every transition. Dermixen levels each track with a gain written into the mix document when the track joins the mix.

## See the gain each track got

```
dermixen mix show set.dmx
```

The fourth column of each line is the track's gain, like `+3.7 dB` or `-6.7 dB`. The rule is the smaller of two differences: minus fourteen loudness units relative to full scale (LUFS) minus the track's integrated loudness, which brings the track to the target, and minus one decibel true peak (dBTP) minus the track's true peak, which stops a quiet track with sharp peaks from being raised into clipping. A loud track is turned down by exactly what brings it to minus fourteen, and a quiet one is turned up by that much or by as much as its peaks allow, whichever is less.

The gain is added to the track's volume envelope at every moment, so the fades still reach silence where the envelope reaches it.

## Set a gain yourself

```
dermixen mix add set.dmx track.mp3 --gain -3
```

`--gain` writes the given number of decibels instead of the leveling gain. A gain of `0` plays the file at its mastered level. To change the gain of a track already in the mix, edit its `gain_db` field in the mix document, which is JSON. The window shows each track's gain along the top of its lane.

## Measure loudness for tracks that have none

A track whose record in [the library](../library.md) has no loudness gets a gain of `+0.0 dB`, and `mix add` warns you on standard error, naming the file. Scan the track's folder again:

```
dermixen library scan ~/audio/goa
```

The scan decodes each such file, measures it, stores the loudness, and counts the file as completed. Add the track again afterwards, or write the gain by hand. A file the meter finds nothing in (silent, shorter than four tenths of a second, or quieter than minus seventy LUFS) keeps no loudness however often it's scanned, and the warning says so.

## What leveling doesn't do

There's no master volume and no limiter. The target of minus fourteen LUFS and the ceiling of minus one dBTP apply to each track on its own, and the mix is the sum of the leveled tracks. Final level, limiting, and mastering belong to whatever you run on the rendered file afterwards. `DESIGN.md` lists a master volume and mastering among the non-goals.
