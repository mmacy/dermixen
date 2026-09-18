//! The time-stretcher behind which Signalsmith Stretch, plain resampling, and
//! any later replacement sit.

#[cfg(feature = "signalsmith")]
use dermixen_core::SAMPLE_RATE;
use dermixen_core::Samples;
use dermixen_media::Frame;

/// Changes the speed of audio, block by block.
///
/// The render graph feeds each stretcher input in blocks and asks for output
/// in blocks. The speed for a block is set by the two lengths: a call that
/// takes `input.len()` frames and produces `output.len()` frames plays the
/// audio at `input.len() / output.len()` times its original speed. Every
/// call takes the whole of its input, so nothing is left over for a later
/// call; an implementation that cannot use every frame at the speed asked of
/// it discards what it cannot use. Whether pitch is preserved is up to the
/// implementation, which is what the keylock toggle chooses between.
///
/// Implementations keep internal state between calls, so a stretcher belongs
/// to one track for the duration of a render. The latencies say how far the
/// output lags the input, so the graph can prime the stretcher and discard
/// the lead-in. They are kept apart because they are counted in different
/// frames: the input latency in input frames and the output latency in output
/// frames. To line the output up with the input, discard
/// `round(input_latency / speed) + output_latency` output frames, where speed
/// is input frames per output frame, as above. Dividing is what makes the
/// alignment hold at every speed rather than only at speed one.
pub trait TimeStretcher {
    /// Takes all of `input` and fills all of `output`.
    fn process(&mut self, input: &[Frame], output: &mut [Frame]);

    /// How many input frames the stretcher takes in before its output starts
    /// to reflect them, counted in input frames.
    fn input_latency(&self) -> Samples;

    /// How many output frames the stretcher produces before its output starts
    /// to reflect the input that has already passed the input latency,
    /// counted in output frames.
    fn output_latency(&self) -> Samples;

    /// Forgets all internal state, as if newly created.
    fn reset(&mut self);
}

/// A stretcher that does not stretch: it copies input to output frame for
/// frame, dropping input that does not fit and padding output with silence.
///
/// It exists so the trait has a second implementation to test against, not
/// for use in a render.
#[derive(Debug, Default, Clone, Copy)]
pub struct Passthrough;

impl TimeStretcher for Passthrough {
    fn process(&mut self, input: &[Frame], output: &mut [Frame]) {
        let copied = input.len().min(output.len());
        output[..copied].copy_from_slice(&input[..copied]);
        for frame in &mut output[copied..] {
            *frame = [0.0, 0.0];
        }
    }

    fn input_latency(&self) -> Samples {
        Samples::ZERO
    }

    fn output_latency(&self) -> Samples {
        Samples::ZERO
    }

    fn reset(&mut self) {}
}

/// A stretcher that changes speed by resampling, so pitch moves with speed
/// the way a record does. This is what a track without keylock uses.
///
/// Each output frame is read from the input at a position that moves on by
/// `input.len() / output.len()` input frames per output frame, and a position
/// that falls between two input frames is filled in by drawing a straight line
/// between them. The position runs on from one call to the next, so a speed
/// that changes between one block and the next joins without a click.
///
/// Output frame `j` of a block reads input position `j * speed - 1`, counted
/// from the start of that block, so the output lags the input by exactly one
/// input frame and by no output frames at all. That is why the resampler
/// reports an input latency of one frame and no output latency.
#[derive(Debug)]
pub struct Resampler {
    /// Where the first output frame of the next block reads from, counted in
    /// input frames from the start of that block. It is minus one to begin
    /// with and the arithmetic at the end of every block returns it to minus
    /// one, because a block's output reads exactly one block's worth of input.
    /// The frame that joins one block to the next is therefore not this
    /// position but the held frame below, which is the one at minus one.
    position: f64,
    /// The last frame of the previous block, which is the frame at position
    /// minus one. It is silence until a first block has been processed.
    previous: Frame,
}

impl Default for Resampler {
    fn default() -> Self {
        Self::new()
    }
}

impl Resampler {
    /// A resampler at the start of its input.
    pub fn new() -> Self {
        Self {
            position: -1.0,
            previous: [0.0, 0.0],
        }
    }

    /// The frame at `at`, counted in input frames from the start of `input`,
    /// with a position between two frames filled in by drawing a straight line
    /// between them.
    fn read(&self, input: &[Frame], at: f64) -> Frame {
        // The two frames a straight line is drawn between are the ones at
        // `whole` and `whole + 1`, so the position is held inside the range
        // where both of those frames exist.
        let last = (input.len() as f64 - 1.0).max(-1.0);
        let at = at.clamp(-1.0, last);
        let whole = at.floor();
        let fraction = (at - whole) as f32;
        let before = self.frame(input, whole as i64);
        let after = self.frame(input, whole as i64 + 1);
        [
            before[0] + (after[0] - before[0]) * fraction,
            before[1] + (after[1] - before[1]) * fraction,
        ]
    }

    /// The frame at the whole position `index`, where minus one is the last
    /// frame of the block before this one and anything past the end of the
    /// block is silence.
    fn frame(&self, input: &[Frame], index: i64) -> Frame {
        if index < 0 {
            self.previous
        } else {
            input.get(index as usize).copied().unwrap_or([0.0, 0.0])
        }
    }
}

