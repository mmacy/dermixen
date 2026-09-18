# aubio-sys

The beat and tempo tracker from the aubio library, vendored as C and made callable from Rust.

Dermixen uses aubio as the baseline row on the analyzer scoreboard. The row is the score a bespoke beat tracker has to beat before that bespoke tracker is worth shipping, and aubio is the tracker the rest of Dermixen leans on until a bespoke one beats it.

The crates that make up the app itself forbid unsafe code, and calling into a C library needs it, so all of the risk of these calls sits in `src/lib.rs`, which is short enough to read in one sitting. Four crates allow unsafe code: this one, `crates/signalsmith-sys` for the time-stretcher, `crates/keyfinder-sys` for the key detector, and `crates/macos-documents-sys` for the documents macOS asks the app to open.

## What is vendored

`vendor/aubio` holds part of the source of aubio 0.4.9, from [aubio.org](https://aubio.org/) and [github.com/aubio/aubio](https://github.com/aubio/aubio), compiled from source on every build, so there is no prebuilt library to trust and nothing to download.

Only the files the beat tracker reaches are vendored: the vector types, the maths helpers, the phase vocoder and its built-in fast Fourier transform, the onset description and peak picking, the beat tracker itself, and the logging and histogram helpers those use. The rest of aubio, which includes pitch detection, note tracking, mel-frequency coefficients, and the audio file readers, is not here, because no part of Dermixen calls any of those.

aubio is under the GNU General Public License version 3, and `vendor/aubio/COPYING` is aubio's own copy of that license. Dermixen is under the same license, which is what makes linking aubio into Dermixen allowed. `vendor/aubio/AUTHORS` names the people who wrote aubio, and `vendor/aubio/VERSION` records the release these files came from.

## How the build works

`build.rs` compiles the vendored files with the `cc` crate into one static library. aubio normally has a configuration step that probes the machine for optional libraries and standard headers; there is no probe here. Every header the build tells aubio it has is part of C99, and none of the optional accelerators are turned on, so aubio falls back to the fast Fourier transform it ships with. Building the same way on every machine makes the analysis come out the same on every machine, which matters because the scoreboard compares numbers taken on different machines. On everything except macOS and Windows the build also asks the linker for the system maths library, which is where older releases of the GNU C library and every release of musl keep the floating-point functions aubio calls.

## What the Rust side offers

`src/lib.rs` wraps aubio's tempo object in one type, `Tempo`. A caller creates a `Tempo`, pushes a track through that `Tempo` one block of mono samples at a time, and gets back the position of each beat aubio hears, along with the tempo aubio settled on and how confident aubio is in that tempo. aubio's confidence is a ratio with no upper limit, so `Tempo` clamps the figure into the range from zero to one before handing it over. Nothing else from aubio is exposed, because nothing else is called.

`Tempo` holds a pointer into memory aubio allocated, frees that memory when the `Tempo` is dropped, and never hands the pointer out, so safe Rust has no way to misuse the pointer.

`Tempo::new` checks the sample rate before it reaches aubio, and hands back nothing for a rate outside 8,000 to 384,000 samples per second. That range is what keeps aubio out of a loop that never ends. aubio works out how many steps of analysis cover about six seconds of audio, as 5.8 times the rate divided by the hop, and rounds that count up to a power of two by doubling a 32-bit number until it reaches the count. A count above 2,147,483,648 makes the doubling pass the largest 32-bit number, wrap to zero, and double zero for as long as the program runs. The largest count the range allows is 5.8 times 384,000 over a hop of one, which is 2,227,200. aubio refuses a hop of zero itself, before it works the count out.
