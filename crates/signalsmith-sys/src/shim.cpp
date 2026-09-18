// A C boundary around Signalsmith Stretch, which is a C++ class template.
//
// Rust cannot call a C++ class template, so this file instantiates the
// template for 32-bit float samples and exposes the handful of operations the
// render graph needs as plain C functions. Every function here takes a pointer
// to one stretcher object, which `dermixen_signalsmith_new` creates and
// `dermixen_signalsmith_free` destroys.
//
// Audio crosses the boundary interleaved, one left sample then one right
// sample and so on, because that is the layout the rest of Dermixen uses.
// Signalsmith Stretch reads one array per channel, so this file separates the
// channels on the way in and puts them back together on the way out.
//
// The library and the buffers around it allocate, so both can throw, and a C++
// exception unwinding into a Rust frame is undefined behavior. Every function
// below catches everything and reports a failure in its own answer instead.
//
// Positions inside a block are counted in `std::size_t`, which spans any slice
// Rust can hand across. The frame counts arrive as C `int` values, so a block
// of two thousand million frames of stereo audio holds more samples than an
// `int` counts, and multiplying a frame number by the channel count in an
// `int` would overflow.

#include <cstddef>
#include <memory>
#include <utility>
#include <vector>

#include "signalsmith-stretch.h"

namespace {
/// Signalsmith Stretch instantiated for the 32-bit float samples Dermixen uses.
using Library = signalsmith::stretch::SignalsmithStretch<float>;

/// The call finished.
const int STATUS_OK = 0;
/// The call raised a C++ exception, which this file caught.
const int STATUS_FAILED = 1;
} // namespace

/// One stretcher: the library object, the buffers used to separate and
/// recombine the channels, and everything needed to build the library object
/// again from scratch when the stretcher is reset.
struct DermixenSignalsmithStretch {
    int channels;
    float sampleRate;
    long seed;
    std::unique_ptr<Library> library;
    std::vector<std::vector<float>> inputChannels;
    std::vector<std::vector<float>> outputChannels;
    std::vector<const float *> inputPointers;
    std::vector<float *> outputPointers;

    DermixenSignalsmithStretch(int channels, float sampleRate, long seed)
        : channels(channels), sampleRate(sampleRate), seed(seed),
          inputChannels(channels), outputChannels(channels),
          inputPointers(channels), outputPointers(channels) {
        build();
    }

    /// Builds the library object from scratch, at the size the default preset
    /// chooses for this sample rate. The seed is the same every time, so two
    /// renders of the same mix produce the same audio.
    ///
    /// The new object is configured before it takes the place of the old one,
    /// so a failure part of the way through leaves the stretcher holding the
    /// object it held before, which is one whose buffers all match each other.
    void build() {
        std::unique_ptr<Library> built(new Library(seed));
        built->presetDefault(channels, sampleRate);
        library = std::move(built);
    }

    void process(const float *input, int inputFrames, float *output, int outputFrames) {
        const std::size_t channelCount = (std::size_t)channels;
        // A block with no frames on one side or the other is turned away here
        // rather than passed on, because an empty vector yields a pointer that
        // may be null and the library is not asked to take one.
        if (inputFrames <= 0) {
            // There is no audio to read, so the output is silence.
            if (outputFrames > 0) {
                const std::size_t samples = (std::size_t)outputFrames * channelCount;
                for (std::size_t sample = 0; sample < samples; ++sample) {
                    output[sample] = 0.0f;
                }
            }
            return;
        }
        if (outputFrames <= 0) {
            // There is nowhere to put this block, so all of it is discarded.
            return;
        }
        for (int channel = 0; channel < channels; ++channel) {
            inputChannels[channel].resize((std::size_t)inputFrames);
            outputChannels[channel].resize((std::size_t)outputFrames);
            for (int frame = 0; frame < inputFrames; ++frame) {
                const std::size_t at = (std::size_t)frame * channelCount + (std::size_t)channel;
                inputChannels[channel][(std::size_t)frame] = input[at];
            }
            inputPointers[channel] = inputChannels[channel].data();
            outputPointers[channel] = outputChannels[channel].data();
        }
        library->process(inputPointers.data(), inputFrames, outputPointers.data(), outputFrames);
        for (int channel = 0; channel < channels; ++channel) {
            for (int frame = 0; frame < outputFrames; ++frame) {
                const std::size_t at = (std::size_t)frame * channelCount + (std::size_t)channel;
                output[at] = outputChannels[channel][(std::size_t)frame];
            }
        }
    }
};

extern "C" {

/// Creates a stretcher for `channels` channels of audio at `sampleRate` hertz,
/// with `seed` fixing the random numbers the library uses internally.
///
/// The answer is null when the library or the buffers around it could not be
/// allocated. The caller checks the two sizes before it calls this, so nothing
/// here checks them again.
DermixenSignalsmithStretch *dermixen_signalsmith_new(int channels, float sampleRate, long seed) {
    try {
        return new DermixenSignalsmithStretch(channels, sampleRate, seed);
    } catch (...) {
        return nullptr;
    }
}

/// Destroys a stretcher created by `dermixen_signalsmith_new`.
///
/// Nothing this destroys throws, and the catch is here so that every function
/// on this boundary ends the same way.
void dermixen_signalsmith_free(DermixenSignalsmithStretch *stretcher) {
    try {
        delete stretcher;
    } catch (...) {
    }
}

/// Puts a stretcher back into the state it was created in, so that the audio
/// fed to it before is forgotten. The answer is zero when the stretcher was
/// reset and one when the library object could not be built again, which
/// leaves the stretcher holding the audio it held before.
///
/// This builds the library object again rather than calling the library's own
/// reset, because that reset clears the buffers but leaves the random engine
/// where it stood. Building again puts the engine back to the same seed, which
/// is what makes two runs either side of a reset produce the same audio. The
/// cost is that the buffers are allocated again.
int dermixen_signalsmith_reset(DermixenSignalsmithStretch *stretcher) {
    try {
        stretcher->build();
        return STATUS_OK;
    } catch (...) {
        return STATUS_FAILED;
    }
}

/// How many frames of input the library takes in before its output reflects them.
///
/// This reads a size the library worked out when it was built, so it throws
/// nothing. A caught exception is answered with no latency at all.
int dermixen_signalsmith_input_latency(const DermixenSignalsmithStretch *stretcher) {
    try {
        return stretcher->library->inputLatency();
    } catch (...) {
        return 0;
    }
}

/// How many frames of output the library produces before the output reflects
/// the input that has reached the end of the input latency.
///
/// This reads a size the library worked out when it was built, so it throws
/// nothing. A caught exception is answered with no latency at all.
int dermixen_signalsmith_output_latency(const DermixenSignalsmithStretch *stretcher) {
    try {
        return stretcher->library->outputLatency();
    } catch (...) {
        return 0;
    }
}

/// Takes all `inputFrames` frames of interleaved input and fills all
/// `outputFrames` frames of interleaved output, which plays the input at
/// `inputFrames / outputFrames` times its original speed without moving its
/// pitch.
///
/// The answer is zero when the block was stretched and one when the buffers
/// this separates the channels into could not be allocated, in which case the
/// output holds whatever it held before.
int dermixen_signalsmith_process(DermixenSignalsmithStretch *stretcher, const float *input,
                                 int inputFrames, float *output, int outputFrames) {
    try {
        stretcher->process(input, inputFrames, output, outputFrames);
        return STATUS_OK;
    } catch (...) {
        return STATUS_FAILED;
    }
}
}
