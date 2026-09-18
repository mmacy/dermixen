#![forbid(unsafe_code)]

//! Test support shared by the workspace.
//!
//! Tests that need whole tracks from the reference library find it through
//! [`library_root`], tests that need audio with known properties
//! generate it with the functions in [`synth`], and tests that judge audio by
//! what is in it measure it with [`spectrum`]. See `docs/fixtures.md` for the
//! policy.

use std::path::PathBuf;

pub mod mp3;
pub mod spectrum;
pub mod synth;

/// The environment variable that points at a local copy of the reference library.
pub const LIBRARY_ENV: &str = "DERMIXEN_LIBRARY";

/// The root of the reference library, or `None` when [`LIBRARY_ENV`] is unset
/// or does not point at a directory. A test that needs it returns early when
/// this is `None`, which counts as passing.
pub fn library_root() -> Option<PathBuf> {
    let root = PathBuf::from(std::env::var_os(LIBRARY_ENV)?);
    root.is_dir().then_some(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_library_test_runs_only_when_the_library_is_present() {
        let Some(root) = library_root() else {
            eprintln!("skipping: {LIBRARY_ENV} is not set");
            return;
        };
        assert!(
            root.join("mixes").is_dir(),
            "{} has no mixes folder",
            root.display()
        );
    }
}
