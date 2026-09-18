# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project overview

Dermixen is a studio mix-authoring desktop app for macOS and Linux, inspired by MixMeister: drag tracks onto a timeline, get automatic beatmatched transitions, edit them, render a single continuous mix. It is not a live performance tool.

**Read `DESIGN.md` before doing substantive work. It is the authoritative spec.** It records every product and technology decision, including the explicit non-goals (no effects beyond a 3-band EQ, no MIDI, no plugins, no master volume, no variable-tempo source material) and the analyzer strategy.

`PLAN.md` describes the working method: how work is verified, the roles, the work packet a role receives, and the harness settings. The roles are subagent definitions in `.claude/agents/`, and `.claude/settings.json` holds the harness limits they rely on. Read `PLAN.md` before assigning or starting any implementation work.

## Building and testing

The app is a Cargo workspace. `crates/core` holds the mix document model, `crates/media` decoding and encoding, `crates/analysis` the analyzers and the scoreboard, `crates/library` the scanner, the metadata reader, and the SQLite index, `crates/engine` the render graph, `crates/cli` the `dermixen` command, `crates/app` the desktop shell, and `crates/testkit` test support that is not part of the app. Four foreign-function wrapper crates, `crates/signalsmith-sys` for the pitch-preserving stretcher, `crates/aubio-sys` for the beat tracker baseline, `crates/keyfinder-sys` for the key detection baseline, and `crates/macos-documents-sys` for the documents macOS asks the app to open, are the only crates allowed unsafe code. Every other crate forbids it. The MP3 encoder is LAME, reached through the `mp3lame-encoder` crate behind the media crate's `mp3` feature, so it needs no wrapper crate in the workspace.

```
cargo build --workspace
cargo test --workspace --no-fail-fast
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo run -p dermixen-cli -- --help
cargo run --release -p dermixen-app -- set.dmx
```

`rust-toolchain.toml` names the toolchain the workspace builds with, and `rustup` installs it on the first cargo command. The window needs at least Rust 1.95, because it is built on `eframe` 0.36. The window is slow in a debug build, so run it with `--release`.

Continuous integration runs the formatting check, the lints, and the tests on Linux and macOS for every push to `main` and every pull request. A `cargo deny check` confirms that every dependency's license is allowed. It also confirms that no dependency carries an unaddressed security advisory or an unpinned wildcard version, and that every dependency comes from a known source. `scripts/check.sh` runs the formatting check, the lints, and the tests locally. On Linux the audio output needs the ALSA development headers, which is the `libasound2-dev` package on Debian and Ubuntu. The workflow installs it.

The documentation in `docs/` is also a MkDocs site. `uv sync` installs the toolchain that `pyproject.toml` lists, `uv run mkdocs serve` serves the site locally, and `uv run mkdocs build --strict` builds the site and fails on a broken link or a page missing from the navigation in `mkdocs.yml`. The `Docs` workflow in `.github/workflows/docs.yml` builds the site the same way and publishes it to GitHub Pages at https://mmacy.github.io/dermixen/ on every push to `main` that changes the docs. The docs build isn't part of `scripts/check.sh`.

Acceptance tests are committed before the code that makes them pass and are marked ignored until their chunk lands, so `cargo test` stays green and an ignored test is a chunk that has not been implemented yet. `cargo test --workspace -- --include-ignored` runs everything and shows those tests failing. The unit types in `crates/core/src/units.rs` are the vocabulary every other crate uses: `Seconds`, `Samples`, `Beats`, `Bpm`, and `Decibels`, with the sample rate and the beats per bar as named constants. The project file format is described in `docs/project-file.md`, and the fixture tiers in `docs/fixtures.md`. `docs/README.md` lists every documentation page by kind: tutorials, how-to guides, reference, and explanation.

