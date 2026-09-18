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
//! in one sitting. The only other crate that allows unsafe code is the aubio
//! wrapper, which does the same job for the beat tracker.
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

unsafe extern "C" {
    fn dermixen_signalsmith_new(channels: c_int, sample_rate: f32, seed: c_long) -> *mut Opaque;
    fn dermixen_signalsmith_free(stretcher: *mut Opaque);
    fn dermixen_signalsmith_reset(stretcher: *mut Opaque);
    fn dermixen_signalsmith_input_latency(stretcher: *const Opaque) -> c_int;
    fn dermixen_signalsmith_output_latency(stretcher: *const Opaque) -> c_int;
    fn dermixen_signalsmith_process(
        stretcher: *mut Opaque,
        input: *const f32,
        input_frames: c_int,
        output: *mut f32,
        output_frames: c_int,
    );
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
pub const LOWEST_SAMPLE_RATE: f32 = 8_000.0;

/// The highest sample rate [`Stretch::new`] accepts, in samples per second.
pub const HIGHEST_SAMPLE_RATE: f32 = 384_000.0;

/// The most channels [`Stretch::new`] accepts.
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
    /// reach the library.
    ///
    /// # Panics
    ///
    /// Panics if `channels` is zero, if `channels` or `seed` is too large for
    /// the C types they are passed as, or if the C++ side returns nothing.
    pub fn new(channels: usize, sample_rate: f32, seed: i64) -> Option<Self> {
        assert!(channels > 0, "a stretcher is built for one channel or more");
        let count = c_int::try_from(channels).expect("the channel count fits in a C int");
        let seed = c_long::try_from(seed).expect("the seed fits in a C long");
        // SAFETY: the C++ side allocates the object and returns the only
        // pointer to it. Every later call passes that same pointer back, and
        // the `Drop` below frees it exactly once.
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
    pub fn reset(&mut self) {
        // SAFETY: the pointer is the one `new` returns and this value still owns it.
        unsafe { dermixen_signalsmith_reset(self.stretcher) };
    }

    /// Takes all of `input` and fills all of `output`, playing the input at
    /// `input frames / output frames` times its original speed while leaving
    /// its pitch where it was.
    ///
    /// Both slices hold interleaved samples: one sample per channel per frame,
    /// in channel order.
    ///
    /// # Panics
    ///
    /// Panics if either slice is not a whole number of frames, or if either
    /// frame count is too large for the C type it is passed as.
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) {
        let input_frames = self.frames(input.len(), "input");
        let output_frames = self.frames(output.len(), "output");
        // SAFETY: the pointer is the one `new` returns and this value still
        // owns it. Both slices are valid for the frame counts passed with them,
        // because those counts are their own lengths divided by the channel
        // count, and the two slices cannot overlap because one is a shared
        // borrow and the other an exclusive one.
        unsafe {
            dermixen_signalsmith_process(
                self.stretcher,
                input.as_ptr(),
                input_frames,
                output.as_mut_ptr(),
                output_frames,
            );
        }
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
    #[ignore = "wrapper-limits"]
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
    #[ignore = "wrapper-limits"]
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
