# Security

Report a security problem in Dermixen privately through [GitHub's private vulnerability reporting](https://github.com/mmacy/dermixen/security/advisories/new) rather than in a public issue. Say which command, window action, or file format triggers the problem and how to reproduce it.

## What the app reads

The `dermixen` command and the `dermixen-app` window read audio files, mix documents and their autosaves, the settings file, the library file, a playlist for `mix plan`, and a folder to scan. Every file among them has stated limits, and a scan applies the audio file limits to each file it finds. The "Limits" section of `DESIGN.md` gives every number: the size and sample rate a decoder accepts, the length of a mix or a track, the size of a mix document or a playlist, and the rest.

A file outside a stated limit is refused with a message naming the value. That refusal is the intended behavior. A crash, a hang, a lost or replaced file, or memory use that grows without bound is worth reporting.

## Network access

The `dermixen` command and the `dermixen-app` window open no network connection, and no crate in the workspace depends on a networking or TLS library. `cargo tree --workspace --all-features -e all | grep -iE 'reqwest|hyper|ureq|rustls|native-tls|openssl|curl|tokio'` prints nothing.

Two programs under `tools/`, `discogs_release.py` and `discogs_year.py`, make requests to Discogs to fill in release and year metadata. Neither ships with the app.
