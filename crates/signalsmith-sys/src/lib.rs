//! Signalsmith Stretch, the pitch-preserving time-stretcher, made callable
//! from Rust.
//!
//! Signalsmith Stretch is a C++ library. Its source is vendored under
//! `vendor/signalsmith-stretch` in this crate and compiled by `build.rs`
//! together with `src/shim.cpp`, which is a short piece of C++ that turns the
//! library's class template into plain C functions. This file wraps those C
//! functions in [`Stretch`], which is safe to use and frees the C++ object when
//! it is dropped.
//!
//! The crates that make up the app itself forbid unsafe code, so the risk of
//! calling into this C++ sits here, in about a hundred lines that can be read
//! in one sitting. Four crates allow unsafe code: `signalsmith-sys` around the
//! time-stretcher, `aubio-sys` around the beat tracker, `keyfinder-sys` around
//! the key detector, and `macos-documents-sys` around the documents macOS asks
//! the app to open.
//!
//! Every safe function here checks what it is given before it reaches the C++,
//! so no value a caller can write reaches the library outside the range the
//! library handles.
//!
//! The vendored library and the digital signal processing headers it includes
//! are both under the MIT license, and the two `LICENSE.txt` files that state
//! that are vendored alongside them.

use std::ffi::{c_int, c_long};
use std::fmt;

/// The C++ stretcher object. Rust never looks inside one: it holds a pointer
/// to one and hands that pointer back to the C++ side on every call.
#[repr(C)]
struct Opaque {
    _private: [u8; 0],
}

/// The status the C++ side answers with when the call finished.
const STATUS_OK: c_int = 0;

unsafe extern "C" {
    fn dermixen_signalsmith_new(channels: c_int, sample_rate: f32, seed: c_long) -> *mut Opaque;
    fn dermixen_signalsmith_free(stretcher: *mut Opaque);
    fn dermixen_signalsmith_reset(stretcher: *mut Opaque) -> c_int;
    fn dermixen_signalsmith_input_latency(stretcher: *const Opaque) -> c_int;
    fn dermixen_signalsmith_output_latency(stretcher: *const Opaque) -> c_int;
    fn dermixen_signalsmith_process(
        stretcher: *mut Opaque,
        input: *const f32,
        input_frames: c_int,
        output: *mut f32,
        output_frames: c_int,
    ) -> c_int;
}

/// One stretcher, which changes the speed of audio without moving its pitch.
///
/// A stretcher holds the audio recently fed to it, so one stretcher belongs to
/// one piece of audio for as long as that audio is being stretched. Dropping
/// the stretcher frees the C++ object behind it.
pub struct Stretch {
    stretcher: *mut Opaque,
    channels: usize,
}

/// The lowest sample rate [`Stretch::new`] accepts, in samples per second.
///
/// This is the lowest rate audio is recorded at. The library works out the
/// window it analyzes, and the interval between one window and the next, from
/// the sample rate, and holds each as a whole number of samples. The interval
/// is three hundredths of the rate, so it is zero for every rate below one
/// hundred divided by three, which is 33.34 samples per second to the nearest
/// hundredth. The library divides by that interval and steps a loop by it.
pub const LOWEST_SAMPLE_RATE: f32 = 8_000.0;

/// The highest sample rate [`Stretch::new`] accepts, in samples per second.
///
/// This is the highest rate audio is recorded at. The library turns twelve
/// hundredths of the sample rate into a window length held in a C `int`, and
/// that conversion is undefined for a rate above about eighteen thousand
/// million samples per second.
pub const HIGHEST_SAMPLE_RATE: f32 = 384_000.0;

/// The most channels [`Stretch::new`] accepts.
///
/// Dermixen stretches stereo audio, so it asks for two. Eight leaves room for
/// surround material, and a caller that asks for more has made a mistake,
/// which this limit catches before the library allocates a buffer for every
/// channel.
pub const MOST_CHANNELS: usize = 8;

