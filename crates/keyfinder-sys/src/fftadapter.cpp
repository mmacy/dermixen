// The fast Fourier transform libkeyfinder runs on, built on the vendored
// Ooura transform instead of FFTW.
//
// libkeyfinder declares its transform in `fftadapter.h` and implements it in
// an `fftadapter.cpp` that calls FFTW, a library Dermixen would then have to
// find installed on every machine it builds on. The libkeyfinder authors left
// room for the swap, and wrote a comment in their own implementation file
// explaining that the FFTW header is included there "to allow substitution of
// a separate implementation .cpp". This file is that substitution. It is the
// only part of libkeyfinder Dermixen writes for itself, and it leaves the
// vendored header exactly as the libkeyfinder authors wrote it, so every
// other vendored file compiles unchanged.
//
// The two classes here answer the same questions FFTW answered, in the same
// units and the same array positions, so the numbers libkeyfinder works from
// are the numbers it would have had from FFTW:
//
//   - The forward transform takes a frame of real samples and reports, for
//     each frequency bin from zero up to half the frame size, how much of
//     that frequency the frame holds and at what phase. Bins above half the
//     frame size stay at zero, which is where FFTW left them too, because a
//     frame of real samples has nothing to say about them.
//   - The inverse transform takes a frequency response and reports the run of
//     samples that produces it, without dividing by the frame size. The
//     divide happens in `getOutput`, which is upstream's own arrangement.
//
// The Ooura transform states its results as a cosine part and a sine part.
// The sine part is the negative of the imaginary part every other library
// reports, so this file flips its sign on the way out of the forward
// transform and on the way into the inverse one.

#include "fftadapter.h"

#include <cmath>
#include <vector>

extern "C" {
// The vendored Ooura real transform. `isgn` is 1 for the forward direction
// and -1 for the inverse. `data` is read and written in place, `bitReversal`
// and `table` are work areas the transform fills the first time it is called
// with `bitReversal[0]` set to zero. The build gives the function this name;
// see the crate's `build.rs`.
void dermixen_keyfinder_ooura_rdft(int n, int isgn, double *data, int *bitReversal, double *table);
}

namespace {

/// The work areas one Ooura transform of a fixed frame size needs, along with
/// the frame the transform reads and writes in place.
class OouraTransform {
public:
    explicit OouraTransform(unsigned int frameSize)
        : frame(frameSize, 0.0),
          // Ooura asks for two entries plus the square root of half the frame
          // size, rounded up.
          bitReversal(2 + (unsigned int)std::ceil(std::sqrt(frameSize / 2.0)), 0),
          table(frameSize / 2, 0.0) {
        // A zero in the first entry tells the transform to build both work
        // areas on its first call.
        bitReversal[0] = 0;
    }

    /// Runs the transform over `frame`, forward when `forward` is true and
    /// inverse when it is false.
    void run(bool forward) {
        dermixen_keyfinder_ooura_rdft((int)frame.size(), forward ? 1 : -1, frame.data(),
                                      bitReversal.data(), table.data());
    }

    std::vector<double> frame;

private:
    std::vector<int> bitReversal;
    std::vector<double> table;
};

/// Whether `frameSize` is a power of two of at least two, which is the only
/// shape of frame the Ooura transform handles.
bool isUsableFrameSize(unsigned int frameSize) {
    return frameSize >= 2 && (frameSize & (frameSize - 1)) == 0;
}

} // namespace

namespace KeyFinder {

/// Everything one forward transform owns: the samples waiting to go in, the
/// frequency response that came out, and the transform itself.
class FftAdapterPrivate {
public:
    explicit FftAdapterPrivate(unsigned int frameSize)
        : input(frameSize, 0.0), real(frameSize, 0.0), imaginary(frameSize, 0.0),
          transform(frameSize) {}

