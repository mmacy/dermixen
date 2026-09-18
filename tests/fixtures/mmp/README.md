# MixMeister project fixtures

Six MixMeister Fusion project files, each a mix of whole tracks. Together they hold 78 tracks. `python3 tools/mmp_dump.py FILE` prints one as text, and `--json` prints it as JSON. The track counts and tempo ranges below are what the dump reports. The tempo range is the range of the tracks' own tempos, which MixMeister calls the original BPM.

| File | Tracks | Tempo range |
| --- | --- | --- |
| `slow-ascent.mmp` | 12 | 128.0-136.1 |
| `slow-ascent-2.mmp` | 10 | 135.9-138.8 |
| `emergent-thisness.mmp` | 13 | 100.1-143.9 |
| `epcc-2.mmp` | 10 | 132.6-147.0 |
| `goat-ranch-monthly-2015-03.mmp` | 11 | 144.4-149.0 |
| `global-goa-party-3.mmp` | 22 | 111.7-150.0 |

`tools/anchor_truth.py from-mmp` builds the anchor ground truth in `tests/ground-truth/anchors/` from these files: the grid fitted to each track's section markers, the cue as the intro anchor, and the measure marker as the outro anchor. `docs/ground-truth.md` describes the labels and the scoreboard that reads them.

Two of the files show how a mix tempo curve follows the tracks. In `slow-ascent.mmp` the mix tempo lane starts at 128.0 beats per minute and climbs through the set to 136.1. In `slow-ascent-2.mmp` it starts at 135.9 and reaches 138.8 by the fifth track, and every track's own tempo lies inside that three-beat range.

Every track path inside the files is a Windows path on a `C:`, `D:`, or `E:` drive or on a `\\media` network share. Those drives exist on no machine that runs the tests, and the audio is not in the repository. `tools/anchor_truth.py` maps each root onto a folder below the audio root its `link` command is given.