impl Stretch {
    /// Builds a stretcher for `channels` channels of audio at `sample_rate`
    /// samples per second.
    ///
    /// `seed` fixes the random numbers the library draws on when it is asked
    /// for a stretch large enough that it spreads the bins apart at random, so
    /// that rendering the same mix twice gives the same audio both times.
    ///
    /// Returns `None` for a channel count of zero or above [`MOST_CHANNELS`],
    /// and for a sample rate that is not a number from [`LOWEST_SAMPLE_RATE`]
    /// to [`HIGHEST_SAMPLE_RATE`]. The library sizes its buffers from the
    /// rate without checking it, so a rate outside that range must never
    /// reach the library. A sample rate that is not a number, and a sample
    /// rate of infinity, fail that comparison as well.
    ///
    /// `None` also comes back for a seed too large for the C type the seed is
    /// passed as. That cannot happen on macOS or on Linux, the two systems
    /// Dermixen runs on, because a C `long` holds the whole of [`i64`] on
    /// both. The conversion is here so that a platform whose C `long` is
    /// narrower cannot hand the library a seed cut down to fit.
    ///
    /// # Panics
    ///
    /// Panics if the C++ side returns nothing, which it does when it cannot
    /// allocate the library object and the buffers around it.
    pub fn new(channels: usize, sample_rate: f32, seed: i64) -> Option<Self> {
        if channels == 0 || channels > MOST_CHANNELS {
            return None;
        }
        if !(LOWEST_SAMPLE_RATE..=HIGHEST_SAMPLE_RATE).contains(&sample_rate) {
            return None;
        }
        let count = c_int::try_from(channels).ok()?;
        let seed = c_long::try_from(seed).ok()?;
        // SAFETY: the channel count and the sample rate are inside the limits
        // stated above, so the library can size its buffers from them. The C++
        // side allocates the object and returns the only pointer to it. Every
        // later call passes that same pointer back, and the `Drop` below frees
        // it exactly once.
        let stretcher = unsafe { dermixen_signalsmith_new(count, sample_rate, seed) };
        assert!(
            !stretcher.is_null(),
            "the C++ side did not return a stretcher"
        );
        Some(Self {
            stretcher,
            channels,
        })
    }

    /// The number of channels this stretcher was built for.
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// How many frames of input the stretcher takes in before its output
    /// reflects them.
    pub fn input_latency(&self) -> c_int {
        // SAFETY: the pointer is the one `new` returns and this value still owns it.
        unsafe { dermixen_signalsmith_input_latency(self.stretcher) }
    }

    /// How many frames of output the stretcher produces before its output
    /// reflects the input that has already passed the input latency.
    ///
    /// The whole delay through the stretcher, from a frame going in to that
    /// same frame coming out, is the input latency plus the output latency.
    pub fn output_latency(&self) -> c_int {
        // SAFETY: the pointer is the one `new` returns and this value still owns it.
        unsafe { dermixen_signalsmith_output_latency(self.stretcher) }
    }

    /// Forgets the audio fed so far, putting the stretcher back into the state
    /// it was created in.
    ///
    /// # Panics
    ///
    /// Panics if the C++ side reports that it could not build the library
    /// object again. The one thing that makes it report so is the allocator
    /// refusing the memory for the new buffers. The C++ side catches the
    /// exception the allocator raises and answers with a status, so the
    /// exception never crosses into Rust, and this panic is Rust's own. The
    /// stretcher then holds the audio it held before the call, so a panic here
    /// never leaves it half reset.
    pub fn reset(&mut self) {
        // SAFETY: the pointer is the one `new` returns and this value still owns it.
        let status = unsafe { dermixen_signalsmith_reset(self.stretcher) };
        assert_eq!(
            status, STATUS_OK,
            "the C++ side could not build the stretcher again"
        );
    }

    /// Takes all of `input` and fills all of `output`, playing the input at
    /// `input frames / output frames` times its original speed while leaving
    /// its pitch where it was.
    ///
    /// Both slices hold interleaved samples: one sample per channel per frame,
    /// in channel order.
    ///
    /// Each slice holds at most [`c_int::MAX`] frames, because the C++ side
    /// takes the two frame counts as C `int` values. The number of samples in
    /// a slice has no such limit: the C++ side counts sample positions inside
    /// a block in a type that spans any slice Rust can hand it.
    ///
    /// # Panics
    ///
    /// Panics if either slice is not a whole number of frames, or if either
    /// frame count is above [`c_int::MAX`].
    ///
    /// Panics as well if the C++ side reports that it could not stretch the
    /// block. The one thing that makes it report so is the allocator refusing
    /// the memory for the buffers the channels are separated into. The C++
    /// side catches the exception the allocator raises and answers with a
    /// status, so the exception never crosses into Rust, and this panic is
    /// Rust's own.
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) {
        let input_frames = self.frames(input.len(), "input");
        let output_frames = self.frames(output.len(), "output");
        // SAFETY: the pointer is the one `new` returns and this value still
        // owns it. Both slices are valid for the frame counts passed with them,
        // because those counts are their own lengths divided by the channel
        // count, and the two slices cannot overlap because one is a shared
        // borrow and the other an exclusive one.
        let status = unsafe {
            dermixen_signalsmith_process(
                self.stretcher,
                input.as_ptr(),
                input_frames,
                output.as_mut_ptr(),
                output_frames,
            )
        };
        assert_eq!(status, STATUS_OK, "the C++ side could not stretch a block");
    }

    /// The number of frames in `samples` interleaved samples, as the C type the
    /// frame count is passed as. The `what` argument names the slice for the
    /// message raised if its length is not a whole number of frames.
    fn frames(&self, samples: usize, what: &str) -> c_int {
        assert_eq!(
            samples % self.channels,
            0,
            "the {what} is not a whole number of frames"
        );
        c_int::try_from(samples / self.channels).expect("the frame count fits in a C int")
    }
}

