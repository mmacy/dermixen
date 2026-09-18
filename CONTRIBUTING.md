# Contributing

Dermixen is built to `DESIGN.md`, which records every product decision and the non-goals. Read it before proposing a feature, because a change that adds an effect beyond the 3-band EQ, a MIDI surface, a plugin format, a master volume, or variable-tempo source material is out of scope by decision rather than by omission.

## How work is verified

Correctness is shown by things that run as well as by review: acceptance tests, golden render fixtures, the analyzer scoreboard, and the `dermixen` command. A reviewer reads the diff and the code around it, and a pull request also has to show its correctness through one of those: a test, a render, or a scoreboard number.

- Acceptance tests land before the code that makes them pass. A test for an unfinished feature is marked ignored, so `cargo test` stays green and an ignored test is a feature that does not exist yet.
- Tests are not edited to make them pass. A test that is wrong is a separate change with its own explanation.
- A change to the engine's output changes a golden render, and the pull request says why the new render is right.
- A change to an analyzer is measured on the scoreboard, `dermixen scoreboard`, and the pull request gives the numbers before and after.

## Before you open a pull request

`scripts/check.sh` runs what continuous integration runs: the formatting check, the lints with warnings as errors, and every test in the workspace. It must pass on your machine. Continuous integration runs the same three commands on Linux and macOS, and `cargo deny check licenses sources` against `deny.toml`, for every pull request.

```
scripts/check.sh
cargo deny check licenses sources
```

A new dependency has to carry a license on the allow list in `deny.toml`. The app is licensed GPL-3.0-or-later, so a dependency under a license the GPL cannot combine with fails the check.

Only the four foreign-function crates (`aubio-sys`, `keyfinder-sys`, `signalsmith-sys`, and `macos-documents-sys`) may contain unsafe code. Every other crate forbids it.

## Writing

Everything written for a person follows the rules under "Writing for humans" in `CLAUDE.md`: docs, doc comments, comments, commit messages, and pull request text. A document states what is true now, in plain declarative sentences, and never how the code came to be. `python3 tools/check_prose.py docs/*.md` scores Markdown files against those habits.

The documentation under `docs/` is a MkDocs site. `uv sync` installs the toolchain and `uv run mkdocs build --strict` fails on a broken link or a page missing from the navigation, so run it after editing a page.

## Licensing your contribution

By opening a pull request you license your contribution under the GNU General Public License, version 3 or any later version, the same terms as the rest of Dermixen.
