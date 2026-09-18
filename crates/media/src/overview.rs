//! A waveform overview: the peaks of a track per bucket of frames, which is
//! what the timeline draws instead of the audio itself.
//!
//! A decoded track is about twenty megabytes a minute, so the app cannot
//! keep every track of a mix decoded; it keeps an overview per track,
//! computed once when the track is decoded, and drops the audio. An
//! overview at a tenth of a second per bucket is tens of kilobytes per
//! track, and it is enough to draw the whole mix at any zoom the timeline
//! offers: where a pixel spans several buckets the app draws the extreme
//! over them, and where a pixel is narrower than a bucket it draws that
//! bucket's peak. Overviews live in memory and are not written to disk.

use std::ops::Range;

use dermixen_core::Samples;

use crate::Audio;

/// The extremes of the samples in a stretch of audio, over both channels.
///
/// Silence is `low` and `high` both zero.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Peak {
    /// The smallest sample value in the stretch.
    pub low: f32,
    /// The largest sample value in the stretch.
    pub high: f32,
}

/// The peaks of a track per bucket of frames.
#[derive(Debug, Clone, PartialEq)]
pub struct Overview {
    /// Frames per bucket.
    bucket: Samples,
    /// The length of the audio the overview was made from.
    length: Samples,
    /// One peak per bucket, in order.
    peaks: Vec<Peak>,
}

impl Overview {
    /// The peaks of `audio` per `bucket` frames: bucket `n` is the extreme
    /// over both channels of frames `n * bucket` up to but not including
    /// `(n + 1) * bucket`, and the last bucket holds whatever frames are
    /// left when the length is not a whole number of buckets. A bucket
    /// below one frame is taken as one frame. Audio with no frames has no
    /// peaks. A sample that is not a number is ignored, and a bucket with no
    /// other finite sample is silence.
    pub fn of(audio: &Audio, bucket: Samples) -> Overview {
        let bucket = Samples(bucket.0.max(1));
        let length = audio.len();
        let bucket_len = bucket.0 as usize;
        let mut peaks = Vec::with_capacity(audio.frames.len().div_ceil(bucket_len));
        let mut start = 0usize;
        while start < audio.frames.len() {
            let end = (start + bucket_len).min(audio.frames.len());
            let mut low = f32::INFINITY;
            let mut high = f32::NEG_INFINITY;
            for frame in &audio.frames[start..end] {
                for &sample in frame {
                    if sample.is_finite() {
                        low = low.min(sample);
                        high = high.max(sample);
                    }
                }
            }
            peaks.push(if low.is_finite() {
                Peak { low, high }
            } else {
                Peak::default()
            });
            start = end;
        }
        Overview {
            bucket,
            length,
            peaks,
        }
    }

    /// Frames per bucket, as used: one frame when the overview was asked
    /// for less.
    pub fn bucket(&self) -> Samples {
        self.bucket
    }

    /// The length of the audio the overview was made from.
    pub fn length(&self) -> Samples {
        self.length
    }

    /// One peak per bucket, in order.
    pub fn peaks(&self) -> &[Peak] {
        &self.peaks
    }

    /// The extreme over every bucket that holds at least one frame of
    /// `span`, which is a range of frames from the start of the audio. A
    /// span that holds no frame of the audio, because it is empty or lies
    /// outside the audio, is silence; the part of a span before the first
    /// frame or after the last is ignored.
    pub fn peak_over(&self, span: Range<Samples>) -> Peak {
        if span.start >= span.end {
            return Peak::default();
        }
        let start = span.start.0.max(0);
        let end = span.end.0.min(self.length.0);
        if start >= end {
            return Peak::default();
        }
        let bucket_len = self.bucket.0;
        let first = (start / bucket_len) as usize;
        let last = ((end - 1) / bucket_len) as usize;
        let mut low = f32::INFINITY;
        let mut high = f32::NEG_INFINITY;
        for p in &self.peaks[first..=last] {
            low = low.min(p.low);
            high = high.max(p.high);
        }
        Peak { low, high }
    }
}
