# keyfinder-sys

libkeyfinder, the key detection library used as the scoreboard baseline for musical key, vendored as C++ and made callable from Rust.

Dermixen uses libkeyfinder as the baseline row on the key scoreboard. The row is the score a bespoke key detector has to beat before that bespoke detector is worth shipping. libkeyfinder is also the key detector the library uses to tag tracks, and it stays that detector until a bespoke one beats it on the scoreboard.

The crates that make up the app itself forbid unsafe code, and calling into a C++ library needs it, so all of the risk of these calls sits in `src/lib.rs`, which is short enough to read in one sitting. Four crates allow unsafe code: this one, `crates/signalsmith-sys` for the time-stretcher, `crates/aubio-sys` for the beat tracker, and `crates/macos-documents-sys` for the documents macOS asks the app to open.

Both functions `src/lib.rs` offers check the sample rate before they reach the C++. `key_of_audio` fails, and `frame_samples` answers zero, for a rate outside 8,000 to 384,000 samples per second, because libkeyfinder works out how far to downsample a track from the rate and divides by the result.

## What is vendored

`vendor/libkeyfinder` holds the source of libkeyfinder 2.2.7, from [github.com/mixxxdj/libkeyfinder](https://github.com/mixxxdj/libkeyfinder), compiled from source on every build, so there is no prebuilt library to trust and nothing to download.

Every file libkeyfinder ships is here except one, `fftadapter.cpp`, which is the fast Fourier transform libkeyfinder normally runs on. That transform is built on FFTW, a library that would have to be installed on every machine Dermixen builds on, and Dermixen depends on no system library, so `src/fftadapter.cpp` in this crate takes its place. The libkeyfinder authors left room for the swap, and wrote a comment in their own implementation file explaining that the FFTW header is included there so a separate implementation can be substituted. `vendor/libkeyfinder/src/fftadapter.h`, the header that sets out what the transform must do, is vendored unchanged along with everything else.

libkeyfinder is under the GNU General Public License version 3, and `vendor/libkeyfinder/LICENSE` is libkeyfinder's own copy of that license. Dermixen is under the same license, which is what makes linking libkeyfinder into Dermixen allowed.

`vendor/ooura` holds the general purpose fast Fourier transform package Takuya Ooura published, from [the author's page at Kyoto University](https://www.kurims.kyoto-u.ac.jp/~ooura/fft.html). Only `fft4g.c`, the radix-four version, is used; `readme.txt` is the package's own description and states its terms, which allow anyone to use, copy, modify, and distribute the code for any purpose and without fee. Both files are exactly as published.

## How the build works

`build.rs` compiles the vendored files with the `cc` crate into two static libraries, one for the C transform and one for the C++ around it.

The transform is compiled with each of its functions renamed. Ooura gave them names as ordinary as `rdft` and `cft1st`, which would sit in the finished program where anything else linked into Dermixen could collide with them, so the build hands each one a name of its own and the vendored file stays exactly as Ooura published it.

The C++ is compiled with exceptions turned on, because libkeyfinder reports every failure by throwing one. None of them reaches Rust: `src/shim.cpp` catches every exception at the boundary and answers with a status code and a message instead, so nothing ever unwinds out of C++ and into Rust.

libkeyfinder guards its own caches with C++ mutexes, and on Linux those mutexes are built on the POSIX threads library, so on everything except macOS and Windows the build also asks the linker for that library.

Building the same way on every machine makes the analysis come out the same on every machine, which matters because the scoreboard compares numbers taken on different machines.

## What the substitute transform does

`src/fftadapter.cpp` is the only part of libkeyfinder Dermixen writes for itself. It answers the same questions FFTW answered, in the same units and the same array positions, so the numbers libkeyfinder works from are the numbers it would have had from FFTW. Two differences between the two libraries are settled inside that file: the Ooura transform states its result as a cosine part and a sine part, where the sine part is the negative of the imaginary part every other library reports, and it halves the result of the inverse transform where FFTW did not.

The Ooura transform handles a frame whose length is a power of two, which are the only lengths libkeyfinder uses: sixteen thousand three hundred and eighty-four samples for the spectrum of a window, and two thousand and forty-eight for the shape of the low-pass filter. A frame of any other length is turned away with the same kind of error libkeyfinder raises for its own bad arguments.

Every key libkeyfinder names is read out of this transform, so a mistake in it would move every key quietly rather than fail anything. The tests in `src/lib.rs` therefore check it against a transform worked out from the textbook definition, bin by bin, at both of the sizes libkeyfinder uses and at four smaller ones, and check that a frame comes back unchanged from a trip out to the frequency bins and back. Every value has to agree to one part in a billion.

## What the Rust side offers

`src/lib.rs` exposes two functions. `key_of_audio` takes one channel of a whole track and the sample rate, and hands back the key libkeyfinder named and how far ahead of the runner-up that key finished. `frame_samples` reports how much audio fills one of libkeyfinder's analysis frames, which is the length a caller has to reach before an answer is worth having. Nothing else from libkeyfinder is exposed, because nothing else is called.

libkeyfinder numbers its own keys from A upwards and names the five black notes with flats. `key_of_audio` settles both differences, so a caller only ever sees a count of semitones above C and a mode.

The confidence is the margin between the best-scoring key and the second best, divided by the best key's own score. Zero means the top two keys tied; one means the runner-up scored nothing at all. Read the figure as a ranking rather than as a probability. Every key's expected spread of pitches overlaps heavily with every other's, so a plain four-chord progression that sits squarely in one key wins by only a few percent, while noise with no key in it wins by a few tenths of one percent. Higher always means a clearer key.

Three details of libkeyfinder shape how this crate calls into it.

libkeyfinder builds its two tone profiles during the first analysis that needs them and does not guard that first build against a second thread arriving at the same moment. `key_of_audio` therefore builds both profiles once, before any analysis runs, after which every analysis only reads them, so two tracks can be analyzed on two threads at the same time.

libkeyfinder counts samples in a number that reaches a little over four thousand million, which is about twenty-seven hours of audio. The crate refuses a longer track, and the error it answers with names that limit.

libkeyfinder also turns nothing away for being short. Handed a fragment, it pads the fragment with silence until one analysis frame is full and names a key from the result, which is a key read mostly from silence. `frame_samples` reports how many samples fill one such frame, a little under four seconds at the rate Dermixen works at, so a caller can refuse a fragment rather than believe the answer. Dermixen's own key analyzer does exactly that.

One more detail is worth knowing before a whole library is scanned. libkeyfinder holds the track it is working on as double-precision samples, which take twice the room the samples arrive in, so an eight-minute track occupies a little under two hundred megabytes while its key is being found and nothing after that. That figure covers only what libkeyfinder holds: the caller's own stereo audio and the mono copy made to hand across are both alive at the same time, so a scan should expect roughly twice the figure in total. Analyzing several tracks at once multiplies the whole of it by the number of threads.