`tools/` holds Python programs that support the work without being part of the app. It holds `mmp_dump.py`, which reads MixMeister `.mmp` playlists, and `make_audio_fixtures.py`, which generates the synthetic audio fixtures. See `tools/README.md`.

## How work is verified

Correctness is shown by things that run as well as by reading the code: tests, golden render fixtures, the analyzer scoreboard, and the CLI harness are the evidence that work is done, and code review checks the code behind that evidence. Explain technical decisions in plain language, and when a request contains a technically confused premise, call it out and correct it rather than silently working around it.

## Writing for humans

Everything written for a person follows the rules here: docs, docstrings, comments, commit messages, pull request text, the report at the end of a task, and every reply. The reader didn't watch the work happen. Shorthand is fine while you work. It never appears in anything that persists or that a person reads.

The register is developer documentation: plain declarative sentences, one idea each, in the present tense and the active voice, with contractions where speech would use them. Say the fact and let the reader judge whether it's easy or good. A document ends when the information ends.

### Rules

- Write complete sentences. Introduce a term at its first use, with its abbreviation in parentheses if it has one.
- Name the real actor. Software may be the subject of an action it performs: the window draws the lane, the command reports the error. An inert thing can't want, need, decide, say, ask, refuse, or know. Write "the builder shows no trigger field for a trap", never "a trap shows no trigger field".
- Use the verb you mean. A file contains text, a list contains entries, a record has a field. Reserve *carry* for a person or a machine moving an object, and *hold* for logic (the rule holds) and literal holding.
- Resolve every pronoun and quantifier. *It*, *them*, *both*, *either*, and *those* each need one obvious referent that has already appeared. Repeat the noun instead.
- No em dashes and no en dashes. Use a full stop, a comma, a colon, or parentheses. A range is written `3-5 minutes`. A definition list item is `**Term** - definition` or `**Term**: definition`.
- No semicolons. Two clauses are two sentences.
- No arrow chains (`A → B → fails`), no hyphen-stacked compounds, and no labels or codenames invented while working. If working vocabulary has to appear, introduce it as if the reader has never seen it.
- No invented nouns for a situation. A defect isn't a *shape*, a *surface*, or a *story*. Say what happens: which input, which state, which result.
- No evaluative adjectives and no minimizers: seamless, robust, powerful, elegant, simply, easily, just. Keep the fact.
- No stage directions. Don't open a sentence with "Note that", "Importantly", "Under the hood", or "In other words", and don't close a paragraph with a sentence that restates it.
- Never open by describing the document. Not "This document describes", not "This section covers". Start with the subject.
- Documents state current truth only. No "originally X, then Y", no dates of decisions, and no session narrative: not "a survey found", not "inspection showed", not "this predates the change". Git history and the agents' memory record how things came to be. A reader needs what's true now and why it matters going forward.
- When mentioning files, commits, flags, or identifiers, give each its own plain-language clause.
- Headings and user-facing strings are sentence case.
- Open a summary or a report with the outcome in one sentence, then the detail.
- If you have to choose between short and clear, choose clear.

### Edits that change what a document claims

Some prose fixes are technical claims: naming the actor behind a passive, resolving a pronoun, expanding a compressed list, making an implicit limit or default explicit, or splitting a spliced sentence that stated a condition. Before an edit of that kind, confirm the resolution against the code or its tests and cite the file and line, or leave the sentence as it is and propose the wording along with what you couldn't confirm. Guessing isn't an option, and neither is editing first and flagging afterwards. If the code shows the sentence is false, fix the claim and say that you did.

### Checking prose

`python3 tools/check_prose.py docs/*.md` scores Markdown files against a set of writing fingerprints and prints which are out of range: em dashes, semicolons, sentence length, contractions, marketing words, title-case headings, and openers that describe the document. It's a diagnostic. Fixing the prose fixes the numbers. It doesn't find false agency, an unresolved pronoun, or an invented label. A reader finds those, and the reviewer role reads every chunk's comments and docstrings for them.
