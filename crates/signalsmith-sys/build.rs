//! Compiles the C++ shim, and the vendored Signalsmith Stretch headers it
//! includes, into a static library for this crate to link against.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn main() {
    println!("cargo:rerun-if-changed=src/shim.cpp");
    println!("cargo:rerun-if-changed=vendor/signalsmith-stretch");

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .include("vendor/signalsmith-stretch")
        .file("src/shim.cpp")
        // Neither the shim nor Signalsmith Stretch raises a C++ exception, and
        // an exception unwinding into Rust would be undefined behavior, so
        // exceptions are turned off outright.
        .flag("-fno-exceptions")
        // Stretching is heavy arithmetic. Building it unoptimized would make a
        // debug render slow enough to be unusable, so it is optimized whatever
        // the surrounding Rust build is doing.
        .opt_level(2)
        // The vendored headers are someone else's code, so warnings about them
        // are not this project's to act on.
        .warnings(false);

    repair_a_broken_apple_toolchain(&mut build);

    build.compile("dermixen_signalsmith");
}

/// Points the compiler at the C++ standard library inside the macOS software
/// development kit, but only on a machine where it cannot otherwise find that
/// library at all.
///
/// Apple moved the C++ standard library headers into the software development
/// kit some years ago. A machine that had the Command Line Tools installed
/// before that move can be left with an empty shell of the old header
/// directory, and the compiler prefers that empty directory over the working
/// copy in the development kit, so every C++ build on that machine fails on
/// the first standard header it reads. The proper repair is to reinstall the
/// Command Line Tools, which removes the leftover directory. Until someone
/// does that, this points the compiler straight at the working copy.
///
/// On a machine whose compiler already finds the standard library, and on
/// every platform other than macOS, this changes nothing.
fn repair_a_broken_apple_toolchain(build: &mut cc::Build) {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    if compiles_a_standard_header(build) {
        return;
    }
    let Some(headers) = development_kit_cxx_headers() else {
        return;
    };
    build.flag("-nostdinc++").flag("-isystem").flag(&headers);
    println!(
        "cargo:warning=The C++ compiler could not find the C++ standard library, so this build \
         used the copy in {}. Reinstalling the Command Line Tools removes the leftover empty \
         header directory that causes this.",
        headers.display()
    );
}

/// Whether the compiler, set up as `build` describes, can compile a file that
/// includes one header from the C++ standard library.
fn compiles_a_standard_header(build: &cc::Build) -> bool {
    let Ok(directory) = env::var("OUT_DIR") else {
        return true;
    };
    let directory = PathBuf::from(directory);
    let source = directory.join("standard-library-probe.cpp");
    if fs::write(&source, "#include <memory>\nint probe() { return 0; }\n").is_err() {
        return true;
    }
    let Ok(compiler) = build.try_get_compiler() else {
        return true;
    };
    let status = compiler
        .to_command()
        .arg("-c")
        .arg(&source)
        .arg("-o")
        .arg(directory.join("standard-library-probe.o"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    matches!(status, Ok(status) if status.success())
}

/// Where the C++ standard library headers live inside the macOS software
/// development kit, if they are there.
fn development_kit_cxx_headers() -> Option<PathBuf> {
    let output = Command::new("xcrun").arg("--show-sdk-path").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let kit = String::from_utf8(output.stdout).ok()?;
    let headers = PathBuf::from(kit.trim()).join("usr/include/c++/v1");
    headers.join("memory").exists().then_some(headers)
}
