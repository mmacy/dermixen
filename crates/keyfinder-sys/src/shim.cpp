// A C boundary around libkeyfinder, which is a C++ library that reports
// failure by throwing.
//
// Rust cannot call C++ classes and cannot survive a C++ exception unwinding
// into it, so this file exposes the one operation Dermixen needs as a plain C
// function, catches everything libkeyfinder can throw, and turns each throw
// into a status code and a message the caller can read.
//
// Audio crosses the boundary as one channel of 32-bit float samples, because
// libkeyfinder's own low-pass filter works on one channel only and the caller
// has already mixed the track down. The whole track goes across in one call: this
// is the same sequence of steps as libkeyfinder's own `keyOfAudio`, opened up
// only so that the scores behind the answer can be read as well as the answer
// itself.

#include <cmath>
#include <cstddef>
#include <cstring>
#include <exception>
#include <utility>
#include <vector>

#include "fftadapter.h"
#include "keyfinder.h"
#include "toneprofiles.h"

namespace {

/// How many keys libkeyfinder scores: twelve tonics in each of two modes.
const unsigned int KEY_COUNT = 24;

/// The analysis finished and named a key.
const int STATUS_OK = 0;
/// The analysis finished and heard nothing it could take a key from.
const int STATUS_SILENCE = 1;
/// libkeyfinder reported a failure, and the message states what went wrong.
const int STATUS_FAILED = 2;

/// Copies `text` into `message`, which holds `messageLength` bytes including
/// the terminator, cutting the text short if it does not fit.
///
/// This runs while an exception is being handled, so it allocates nothing. A
/// second failure raised on the way out of the first one would leave the
/// program with no way to report either.
void reportMessage(const char *text, char *message, size_t messageLength) {
    if (message == nullptr || messageLength == 0) {
        return;
    }
    if (text == nullptr) {
        message[0] = '\0';
        return;
    }
    size_t length = std::strlen(text);
    size_t copied = length < messageLength - 1 ? length : messageLength - 1;
    std::memcpy(message, text, copied);
    message[copied] = '\0';
}

} // namespace

