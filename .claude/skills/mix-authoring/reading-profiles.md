# Reading the profiles

What `tools/track_profile.py` and `tools/grid_check.py` print, and what each pattern in it means for placing anchors. The runs below are from real tracks, cut to the rows that matter.

## A track's ending

`track_profile.py beats` prints one line per window of `--per` beats: the beat the window starts on, the level in dB below full scale, the pulse of the low band in dB, and a bar that grows with the level. The pulse is how far the low band swings between the kick's hit and the gap after it, folded over the beats of the window, so it tells a kick from a pad without depending on where the beats fall.

```
beat   936  full  -11.8  pulse   13.4  ############################
beat   944  full  -11.8  pulse   12.8  ############################
...
beat   984  full  -11.9  pulse   13.2  ############################
beat   992  full  -13.4  pulse   24.8  ##########################
beat  1000  full  -13.5  pulse   23.5  ##########################
beat  1008  full  -13.4  pulse   24.3  ##########################
beat  1016  full  -13.5  pulse   24.0  ##########################
beat  1024  full  -12.0  pulse   12.9  ############################
beat  1032  full  -11.5  pulse   12.4  ############################
beat  1040  full  -11.6  pulse   12.6  ############################
beat  1048  full  -11.6  pulse   12.5  ############################
beat  1056  full  -24.0  pulse   26.3  ###############
beat  1064  full  -50.9  pulse    2.3
beat  1072  full  -65.4  pulse    2.5
```

The body of this track runs at -12 dB with a pulse of 12 to 13: a kick with a bass line rolling between the hits, which fills the gaps and keeps the swing down. From 992 to 1016 the pulse doubles and the level drops 1.5 dB, which is the bass line leaving and the kick standing alone. The body returns at 1024, and the track stops inside the window that starts at 1056. The kick plays to the end, so the outro anchor can sit as late as about beat 1056. The sections change at 992, 1024, and 1056, so the phrase grid is 0 mod 32. The outro anchor goes at least 96 beats before 1056 on that grid, which is 960.

Compare a track's sections with each other and not with another track's numbers. The pulse of a kick alone runs 20 dB or more, a kick over a rolling bass line runs 8 to 13, and a track with no kick in a window runs under about 10. The jump within one track is what marks the change.

```
beat  1112  full  -11.1  pulse    8.4  ############################
beat  1120  full  -10.6  pulse    6.0  #############################
beat  1128  full   -9.6  pulse    4.6  ##############################
beat  1136  full  -11.4  pulse    9.4  ############################
...
beat  1192  full  -12.1  pulse    8.4  ###########################
beat  1200  full  -28.1  pulse    4.1  ###########
```

This track's body pulses at 8 to 9 because its bass never stops. At 1120 and 1128 the pulse halves while the level rises a decibel: a bass fill with the kick gone for two windows, not a breakdown. The kick is back at 1136 and the track ends inside the window at 1200. A four-bar fill closes the phrase that ends at 1136, so the phrase grid is 16 mod 32, and an outro anchor at 1104 puts the incoming track at -0.5 dB as the file ends.

## A track's opening

```
beat     0  full  -18.4  pulse    3.2  #####################
beat     8  full  -22.7  pulse    8.9  #################
beat    16  full  -22.1  pulse    6.7  #################
beat    24  full  -24.9  pulse   12.4  ###############
beat    32  full  -15.8  pulse   23.1  ########################
beat    40  full  -15.8  pulse   23.4  ########################
beat    48  full  -15.8  pulse   21.8  ########################
beat    56  full  -15.8  pulse   22.4  ########################
beat    64  full  -19.4  pulse    7.3  ####################
beat    72  full  -19.9  pulse    6.1  ####################
beat    80  full  -14.2  pulse   16.4  #########################
beat    88  full  -14.6  pulse   16.2  #########################
...
beat   216  full  -13.3  pulse   15.6  #########################
```

The first 24 beats are an intro with no kick, the kick is heard from 24, and it arrives in full at 32, alone, for 32 beats. At 64 the track breaks for 16 beats, and at 80 the kick returns with the bass and the level settles at -14 dB, which is the body. The intro anchor goes at 80: the kick is under it, nothing breaks in the 112 beats after it, and the pre-roll from 48 to 80 is inside the file and contains music that builds. Beat 32 would fail twice over. Its pre-roll would begin at beat zero, and the break at 64 would fall 32 beats into the rise, where the incoming track is at -5.5 dB and expected to be gaining.

## A rendered clip

`track_profile.py seconds` prints the level on a clock rather than a grid, for a clip that contains two tracks and no single grid.

```
   60.0s  full  -15.9  ########################
   64.0s  full  -14.6  #########################
   68.0s  full  -15.1  ########################
   72.0s  full  -15.6  ########################
   76.0s  full  -14.5  #########################
   80.0s  full  -14.7  #########################
   84.0s  full  -15.2  ########################
   88.0s  full  -16.5  #######################
   92.0s  full  -16.3  #######################
   96.0s  full  -16.5  #######################
  100.0s  full  -15.4  ########################
  104.0s  full  -15.9  ########################
  108.0s  full  -15.3  ########################
  112.0s  full  -16.3  #######################
  116.0s  full  -16.1  #######################
  120.0s  full  -16.1  #######################
  124.0s  full  -15.9  ########################
  128.0s  full  -16.0  #######################
```

