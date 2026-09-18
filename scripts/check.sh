#!/bin/sh
# Runs the checks continuous integration runs, in the same order, on this
# machine: formatting, lints, and every test in the workspace. Exits at the
# first failure. Run it from the repository root before merging a branch.
set -e
cd "$(dirname "$0")/.."
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
