# Build from source

The app is a Cargo workspace, and one command builds both executables. Three vendored libraries are compiled from C and C++ during the build, and the LAME MP3 encoder is built with its own configure script and `make`, so a compiler and `make` are needed beside Rust.

## Prerequisites

On macOS:

- `rust-toolchain.toml` names the toolchain the workspace builds with, and `rustup` installs it on the first cargo command. The window needs at least Rust 1.95, because it is built on `eframe` 0.36.
- The Xcode command line tools, which provide the C and C++ compiler and `make`. `xcode-select --install` installs them.

On Linux:

- `rust-toolchain.toml` names the toolchain the workspace builds with, and `rustup` installs it on the first cargo command. The window needs at least Rust 1.95, because it is built on `eframe` 0.36.
- A C and C++ compiler and `make`, like the `build-essential` package on Debian and Ubuntu.
- The ALSA development headers, which the build needs for audio output: `libasound2-dev` on Debian and Ubuntu.

## Build everything

```sh
cargo build --release --workspace
```

The executables are `target/release/dermixen` and `target/release/dermixen-app`. `dermixen open` looks for `dermixen-app` in the folder that contains `dermixen`, so keep the two together, or set `DERMIXEN_APP` to the window's path. The window is slow in a debug build, so run it with `--release`.

## Install the executables

```sh
cargo install --path crates/cli
cargo install --path crates/app
```

Both land in `~/.cargo/bin`, which is on your `PATH` when Rust was installed with `rustup` (the usual way).

## Build without the foreign libraries

```sh
cargo build --release -p dermixen-cli --no-default-features
```

The stretcher, the aubio beat tracker, the libkeyfinder key detector, the audio output, and the MP3 encoder sit behind the Cargo features `signalsmith`, `aubio`, `keyfinder`, `playback`, and `mp3`, all on by default. Without them, `dermixen` still finds grids and anchors with the built-in analyzers, still renders to WAV, and still writes a `play --capture` file, but it stores no key, resamples instead of preserving pitch, refuses an MP3 output, and can't play through a device. Turn on the ones you want by name:

```sh
cargo build --release -p dermixen-cli --no-default-features --features signalsmith,mp3
```

## Run the checks

```sh
scripts/check.sh
```

The script runs the formatting check, the lints, and every test in the workspace, in that order, and stops at the first failure. Continuous integration runs the same three checks and a `cargo deny` license check on Linux and macOS for every push to `main` and every pull request. The tests that need whole tracks read the `DERMIXEN_LIBRARY` environment variable and pass without running when it's unset, so the suite runs on any machine. [Test fixtures](../fixtures.md) describes the three tiers of audio the tests use.

## Build the documentation site

The pages in `docs/` are also a website, built with MkDocs and the Material theme. `pyproject.toml` at the root of the repository lists the toolchain, and [uv](https://docs.astral.sh/uv/) installs it.

```sh
uv sync
uv run mkdocs serve
```

`mkdocs serve` builds the site, serves it at `http://127.0.0.1:8000/`, and rebuilds it when a file under `docs/` or `mkdocs.yml` changes. `uv run mkdocs build --strict` writes the site to `site/`, which isn't committed, and fails when a page links to a file or a heading that doesn't exist or when a page is missing from the navigation in `mkdocs.yml`.