impl Drop for Stretch {
    fn drop(&mut self) {
        // SAFETY: the pointer is the one `new` returns, nothing else holds a
        // copy of it, and this runs once because a value is dropped once.
        unsafe { dermixen_signalsmith_free(self.stretcher) };
    }
}

impl fmt::Debug for Stretch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Stretch")
            .field("channels", &self.channels)
            .field("input_latency", &self.input_latency())
            .field("output_latency", &self.output_latency())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 44_100.0;
    const SEED: i64 = 20_240_101;

    /// A stereo sine tone of `frames` frames, interleaved.
    fn tone(frames: usize, frequency: f32) -> Vec<f32> {
        let mut samples = Vec::with_capacity(frames * 2);
        for frame in 0..frames {
            let value = (std::f32::consts::TAU * frequency * frame as f32 / RATE).sin() * 0.5;
            samples.push(value);
            samples.push(value);
        }
        samples
    }

    /// Feeds `input` through `stretch` in blocks, asking for `output_block`
    /// output frames for every 1024 input frames, and returns everything
    /// produced.
    fn run(stretch: &mut Stretch, input: &[f32], output_block: usize) -> Vec<f32> {
        let mut produced = Vec::new();
        for block in input.chunks(1024 * 2) {
            let mut output = vec![0.0; output_block * 2];
            stretch.process(block, &mut output);
            produced.extend_from_slice(&output);
        }
        produced
    }

    #[test]
    fn a_stretcher_reports_the_size_it_was_built_at() {
        let stretch = Stretch::new(2, RATE, SEED).unwrap();
        assert_eq!(stretch.channels(), 2);
        // The library's default preset uses a window of about a tenth of a
        // second, so both latencies are a few thousand frames.
        assert!(stretch.input_latency() > 0);
        assert!(stretch.output_latency() > 0);
        assert!(stretch.input_latency() + stretch.output_latency() < 22_050);
        assert!(format!("{stretch:?}").starts_with("Stretch {"));
    }

    #[test]
    fn stretched_audio_comes_out_and_is_not_silence() {
        let mut stretch = Stretch::new(2, RATE, SEED).unwrap();
        let input = tone(44_100, 440.0);
        let produced = run(&mut stretch, &input, 1044);
        let loudest = produced.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
        assert!(loudest > 0.1, "the loudest sample was {loudest}");
    }

    #[test]
    fn resetting_gives_the_same_output_again() {
        let mut stretch = Stretch::new(2, RATE, SEED).unwrap();
        let input = tone(22_050, 330.0);
        let first = run(&mut stretch, &input, 1044);
        stretch.reset();
        let again = run(&mut stretch, &input, 1044);
        assert_eq!(first, again);
    }

    #[test]
    fn a_sample_rate_the_library_cannot_size_its_buffers_from_is_refused() {
        for rate in [
            0.0,
            -44_100.0,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            1.0,
            7_999.0,
            384_001.0,
            1e12,
        ] {
            assert!(Stretch::new(2, rate, SEED).is_none(), "{rate} was accepted");
        }
        for rate in [LOWEST_SAMPLE_RATE, 44_100.0, HIGHEST_SAMPLE_RATE] {
            assert!(Stretch::new(2, rate, SEED).is_some(), "{rate} was refused");
        }
    }

    #[test]
    fn a_channel_count_outside_the_limit_is_refused_and_nothing_aborts() {
        for channels in [0, MOST_CHANNELS + 1, 1_000_000, usize::MAX] {
            assert!(
                Stretch::new(channels, RATE, SEED).is_none(),
                "{channels} channels were accepted"
            );
        }
        assert_eq!(
            Stretch::new(MOST_CHANNELS, RATE, SEED).unwrap().channels(),
            MOST_CHANNELS
        );
    }
}
