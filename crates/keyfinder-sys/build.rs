//! Compiles the vendored libkeyfinder sources, the vendored Ooura fast
//! Fourier transform they run on, and the shim around them into a static
//! library for this crate to link against.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The libkeyfinder source files. This is every file the vendor directory
/// holds, because the one file left out of that directory, the fast Fourier
/// transform that calls FFTW, is replaced by `src/fftadapter.cpp` in this
/// crate.
const LIBKEYFINDER_SOURCES: &[&str] = &[
    "audiodata.cpp",
    "chromagram.cpp",
    "chromatransform.cpp",
    "chromatransformfactory.cpp",
    "constants.cpp",
    "keyclassifier.cpp",
    "keyfinder.cpp",
    "lowpassfilter.cpp",
    "lowpassfilterfactory.cpp",
    "spectrumanalyser.cpp",
    "temporalwindowfactory.cpp",
    "toneprofiles.cpp",
    "windowfunctions.cpp",
    "workspace.cpp",
];

/// Every function the vendored Ooura fast Fourier transform defines.
///
/// Ooura gave them names as ordinary as `rdft` and `cft1st`, which would sit
/// in the finished program where anything else linked into Dermixen could
/// collide with them. The build renames each one, so the vendored file stays
/// exactly as Ooura published it and the names it contributes are its own.
const OOURA_FUNCTIONS: &[&str] = &[
    "cdft",
    "rdft",
    "ddct",
    "ddst",
    "dfct",
    "dfst",
    "makewt",
    "makect",
    "bitrv2",
    "bitrv2conj",
    "cftfsub",
    "cftbsub",
    "cft1st",
    "cftmdl",
    "rftfsub",
    "rftbsub",
    "dctsub",
    "dstsub",
];

fn main() {
    println!("cargo:rerun-if-changed=src/shim.cpp");
    println!("cargo:rerun-if-changed=src/fftadapter.cpp");
    println!("cargo:rerun-if-changed=vendor");

    let libkeyfinder = Path::new("vendor/libkeyfinder/src");

    // The Ooura transform is C, and the rest is C++, so the two are compiled
    // separately and linked together.
    let mut fft = cc::Build::new();
    fft.std("c99")
        .file("vendor/ooura/fft4g.c")
        // Key detection runs a fast Fourier transform over every window of a
        // whole track. Building it unoptimized would make analyzing a library
        // slow enough to be annoying even in a debug build, so it is optimized
        // whatever the surrounding Rust build is doing.
        .opt_level(2)
        // The vendored sources are someone else's code, so warnings about them
        // are not this project's to act on.
        .warnings(false);
    for name in OOURA_FUNCTIONS {
        fft.define(name, format!("dermixen_keyfinder_ooura_{name}").as_str());
    }
    fft.compile("dermixen_keyfinder_fft");

    // libkeyfinder reports every failure by throwing, so this build leaves
    // exceptions turned on. None of them reaches Rust: `src/shim.cpp` catches
    // every one at the boundary and answers with a status code instead.
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++11")
        .include(libkeyfinder)
        .files(
            LIBKEYFINDER_SOURCES
                .iter()
                .map(|name| libkeyfinder.join(name)),
        )
        .file("src/fftadapter.cpp")
        .file("src/shim.cpp")
        .opt_level(2)
        .warnings(false);
    repair_a_broken_apple_toolchain(&mut build);
    build.compile("dermixen_keyfinder");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "macos" && target_os != "windows" {
        // libkeyfinder guards its own caches with C++ mutexes, which on Linux
        // are built on the POSIX threads library.
        println!("cargo:rustc-link-lib=pthread");
    }
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
/// every platform other than macOS, this changes nothing. The build script of
/// the wrapper around the time-stretcher holds the same repair, because that
/// crate builds C++ too.
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
