# Anchor ground truth

`anchors/` holds one `.anchors` annotation file per track. Each file gives the track's beat grid, where its music begins and ends, where its intro and outro anchors belong, the bars a transition was aligned to as `phrase` lines, and any bars at which the arrangement changes as `section` lines. Each label names its source: `ear` for a position confirmed by listening, `mixmeister` for a position read from a MixMeister project file, and `plot` for a value MixMeister's own analysis wrote into the track's plot file.

The audio is not in the repository. Each annotation's `file` line names the audio file's path below an audio root, and `tools/anchor_truth.py link` copies the annotations into a directory with the audio linked beside them. That linked directory is what `dermixen scoreboard` and the `anchors`, `grids`, and `phrases` examples of `dermixen-analysis` run on.

`tools/anchor_truth.py from-mmp` writes the annotations from the MixMeister project files under `tests/fixtures/mmp/`, `from-mxm` writes them from MixMeister plot files, and `phrases` adds the `phrase` lines. `docs/ground-truth.md` describes the format, the sources, and the metrics the anchor scoreboard reports.

One label contradicts its own source. The plot file for `03 Nervasystem - Snaafu` puts the start of the outro range at 452.61 seconds, nine seconds after the effective ending it gives at 443.47 seconds, so its thirty-two-beat outro range runs into the tail of the track. The annotation keeps both values as the file states them, and the loader reports the contradiction when the directory is loaded.