    std::vector<double> input;
    std::vector<double> real;
    std::vector<double> imaginary;
    OouraTransform transform;
};

FftAdapter::FftAdapter(unsigned int inFrameSize) : priv(nullptr) {
    if (!isUsableFrameSize(inFrameSize)) {
        throw Exception("FFT frame size must be a power of two of at least two");
    }
    frameSize = inFrameSize;
    priv = new FftAdapterPrivate(inFrameSize);
}

FftAdapter::~FftAdapter() {
    delete priv;
}

unsigned int FftAdapter::getFrameSize() const {
    return frameSize;
}

void FftAdapter::setInput(unsigned int i, double real) {
    if (i >= frameSize) {
        std::ostringstream ss;
        ss << "Cannot set out-of-bounds sample (" << i << "/" << frameSize << ")";
        throw Exception(ss.str().c_str());
    }
    if (!std::isfinite(real)) {
        throw Exception("Cannot set sample to NaN");
    }
    priv->input[i] = real;
}

double FftAdapter::getOutputReal(unsigned int i) const {
    if (i >= frameSize) {
        std::ostringstream ss;
        ss << "Cannot get out-of-bounds sample (" << i << "/" << frameSize << ")";
        throw Exception(ss.str().c_str());
    }
    return priv->real[i];
}

double FftAdapter::getOutputImaginary(unsigned int i) const {
    if (i >= frameSize) {
        std::ostringstream ss;
        ss << "Cannot get out-of-bounds sample (" << i << "/" << frameSize << ")";
        throw Exception(ss.str().c_str());
    }
    return priv->imaginary[i];
}

double FftAdapter::getOutputMagnitude(unsigned int i) const {
    double real = getOutputReal(i);
    double imaginary = getOutputImaginary(i);
    return std::sqrt(real * real + imaginary * imaginary);
}

void FftAdapter::execute() {
    priv->transform.frame = priv->input;
    priv->transform.run(true);

    // Ooura packs the answer into one array: the bin at zero and the bin at
    // half the frame size are purely real and share the first two positions,
    // and every bin between them takes a pair of positions.
    const unsigned int half = frameSize / 2;
    priv->real[0] = priv->transform.frame[0];
    priv->imaginary[0] = 0.0;
    priv->real[half] = priv->transform.frame[1];
    priv->imaginary[half] = 0.0;
    for (unsigned int bin = 1; bin < half; bin++) {
        priv->real[bin] = priv->transform.frame[bin * 2];
        priv->imaginary[bin] = -priv->transform.frame[bin * 2 + 1];
    }
    // Bins above half the frame size say nothing about a frame of real
    // samples, and FFTW left them at zero, so they stay at zero here.
}

// ================================= INVERSE =================================

/// Everything one inverse transform owns: the frequency response waiting to
/// go in, the samples that came out, and the transform itself.
class InverseFftAdapterPrivate {
public:
    explicit InverseFftAdapterPrivate(unsigned int frameSize)
        : real(frameSize, 0.0), imaginary(frameSize, 0.0), output(frameSize, 0.0),
          transform(frameSize) {}

    std::vector<double> real;
    std::vector<double> imaginary;
    std::vector<double> output;
    OouraTransform transform;
};

InverseFftAdapter::InverseFftAdapter(unsigned int inFrameSize) : priv(nullptr) {
    if (!isUsableFrameSize(inFrameSize)) {
        throw Exception("FFT frame size must be a power of two of at least two");
    }
    frameSize = inFrameSize;
    priv = new InverseFftAdapterPrivate(inFrameSize);
}

InverseFftAdapter::~InverseFftAdapter() {
    delete priv;
}

unsigned int InverseFftAdapter::getFrameSize() const {
    return frameSize;
}

void InverseFftAdapter::setInput(unsigned int i, double real, double imaginary) {
    if (i >= frameSize) {
        std::ostringstream ss;
        ss << "Cannot set out-of-bounds sample (" << i << "/" << frameSize << ")";
        throw Exception(ss.str().c_str());
    }
    if (!std::isfinite(real) || !std::isfinite(imaginary)) {
        throw Exception("Cannot set sample to NaN");
    }
    priv->real[i] = real;
    priv->imaginary[i] = imaginary;
}

double InverseFftAdapter::getOutput(unsigned int i) const {
    if (i >= frameSize) {
        std::ostringstream ss;
        ss << "Cannot get out-of-bounds sample (" << i << "/" << frameSize << ")";
        throw Exception(ss.str().c_str());
    }
    // divide by frameSize to normalise
    return priv->output[i] / frameSize;
}

void InverseFftAdapter::execute() {
    // Only the bins from zero up to half the frame size are read, because a
    // frame of real samples is what comes back out and the bins above the
    // halfway point are fixed by the ones below it. FFTW read the same range
    // and ignored the rest.
    const unsigned int half = frameSize / 2;
    priv->transform.frame[0] = priv->real[0];
    priv->transform.frame[1] = priv->real[half];
    for (unsigned int bin = 1; bin < half; bin++) {
        priv->transform.frame[bin * 2] = priv->real[bin];
        priv->transform.frame[bin * 2 + 1] = -priv->imaginary[bin];
    }

    priv->transform.run(false);

    // Ooura halves the result of the inverse transform; FFTW did not, and
    // `getOutput` above divides by the frame size on the assumption that
    // nothing else has. Doubling here puts the two libraries back in step.
    for (unsigned int sample = 0; sample < frameSize; sample++) {
        priv->output[sample] = priv->transform.frame[sample] * 2.0;
    }
}

} // namespace KeyFinder
