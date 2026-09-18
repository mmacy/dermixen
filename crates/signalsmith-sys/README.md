# signalsmith-sys

Signalsmith Stretch, the pitch-preserving time-stretcher behind Dermixen's keylock, vendored as C++ and made callable from Rust.

The crates that make up the app itself forbid unsafe code, so the risk of calling into this C++ sits in `src/lib.rs`, which is short enough to read in one sitting. The other crates that allow unsafe code are `crates/aubio-sys`, which does the same job for the beat tracker, and `crates/keyfinder-sys`, which does it for the key detector.

## What is vendored

The C++ source lives under `vendor/signalsmith-stretch` and is compiled from source on every build, so there is no prebuilt library to trust and nothing to download.

- `signalsmith-stretch.h` is Signalsmith Stretch 1.1.0, from [github.com/Signalsmith-Audio/signalsmith-stretch](https://github.com/Signalsmith-Audio/signalsmith-stretch).
- `dsp/` holds the six headers that `signalsmith-stretch.h` includes, from the Signalsmith digital signal processing library 1.6.1, at [github.com/Signalsmith-Audio/dsp](https://github.com/Signalsmith-Audio/dsp). They are `common.h`, `delay.h`, `fft.h`, `perf.h`, `spectral.h`, and `windows.h`. The rest of that library is not vendored because nothing here reads it.

Both are under the MIT license. Each keeps its own `LICENSE.txt` beside the headers it covers: `vendor/signalsmith-stretch/LICENSE.txt` for the stretcher, and `vendor/signalsmith-stretch/dsp/LICENSE.txt` for the signal processing headers. Dermixen itself is under the GNU General Public License version 3, which the MIT license allows those files to sit inside.

## How the build works

`build.rs` compiles one file, `src/shim.cpp`, with the `cc` crate. Signalsmith Stretch is a C++ class template, which Rust cannot call, so the shim instantiates that template for 32-bit float samples and exposes the six operations the render graph needs as plain C functions. Audio crosses that boundary interleaved, one left sample then one right sample, because that is the layout the rest of Dermixen uses; the shim separates the channels on the way in and puts them back together on the way out.

The stretcher is built with a fixed random seed, so rendering the same mix twice gives the same audio both times.

## If the build cannot find the C++ standard library

Apple moved the C++ standard library headers into the macOS software development kit some years ago. A Mac that had the Command Line Tools installed before that move can be left with an empty shell of the old header directory, and the compiler prefers that empty directory over the working copy in the development kit. Every C++ build on such a machine fails on the first standard header it reads, whether or not it has anything to do with Dermixen.

The build script notices that case and points the compiler at the working copy, printing a warning when it does. The proper repair is to reinstall the Command Line Tools, which removes the leftover directory:

```
sudo rm -rf /Library/Developer/CommandLineTools
sudo xcode-select --install
```

After that the warning stops appearing and the build script changes nothing.
