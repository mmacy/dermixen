# What you hear is what renders

`DESIGN.md` puts one value above every feature: the app never crashes, never loses work, never renders something different from what you heard, and never claims confidence it doesn't have. Several things about how the app is built follow from that value, and each of them can be checked from outside the code.

## One render path

The engine has one path from a document to samples: stretch each track by the ratio of the mix tempo to its original tempo, apply its EQ, apply its volume and gain, and sum the tracks. A render pulls that path as fast as the machine can go and writes the result to a file. The window's preview, and `dermixen play`, pull the same path in real time and hand the samples to the audio device. There's no second engine tuned for playback.

Preview runs ahead of the device by up to two seconds of rendered audio. When the machine can't keep up, the device runs out of frames and you hear a gap. You never hear different audio, because the frames that arrive late are the same frames. `dermixen play` counts those underruns and reports them, and the window shows the count in its status line.

The rule is checked from the command line: `dermixen play --capture` writes the frames the device would have played to a file, and a render of the same span writes the same samples. An excerpt rendered with `--from` and `--for` contains what a render of the whole mix contains at those positions, so checking one transition checks the finished mix. For a track with keylock off, the excerpt's samples are the whole render's samples. For a track with keylock on, the excerpt is the same music rather than the same samples: the engine joins a track that is already sounding from half a second of the track's own audio rather than from its first frame, which is what lets a preview start in a fraction of a second at any depth into a mix, and the pitch-preserving stretcher's output depends on everything it has been fed, so the beats land in the same places at the same levels and the individual samples differ. The engine's tests hold every beat within a millisecond and within a decibel of the render's, and the level of every quarter second within half a decibel.

## Determinism

A render depends on the document alone. The leveling gain is written into the document when a track joins the mix rather than computed at render time, so a mix renders the same on a machine whose [library](../library.md) is different or missing. The document names each file by the hash of its bytes, and `render` and `play` hash every file before they start: a file that's missing or has changed stops the render before anything is written, with a message that names the file and points at `dermixen mix relink`.

The golden render tests in the workspace hold this in place. A fixed document goes in, and the rendered audio is compared against stored audio, within a tolerance for the floating-point differences between platforms (macOS and Linux don't produce identical samples).

## Not losing work

A save writes the document to a temporary file beside the project file, flushes it to the disk, and only then renames it over the project file, which the file system does in one step. A disk that fills or a machine that stops partway leaves the last complete document in place rather than half of one. `dermixen mix add` rewrites a document the same way, and a render writes its output to a temporary file that's moved into place only when the render completes, so a failed render leaves no partial file.

After every edit, the window writes the whole document to an autosave file beside the project file. A quit that never reached the window, a close while minimized, or a power failure costs at most the edit in progress. Opening a mix beside an autosave that differs from it offers the changes back before anything else happens, and a save that succeeds removes the autosave, since the project file then contains the same document.

A render streams. Each track is decoded when it's first heard and released when it's finished, so the memory a render needs depends on how many tracks overlap at once, not on the length of the mix.

## Not claiming confidence

Every analyzer reports a confidence with its answer, and the app shows what analysis couldn't settle rather than guessing quietly. Metadata guessed from a file name is marked as guessed, and the window draws it in italics. A track with no key shows `no key`, and a track with no measurable loudness gets a gain of zero and a warning that says why. [How analysis is trusted](how-analysis-is-trusted.md) covers where the confidence numbers come from and what they're good for.