impl TimeStretcher for Resampler {
    fn process(&mut self, input: &[Frame], output: &mut [Frame]) {
        if input.is_empty() {
            // There is no audio to read from, so the output is silence. The
            // held frame stays as it was, because repeating it across the
            // whole block would put out a steady level rather than nothing.
            output.fill([0.0, 0.0]);
            return;
        }
        if output.is_empty() {
            // There is nowhere to put this block, so all of it is discarded
            // and the next block starts from the front again.
            self.position = -1.0;
        } else {
            let step = input.len() as f64 / output.len() as f64;
            for (index, frame) in output.iter_mut().enumerate() {
                *frame = self.read(input, self.position + index as f64 * step);
            }
            // What is left of this block after the last frame read from it is
            // where the next block starts reading.
            self.position += output.len() as f64 * step - input.len() as f64;
        }
        self.previous = input[input.len() - 1];
    }

    fn input_latency(&self) -> Samples {
        Samples(1)
    }

    fn output_latency(&self) -> Samples {
        Samples::ZERO
    }

    fn reset(&mut self) {
        *self = Self::new();
    }
}

/// The number of channels in every buffer Dermixen passes around.
#[cfg(feature = "signalsmith")]
const CHANNELS: usize = 2;

/// The seed for the random numbers Signalsmith Stretch draws on when it is
/// asked for a stretch large enough that it spreads the bins apart at random.
/// It is a fixed number so that rendering the same mix twice gives the same
/// audio both times, which is what makes a golden render worth keeping.
#[cfg(feature = "signalsmith")]
const SIGNALSMITH_SEED: i64 = 5_705_146;

/// Signalsmith Stretch: a pitch-preserving time-stretcher, which is what a
/// track with keylock uses. Available only with the `signalsmith` feature,
/// which builds the vendored C++ library.
///
/// The library works on a window of 5292 frames, 120 milliseconds at the
/// sample rate Dermixen uses, and its output lags its input by the length of
/// that window: 2646 input frames of input latency and 2646 output frames of
/// output latency. That is far more than the resampler's single frame, which
/// is why a render has to prime a keylocked track and drop the lead-in rather
/// than reading it straight through.
#[cfg(feature = "signalsmith")]
#[derive(Debug)]
pub struct SignalsmithStretcher {
    stretch: signalsmith_sys::Stretch,
}

#[cfg(feature = "signalsmith")]
impl Default for SignalsmithStretcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "signalsmith")]
impl SignalsmithStretcher {
    /// A stretcher configured for stereo audio at the internal sample rate.
    pub fn new() -> Self {
        Self {
            stretch: signalsmith_sys::Stretch::new(CHANNELS, SAMPLE_RATE as f32, SIGNALSMITH_SEED)
                .expect("two channels at 44.1 kHz are within the stretcher's limits"),
        }
    }
}

#[cfg(feature = "signalsmith")]
impl TimeStretcher for SignalsmithStretcher {
    fn process(&mut self, input: &[Frame], output: &mut [Frame]) {
        // A frame is a pair of samples, so a slice of frames is already laid
        // out as the interleaved audio the library reads.
        self.stretch
            .process(input.as_flattened(), output.as_flattened_mut());
    }

    fn input_latency(&self) -> Samples {
        Samples(i64::from(self.stretch.input_latency()))
    }

    fn output_latency(&self) -> Samples {
        Samples(i64::from(self.stretch.output_latency()))
    }

    fn reset(&mut self) {
        self.stretch.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Any stretcher, used through the trait object the render graph will hold.
    fn run(stretcher: &mut dyn TimeStretcher, input: &[Frame], output_len: usize) -> Vec<Frame> {
        let mut output = vec![[7.0, 7.0]; output_len];
        stretcher.process(input, &mut output);
        output
    }

    #[test]
    fn passthrough_copies_and_pads() {
        let input = [[1.0, 2.0], [3.0, 4.0], [5.0, 6.0]];
        let mut stretcher = Passthrough;
        assert_eq!(
            run(&mut stretcher, &input, 5),
            vec![[1.0, 2.0], [3.0, 4.0], [5.0, 6.0], [0.0, 0.0], [0.0, 0.0]]
        );
        assert_eq!(run(&mut stretcher, &input, 2), vec![[1.0, 2.0], [3.0, 4.0]]);
        assert_eq!(stretcher.input_latency(), Samples::ZERO);
        assert_eq!(stretcher.output_latency(), Samples::ZERO);
    }

    #[test]
    fn a_block_with_no_input_gives_silence_rather_than_a_held_level() {
        let mut stretcher = Resampler::new();
        // A first block leaves the resampler holding a frame well away from
        // silence, which is what a block with no input must not repeat.
        run(&mut stretcher, &[[0.5, 0.5]; 4], 4);
        assert_eq!(run(&mut stretcher, &[], 3), vec![[0.0, 0.0]; 3]);
        // The held frame is still there for the block that follows.
        assert_eq!(
            run(&mut stretcher, &[[0.5, 0.5]; 2], 2),
            vec![[0.5, 0.5]; 2]
        );
    }

    #[test]
    fn the_trait_is_usable_as_a_boxed_object() {
        let mut boxed: Box<dyn TimeStretcher> = Box::new(Passthrough);
        boxed.reset();
        assert_eq!(run(boxed.as_mut(), &[[1.0, 1.0]], 1), vec![[1.0, 1.0]]);
    }
}
