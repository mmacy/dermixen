//! Compiles the vendored aubio C sources that the beat tracker needs into a
//! static library for this crate to link against.

use std::path::Path;

/// The aubio source files the tempo and beat tracker reaches. Every file the
/// vendor directory holds is in this list, because the parts of aubio that beat
/// tracking never reaches, such as pitch detection and the audio file readers,
/// were left out of the vendor directory in the first place.
const SOURCES: &[&str] = &[
    "cvec.c",
    "fvec.c",
    "lvec.c",
    "mathutils.c",
    "musicutils.c",
    "onset/peakpicker.c",
    "spectral/fft.c",
    "spectral/ooura_fft8g.c",
    "spectral/phasevoc.c",
    "spectral/specdesc.c",
    "spectral/statistics.c",
    "tempo/beattracking.c",
    "tempo/tempo.c",
    "temporal/biquad.c",
    "temporal/filter.c",
    "utils/hist.c",
    "utils/log.c",
    "utils/scale.c",
];

/// The standard headers aubio uses when it is told they exist. Its own
/// configuration step probes for each one and defines the matching name; every
/// header in this list is part of C99, so the probe is not worth repeating.
const HEADER_DEFINES: &[&str] = &[
    "HAVE_STDLIB_H",
    "HAVE_STDIO_H",
    "HAVE_MATH_H",
    "HAVE_STRING_H",
    "HAVE_ERRNO_H",
    "HAVE_LIMITS_H",
    "HAVE_STDARG_H",
];

fn main() {
    println!("cargo:rerun-if-changed=vendor/aubio");

    let source_dir = Path::new("vendor/aubio/src");

    let mut build = cc::Build::new();
    build
        .std("c99")
        .include(source_dir)
        .files(SOURCES.iter().map(|name| source_dir.join(name)))
        // Beat tracking runs a fast Fourier transform on every half window of
        // a whole track. Building it unoptimized would make analyzing an
        // hour-long set slow enough to be annoying even in a debug build, so
        // it is optimized whatever the surrounding Rust build is doing.
        .opt_level(2)
        // The vendored sources are someone else's code, so warnings about them
        // are not this project's to act on.
        .warnings(false);

    for name in HEADER_DEFINES {
        build.define(name, None);
    }
    // aubio's error and warning macros take a variable number of arguments,
    // which every C99 compiler supports.
    build.define("HAVE_C99_VARARGS_MACROS", None);
    // Lets aubio copy whole vectors with memcpy instead of element by element.
    build.define("HAVE_MEMCPY_HACKS", None);
    // Leaves the assertions in aubio's own code out of release builds, and in
    // debug builds leaves them in.
    if std::env::var("PROFILE").as_deref() == Ok("release") {
        build.define("NDEBUG", None);
    }

    build.compile("dermixen_aubio");

    // The vendored C calls the floating-point maths functions, `powf` and
    // `sqrtf` among them, which older releases of the GNU C library and every
    // release of musl keep in a maths library that a program has to ask for by
    // name; macOS and Windows have no separate maths library to ask for.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "macos" && target_os != "windows" {
        println!("cargo:rustc-link-lib=m");
    }
}