The rise starts at 60 seconds and the outgoing track ends at about 124. The level stays between -14.5 and -16.5 dB the whole way, which is a handover that holds. A hole shows as a run of windows 3 dB or more below the windows either side of it. A leveling difference shows as one steady level before the handover and another after it.

## A grid check

`grid_check.py` finds the kick by its pitch sweep, fits a line through the offsets it finds, and reports the line and where the kick starts. This is Total Eclipse - Wailing For A New Life on the grid analysis gave it:

```
kick offset from the grid every 8 beats from 200 to 1100: -180 ms at beat 200, drift -2.1 ms per 1000 beats, 86 of 113 checks within 20 ms of that line, tempo fits 139.861 (given 139.860)
kick starts -181 ms after the grid beat, one kick onset per beat. --first-beat 0.127 would put it at +20 ms
  beat   200:   -180 ms
  beat   256:   -180 ms
  ...
  beat  1064:   -181 ms
```

The tempo is right: the drift is two milliseconds over a thousand beats and the fitted tempo is within a thousandth of the given one. The checks off the line sit in the track's two breakdowns, where there is no kick to find. The grid is not right. The kick starts 181 ms before every grid beat, which at this tempo is 42 percent of a beat, and what sits on the grid beat is the offbeat bass note, as loud as the kick in the low band. The level profile of this track reads as a clean kick throughout, and so does the low band's peak, because both are measuring the bass. Mixed on this grid, the track's kicks land between the other track's. The second line is the fix: `mix add --bpm 139.860 --first-beat 0.127` with the anchors given by hand.

A grid that sits on the kick reads like Koxbox - Space Traveller:

```
kick offset from the grid every 8 beats from 200 to 1100: +20 ms at beat 200, drift -2.7 ms per 1000 beats, 107 of 113 checks within 20 ms of that line, tempo fits 140.006 (given 140.005)
kick starts +10 ms after the grid beat, one kick onset per beat. --first-beat 1.209 would put it at +20 ms
```

The kick's sweep starts within a few tens of milliseconds after the grid beat on every grid that sits on the kick, and +20 ms is the reference offset the report uses, so the suggested first beat moves a grid that reads +10 by ten milliseconds. A suggestion that close to the given first beat is not worth taking. One that moves the grid by a quarter or half a beat is.

A grid that does not hold in the other way shows a drift of tens of milliseconds or more per thousand beats, and the fitted tempo is the one to give `mix add --bpm`. Many checks off the line along the whole stretch, with no breakdowns to account for them, is a grid that does not describe the track at all, and the track needs its grid fixed in the window before it goes in a mix (`docs/how-to/fix-a-beat-grid.md`).

## A rendered clip's kicks

The same program on a rendered handover checks that the two tracks' kicks land together. It takes the incoming track's tempo and the anchor time that `render --handover` printed, over the beats where both tracks are near full level. This is the handover into Wailing For A New Life as it was first rendered, with the grid above:

```
kick offset from the grid every 8 beats from 48 to 112: -180 ms at beat 48, drift -2.4 ms per 1000 beats, 6 of 8 checks within 20 ms of that line, tempo fits 139.861 (given 139.860)
two kick onsets per beat: -180 ms and +20 ms, 200 ms apart
```

Two kick onsets per beat, 200 ms apart, is two grids that disagree by that much. The one at +20 is the outgoing track's kick, on the anchor grid, and the one at -180 is Wailing's. The same handover with Wailing's first beat corrected:

```
kick offset from the grid every 8 beats from 48 to 112: +20 ms at beat 48, drift -2.8 ms per 1000 beats, 7 of 8 checks within 20 ms of that line, tempo fits 139.861 (given 139.860)
kick starts +20 ms after the grid beat, one kick onset per beat. --first-beat 73.728 would put it at +20 ms
```

One kick onset per beat is a handover whose kicks land as one. The suggested first beat here is the anchor time the render printed, which is what was given, so there is nothing to change.

## A track's sections

`track_profile.py sections` lists the beats where a track's sections change, so the phrase grid is read from one run. This is the last 320 beats of Wailing For A New Life on its corrected grid:

```
beat  1184  level -4.8 dB  (0 mod 32)
beat  1188  kick leaves   (4 mod 32)
beat  1220  level -2.5 dB  (4 mod 32)
beat  1239  kick returns  (23 mod 32)
beat  1271  level -3.0 dB  (23 mod 32)
beat  1285  level -5.2 dB  (5 mod 32)
```

The level falls by nearly 5 dB at 1184, on the phrase grid, and that is where the outro anchor counts back from: 96 beats before it is 1088. The kick thins out four beats later and comes back for the last bars, and the level steps after 1184 are the track's tail falling away in stages off the grid. A kick that leaves on a phrase line and stays gone is a section, and the `beats` profile around it shows the shape.