extern "C" {

/// Builds the tables libkeyfinder shares between analyses.
///
/// libkeyfinder builds its two tone profiles the first time any analysis asks
/// for them and keeps them for every later analysis, without guarding that
/// first build against a second thread arriving at the same moment. Calling
/// this once, before any analysis runs, builds both profiles while only one
/// thread is looking, after which every analysis only reads them.
///
/// Building the two tables allocates, so it can throw, and an exception
/// unwinding out of here and into Rust would be undefined behavior. A failure
/// is therefore swallowed rather than reported: the first analysis asks
/// libkeyfinder for the same two tables, and if building them fails again it
/// fails inside `dermixen_keyfinder_analyze`, which does report the reason.
void dermixen_keyfinder_prepare() {
    try {
        KeyFinder::toneProfileMajor();
        KeyFinder::toneProfileMinor();
    } catch (...) {
    }
}

/// Finds the key of a whole track.
///
/// `samples` is one channel of `count` samples at `frameRate` samples per
/// second. On success `key` is set to the position of the key libkeyfinder
/// named, counted the way libkeyfinder counts: A major is zero, A minor is
/// one, B flat major is two, and so on through the twelve tonics in the two
/// modes. `confidence` is set to the margin by which the best-scoring key beat
/// the second best, as a fraction of the best score, from zero when the two
/// tied to one when the second best scored nothing at all.
///
/// The answer is zero when a key was named, one when the track held nothing
/// to take a key from, and two when libkeyfinder reported a failure, in which
/// case `message` holds the reason as a null-terminated string of at most
/// `messageLength` bytes.
int dermixen_keyfinder_analyze(const float *samples, size_t count, unsigned int frameRate,
                               int *key, double *confidence, char *message,
                               size_t messageLength) {
    try {
        KeyFinder::AudioData audio;
        audio.setChannels(1);
        audio.setFrameRate(frameRate);
        audio.addToSampleCount((unsigned int)count);
        for (size_t sample = 0; sample < count; sample++) {
            audio.setSample((unsigned int)sample, (double)samples[sample]);
        }

        KeyFinder::KeyFinder finder;
        KeyFinder::Workspace workspace;
        // The audio is moved rather than copied, so a long track is held once
        // rather than twice. Everything after this point matches
        // libkeyfinder's own `keyOfAudio` step for step.
        finder.progressiveChromagram(std::move(audio), workspace);
        finder.finalChromagram(workspace);

        if (workspace.chromagram == nullptr || workspace.chromagram->getHops() == 0) {
            return STATUS_SILENCE;
        }

        KeyFinder::key_t found = finder.keyOfChromagram(workspace);
        if (found == KeyFinder::SILENCE) {
            return STATUS_SILENCE;
        }

        // libkeyfinder decides the key by scoring the track's average
        // chromagram against each of the twenty-four keys and taking the
        // highest score. It reports only which key won, so the scores are
        // worked out again here, the same way and from the same chromagram,
        // to see how far ahead of the runner-up the winner finished.
        std::vector<double> chroma = workspace.chromagram->collapseToOneHop();
        KeyFinder::ToneProfile major(KeyFinder::toneProfileMajor());
        KeyFinder::ToneProfile minor(KeyFinder::toneProfileMinor());
        double best = 0.0;
        double second = 0.0;
        for (unsigned int tonic = 0; tonic * 2 < KEY_COUNT; tonic++) {
            double pair[2] = {major.cosineSimilarity(chroma, (int)tonic),
                              minor.cosineSimilarity(chroma, (int)tonic)};
            for (unsigned int mode = 0; mode < 2; mode++) {
                if (pair[mode] > best) {
                    second = best;
                    best = pair[mode];
                } else if (pair[mode] > second) {
                    second = pair[mode];
                }
            }
        }

        double margin = best > 0.0 ? (best - second) / best : 0.0;
        if (!std::isfinite(margin)) {
            margin = 0.0;
        }
        if (margin < 0.0) {
            margin = 0.0;
        }
        if (margin > 1.0) {
            margin = 1.0;
        }

        *key = (int)found;
        *confidence = margin;
        return STATUS_OK;
    } catch (const std::exception &failure) {
        reportMessage(failure.what(), message, messageLength);
        return STATUS_FAILED;
    } catch (...) {
        reportMessage("libkeyfinder failed without saying why", message, messageLength);
        return STATUS_FAILED;
    }
}

/// How many samples at `frameRate` samples per second fill one of the frames
/// libkeyfinder works a spectrum out from.
///
/// libkeyfinder turns nothing away for being short: it pads a short buffer
/// with silence until one frame is full and names a key from the result, which
/// is a key read mostly from silence. A caller that would rather refuse such a
/// fragment than believe the answer needs this figure to compare against. The
/// arithmetic here is the arithmetic in libkeyfinder's own preprocessing step,
/// so it stays true to whatever the vendored source says.
size_t dermixen_keyfinder_frame_samples(unsigned int frameRate) {
    double downsampleCutoff = KeyFinder::getLastFrequency() * 1.10;
    double factor = std::floor(frameRate / 2.0 / downsampleCutoff);
    if (!(factor >= 1.0)) {
        // A sample rate this low is one libkeyfinder turns away anyway, for
        // putting the notes it looks for above the Nyquist frequency.
        factor = 1.0;
    }
    return (size_t)FFTFRAMESIZE * (size_t)factor;
}

/// Runs the forward transform over `frameSize` samples of `input` and writes
/// the real and imaginary part of every bin into `real` and `imaginary`, each
/// of which holds `frameSize` values.
///
/// This exists so that the transform Dermixen substitutes for FFTW can be
/// checked, bin by bin, against a transform worked out from the definition.
/// Nothing in the analysis path calls it.
int dermixen_keyfinder_forward_transform(const double *input, size_t frameSize, double *real,
                                         double *imaginary, char *message, size_t messageLength) {
    try {
        KeyFinder::FftAdapter transform((unsigned int)frameSize);
        for (size_t sample = 0; sample < frameSize; sample++) {
            transform.setInput((unsigned int)sample, input[sample]);
        }
        transform.execute();
        for (size_t bin = 0; bin < frameSize; bin++) {
            real[bin] = transform.getOutputReal((unsigned int)bin);
            imaginary[bin] = transform.getOutputImaginary((unsigned int)bin);
        }
        return STATUS_OK;
    } catch (const std::exception &failure) {
        reportMessage(failure.what(), message, messageLength);
        return STATUS_FAILED;
    } catch (...) {
        reportMessage("the forward transform failed without saying why", message, messageLength);
        return STATUS_FAILED;
    }
}

/// Runs the inverse transform over the `frameSize` bins held in `real` and
/// `imaginary` and writes `frameSize` samples into `output`.
///
/// This exists for the same reason as the forward transform above, and
/// nothing in the analysis path calls it either.
int dermixen_keyfinder_inverse_transform(const double *real, const double *imaginary,
                                         size_t frameSize, double *output, char *message,
                                         size_t messageLength) {
    try {
        KeyFinder::InverseFftAdapter transform((unsigned int)frameSize);
        for (size_t bin = 0; bin < frameSize; bin++) {
            transform.setInput((unsigned int)bin, real[bin], imaginary[bin]);
        }
        transform.execute();
        for (size_t sample = 0; sample < frameSize; sample++) {
            output[sample] = transform.getOutput((unsigned int)sample);
        }
        return STATUS_OK;
    } catch (const std::exception &failure) {
        reportMessage(failure.what(), message, messageLength);
        return STATUS_FAILED;
    } catch (...) {
        reportMessage("the inverse transform failed without saying why", message, messageLength);
        return STATUS_FAILED;
    }
}
}
