# Check a transition by ear

A transition is a stretch of the mix, and both `render` and `play` take a span, so you can hear one transition without rendering the whole mix.

## Find where the transition is

```
dermixen mix show set.dmx
```

```
  1  VARIOUS ARTISTS - FILL YOUR HEAD WITH PHANTASM VOL.2 - 07 L.S.C. - Big Brain.mp3   145.00 bpm    +3.7 dB  intro       32  outro      960  0:00.0 to 7:53.5
  2  202 Total Eclipse & Simon Posford - Sound Is Solid (Remix).mp3   139.85 bpm    -6.7 dB  intro       64  outro      816  6:10.5 to 12:55.8
2 tracks, 12:55.8 long, 34214245 samples
```

The last two numbers on each line are where the track's file begins and ends on the timeline. The transition lies inside the stretch where the two files overlap, from 6:10.5 to 7:53.5 here, and a span that covers that stretch is sure to contain it. The transition is the default `blend`, so the second track's volume curve starts rising well before its intro anchor and keeps rising well after it. To find where the two anchors meet, multiply the incoming track's intro beat by sixty, divide by the mix tempo there, and add the time of the track's beat zero, which is `first_beat_sample` in the mix document divided by 44100 and usually a fraction of a second: 64 beats at 145 beats per minute is 26.5 seconds, plus 0.3 seconds to beat zero, so the anchors meet at 6:37. In a render of this mix, the second track's audio rises above silence at 6:24.5, is twelve decibels down at 6:37, where the anchors meet, and reaches full level at 7:25. The first track keeps playing underneath the whole way, easing down rather than fading to silence, until its file ends at 7:53.5. Its audio stops about a second earlier, around 7:52.5, because the MP3 ends in a second of digital silence.

## Render the span to a file

```
dermixen render set.dmx transition.wav --from 6:00 --for 2:00
```

`--from` is where to start and `--for` how much to render, each as minutes and seconds like `6:00` or `6:00.5`, or as seconds like `360`. The tracks that end before the span begins, or start after it ends, are never decoded. The frames written are the frames a render of the whole mix contains at those positions. For a track without keylock they are those samples exactly. For a track with keylock they are the same music, with the beats where the whole render puts them and the levels the whole render's. An excerpt sounds as the transition sounds in the whole mix.

## Play the span through the speakers

```
dermixen play set.dmx --from 6:00 --for 2:00
```

The command decodes the tracks that sound at the start of the span, about a third of a second for each seven-minute MP3, then plays through the default audio device and returns when the last frame has been played. One line every ten seconds on standard error says where in the mix the sound has reached. The line printed at the end says how much was played and either `no underruns` or how many times the device needed frames that weren't ready. An underrun is heard as a gap, never as different audio.

## Prove that play and render agree

```
dermixen play set.dmx --from 6:00 --for 2:00 --capture played.wav
dermixen render set.dmx rendered.wav --from 6:00 --for 2:00
cmp played.wav rendered.wav
```

`--capture` writes the frames the device would have played to a file instead of playing them. The two files contain identical samples, which is the rule that what you preview is what you render, checked from the command line.

## Check it in the window

Open the mix with `dermixen open set.dmx`, click the ruler a little before the transition, and press the space bar. The window plays the same render path. [Editing a mix in the window](../tutorials/editing-a-mix-in-the-window.md) covers playing and moving the playhead.
