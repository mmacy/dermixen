# How analysis is trusted

An analyzer is trusted by its score against tracks whose answers are already known. Reading its code says whether it is sound, and the score says whether it is right. The scoreboard makes analyzer quality a number, and that number decides what ships.

## The scoreboard

`dermixen scoreboard DIR` runs every built-in analyzer over a folder of audio files with annotation files beside them and prints one row per analyzer: how many tracks it ran on and failed on, its accuracy, and the seconds it took per track. For tempo, an estimate within four percent of the annotated tempo counts as right (that's the tolerance the music information retrieval community uses), and a second column also counts estimates at double, half, triple, or a third of the tempo, so the gap between the two columns is the octave-error rate. For beats, a beat counts as found within seventy milliseconds of an annotated beat. For keys, one column counts exact matches and another gives partial credit for a key a fifth away or the relative or parallel key. [Ground truth for the scoreboard](../ground-truth.md) defines every column and every annotation format.

Ground truth comes from two places. The GiantSteps tempo and key datasets are annotated Beatport previews of electronic dance music, which is the music the app is for. Your own corrections are the second source, and they grow with use.

## Baselines and bespoke analyzers

Well-known open-source analyzers run inside the scoreboard as baselines: the aubio beat tracker for beats and tempo, and libkeyfinder for key. They're always available to ship, so the app is shippable at every moment, and a bespoke analyzer earns its place only by beating the baseline on the board. A bespoke analyzer that doesn't beat the baseline is discarded.

Bespoke analyzers can win here because the problem is narrow. The baselines solve a general one: any music, often in real time, with no assumptions. Dermixen analyzes whole files offline, and the files are machine-made, constant-tempo, mostly four-on-the-floor music rendered from a grid in a digital audio workstation. Recovering a grid that exists is an easier problem than imposing one on a human performance, and offline analysis can use agreement across the whole track that a real-time tracker can't.

The grids in [the library](../library.md) come from the built-in `pulse` analyzer and its anchors from the built-in `kick` analyzer. The library's keys come from libkeyfinder, because the bespoke key analyzer with profiles tuned for electronic music scores below it on the GiantSteps key set, so the bespoke one stays on the board as the challenger and the baseline ships.

## Downbeats and phrases

Finding beats is a solved problem. To place a transition, the app needs more: which beat starts a bar, and where the eight, sixteen, and thirty-two bar phrases that govern where a transition lands begin. None of the open-source analyzers attempt that, and the bespoke effort goes there. The `shifts` analyzer reads bar lines and phrase starts off the audio and finds section changes where the kick starts or stops and where the levels shift. The window shows what it found as green ticks and amber lines on each lane.

The app shows phrases and doesn't act on them. The analysis crate's `anchors` example runs an analyzer that moves the `kick` analyzer's anchors onto the nearest phrase start, beside `kick` itself, so the two rows show whether snapping brings an anchor closer to a labeled position. That analyzer runs nowhere else. The corrections you make in the window are the labels that settle the question on your own music.

## Confidence

Every analyzer reports how sure it is, from zero to one, and the scoreboard counts sure hits, sure misses, unsure hits, and unsure misses separately. An honest confidence puts the misses among the unsure tracks. Where the numbers show a confidence doesn't separate right from wrong, nothing in the app acts on it.

The known weak spot is lo-fi material, where soft onsets and a half-time feel produce octave errors, a grid at 70 BPM for a track at 140. The `pulse` analyzer weighs its candidates with a tempo prior centered on 130 BPM and an octave wide, which is the first mitigation, and the halve and double buttons in the grid editor are the second. The gap between the two tempo columns on the scoreboard shows how often the error still happens.

## The correction loop

Every beat grid you fix and every anchor you move in the window is written out as an annotation in the scoreboard's format, into a corrections folder beside the library file. The annotation records what you judged by ear at the moment you judged it, so undoing the edit doesn't remove it. Copy the corrections you trust into `tests/ground-truth/anchors/`, and every analyzer is measured against them from then on. An analyzer change that would undo a correction shows up as a lower number on the board. [Measure the analyzers on your own corrections](../how-to/measure-the-analyzers.md) covers running that scoreboard.
