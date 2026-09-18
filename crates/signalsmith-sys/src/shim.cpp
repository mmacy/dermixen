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

#include <memory>
#include <vector>

#include "signalsmith-stretch.h"

namespace {
/// Signalsmith Stretch instantiated for the 32-bit float samples Dermixen uses.
using Library = signalsmith::stretch::SignalsmithStretch<float>;
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
    void build() {
        library.reset(new Library(seed));
        library->presetDefault(channels, sampleRate);
    }

    void process(const float *input, int inputFrames, float *output, int outputFrames) {
        // A block with no frames on one side or the other is turned away here
        // rather than passed on, because an empty vector yields a pointer that
        // may be null and the library is not asked to take one.
        if (inputFrames <= 0) {
            // There is no audio to read, so the output is silence.
            for (int sample = 0; sample < outputFrames * channels; ++sample) {
                output[sample] = 0.0f;
            }
            return;
        }
        if (outputFrames <= 0) {
            // There is nowhere to put this block, so all of it is discarded.
            return;
        }
        for (int channel = 0; channel < channels; ++channel) {
            inputChannels[channel].resize(inputFrames);
            outputChannels[channel].resize(outputFrames);
            for (int frame = 0; frame < inputFrames; ++frame) {
                inputChannels[channel][frame] = input[frame * channels + channel];
            }
            inputPointers[channel] = inputChannels[channel].data();
            outputPointers[channel] = outputChannels[channel].data();
        }
        library->process(inputPointers.data(), inputFrames, outputPointers.data(), outputFrames);
        for (int channel = 0; channel < channels; ++channel) {
            for (int frame = 0; frame < outputFrames; ++frame) {
                output[frame * channels + channel] = outputChannels[channel][frame];
            }
        }
    }
};

extern "C" {

/// Creates a stretcher for `channels` channels of audio at `sampleRate` hertz,
/// with `seed` fixing the random numbers the library uses internally.
DermixenSignalsmithStretch *dermixen_signalsmith_new(int channels, float sampleRate, long seed) {
    return new DermixenSignalsmithStretch(channels, sampleRate, seed);
}

/// Destroys a stretcher created by `dermixen_signalsmith_new`.
void dermixen_signalsmith_free(DermixenSignalsmithStretch *stretcher) {
    delete stretcher;
}

/// Puts a stretcher back into the state it was created in, so that the audio
/// fed to it before is forgotten.
///
/// This builds the library object again rather than calling the library's own
/// reset, because that reset clears the buffers but leaves the random engine
/// where it stood. Building again puts the engine back to the same seed, which
/// is what makes two runs either side of a reset produce the same audio. The
/// cost is that the buffers are allocated again.
void dermixen_signalsmith_reset(DermixenSignalsmithStretch *stretcher) {
    stretcher->build();
}

/// How many frames of input the library takes in before its output reflects them.
int dermixen_signalsmith_input_latency(const DermixenSignalsmithStretch *stretcher) {
    return stretcher->library->inputLatency();
}

/// How many frames of output the library produces before the output reflects
/// the input that has reached the end of the input latency.
int dermixen_signalsmith_output_latency(const DermixenSignalsmithStretch *stretcher) {
    return stretcher->library->outputLatency();
}

/// Takes all `inputFrames` frames of interleaved input and fills all
/// `outputFrames` frames of interleaved output, which plays the input at
/// `inputFrames / outputFrames` times its original speed without moving its
/// pitch.
void dermixen_signalsmith_process(DermixenSignalsmithStretch *stretcher, const float *input,
                                  int inputFrames, float *output, int outputFrames) {
    stretcher->process(input, inputFrames, output, outputFrames);
}
}
