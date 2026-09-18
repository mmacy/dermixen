//! The beat and tempo tracker from the aubio library, made callable from Rust.
//!
//! aubio is a C library. This crate vendors the part of its source that beat
//! tracking reaches, compiles it, and wraps the handful of C functions that
//! drive it in one Rust type, [`Tempo`]. The crates that make up the app itself
//! forbid unsafe code, so all of the risk of these calls sits in this file.
//!
//! Feed a track through [`Tempo`] one hop at a time, in order, from the start.
//! Each hop is a block of mono samples of exactly the size given to
//! [`Tempo::new`], and each call reports whether a beat fell inside that
//! block. When the whole track has gone through, [`Tempo::bpm`] and
//! [`Tempo::confidence`] describe what the tracker settled on.

use std::ffi::{c_char, c_uint};
use std::ptr::NonNull;

/// aubio's vector of samples: a length and a pointer to that many floats.
///
/// aubio reads the pointer and the length and never frees either, so the
/// caller may point one of these at a Rust slice for the duration of a call.
#[repr(C)]
struct FVec {
    length: c_uint,
    data: *mut f32,
}

/// aubio's tempo tracker, whose fields are private to the C library.
#[repr(C)]
struct AubioTempo {
    _opaque: [u8; 0],
}

unsafe extern "C" {
    fn new_aubio_tempo(
        method: *const c_char,
        buf_size: c_uint,
        hop_size: c_uint,
        samplerate: c_uint,
    ) -> *mut AubioTempo;
    fn del_aubio_tempo(tempo: *mut AubioTempo);
    fn aubio_tempo_do(tempo: *mut AubioTempo, input: *const FVec, output: *mut FVec);
    fn aubio_tempo_get_last(tempo: *mut AubioTempo) -> c_uint;
    fn aubio_tempo_get_bpm(tempo: *mut AubioTempo) -> f32;
    fn aubio_tempo_get_confidence(tempo: *mut AubioTempo) -> f32;
}

/// aubio's beat tracker, following one track from its start to its end.
///
/// Create one with [`Tempo::new`], push the whole track through
/// [`Tempo::feed`] in order, then read [`Tempo::bpm`] and
/// [`Tempo::confidence`]. A tracker cannot be rewound: analyzing a second
/// track needs a second [`Tempo`].
pub struct Tempo {
    tracker: NonNull<AubioTempo>,
    hop: usize,
    fed: usize,
}

impl Tempo {
    /// Starts a beat tracker.
    ///
    /// `window` is how many samples of the track the tracker looks at in one
    /// step and `hop` is how far it moves between steps, so `hop` samples of
    /// new audio arrive each step and the rest of the window is audio it has
    /// already seen. `window` must be at least two samples and at least as
    /// large as `hop`, and `sample_rate` must not be zero.
    ///
    /// Returns `None` when aubio rejects those settings or cannot allocate its
    /// working buffers. aubio prints the reason on standard error when it
    /// rejects them.
    pub fn new(window: usize, hop: usize, sample_rate: u32) -> Option<Tempo> {
        let window = c_uint::try_from(window).ok()?;
        let hop_size = c_uint::try_from(hop).ok()?;
        // "default" is the name aubio gives its own recommended settings,
        // which measure how much the spectrum changed since the last window.
        let method = c"default";
        // Safety: the method name is a null-terminated string that outlives
        // the call, and aubio checks the three sizes itself and answers with a
        // null pointer rather than misbehaving when it dislikes them.
        let tracker = unsafe { new_aubio_tempo(method.as_ptr(), window, hop_size, sample_rate) };
        Some(Tempo {
            tracker: NonNull::new(tracker)?,
            hop,
            fed: 0,
        })
    }

    /// The number of samples one call to [`Tempo::feed`] takes.
    pub fn hop(&self) -> usize {
        self.hop
    }

    /// The number of samples fed so far.
    pub fn fed(&self) -> usize {
        self.fed
    }

    /// Pushes the next hop of the track through the tracker.
    ///
    /// The samples are mono, at the sample rate given to [`Tempo::new`], and
    /// follow directly on from the previous call. When a beat falls inside
    /// this hop, the answer is the position of that beat in samples from the
    /// start of the track; otherwise it is `None`.
    ///
    /// The position is counted from the first sample ever fed to this tracker,
    /// not from the start of this block, and aubio has already accounted for
    /// the time its own analysis took to reach the decision. On a test signal
    /// whose beats sit at known places, the positions aubio reports land within
    /// a few milliseconds of those places. A caller should use each position as
    /// it stands and subtract nothing from it.
    ///
    /// # Panics
    ///
    /// Panics when `samples` is not exactly [`Tempo::hop`] samples long. aubio
    /// reads that many whatever it is given, so a short block would read past
    /// the end of it.
    pub fn feed(&mut self, samples: &[f32]) -> Option<usize> {
        assert_eq!(
            samples.len(),
            self.hop,
            "an aubio tempo tracker takes exactly one hop of samples at a time"
        );
        let input = FVec {
            length: self.hop as c_uint,
            // aubio only reads the input, but its C declaration is not marked
            // as read-only, so the pointer has to be a mutable one.
            data: samples.as_ptr().cast_mut(),
        };
        let mut found = [0.0f32];
        let mut output = FVec {
            length: 1,
            data: found.as_mut_ptr(),
        };
        // Safety: both vectors describe live Rust memory of exactly the length
        // they claim, aubio writes only to the one-element output, and neither
        // pointer is kept after the call returns.
        unsafe { aubio_tempo_do(self.tracker.as_ptr(), &input, &mut output) };
        self.fed += self.hop;
        if found[0] == 0.0 {
            return None;
        }
        // Safety: the tracker is live and this only reads from it.
        let at = unsafe { aubio_tempo_get_last(self.tracker.as_ptr()) };
        Some(at as usize)
    }

    /// The tempo the tracker currently believes the track has, in beats per
    /// minute, or zero before it has heard enough to believe anything.
    pub fn bpm(&self) -> f64 {
        // Safety: the tracker is live and this only reads from it.
        f64::from(unsafe { aubio_tempo_get_bpm(self.tracker.as_ptr()) })
    }

    /// How strongly the track's onsets line up with the tempo the tracker
    /// found, from zero for no agreement to one for perfect agreement.
    ///
    /// aubio's own figure is one peak of a correlation divided by the sum of
    /// that whole correlation, which has no upper limit and reaches above six
    /// on a bare kick track. This function clamps aubio's figure into the range
    /// from zero to one, so that a caller can compare the result against the
    /// confidence of any other analyzer and against the threshold at which a
    /// track gets flagged for the manual grid editor.
    pub fn confidence(&self) -> f64 {
        // Safety: the tracker is live and this only reads from it.
        let raw = f64::from(unsafe { aubio_tempo_get_confidence(self.tracker.as_ptr()) });
        if raw.is_finite() {
            raw.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

impl Drop for Tempo {
    fn drop(&mut self) {
        // Safety: this pointer came from new_aubio_tempo, nothing else holds
        // it, and Drop runs once.
        unsafe { del_aubio_tempo(self.tracker.as_ptr()) };
    }
}

impl std::fmt::Debug for Tempo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tempo")
            .field("hop", &self.hop)
            .field("fed", &self.fed)
            .field("bpm", &self.bpm())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A kick drum on every beat at the given tempo, for the given number of
    /// seconds: the same shape of test signal the analysis crate uses.
    fn kicks(bpm: f64, seconds: f64) -> Vec<f32> {
        let rate = 44_100.0;
        let mut samples = vec![0.0f32; (rate * seconds) as usize];
        let period = 60.0 / bpm;
        let mut beat = 0;
        loop {
            let start = (beat as f64 * period * rate).round() as usize;
            if start >= samples.len() {
                break;
            }
            for i in 0..(0.1 * rate) as usize {
                let Some(sample) = samples.get_mut(start + i) else {
                    break;
                };
                let t = i as f64 / rate;
                let tau = std::f64::consts::TAU;
                *sample += ((tau * 60.0 * t).sin() * (-t / 0.015).exp() * 0.9) as f32;
            }
            beat += 1;
        }
        samples
    }

    #[test]
    fn a_tracker_reports_beats_and_a_tempo() {
        let mut tempo = Tempo::new(1024, 128, 44_100).unwrap();
        assert_eq!(tempo.hop(), 128);
        let samples = kicks(128.0, 20.0);
        let mut beats = Vec::new();
        for block in samples.chunks_exact(tempo.hop()) {
            if let Some(at) = tempo.feed(block) {
                beats.push(at as f64 / 44_100.0);
            }
        }
        // Twenty seconds at 128 beats per minute holds forty-two beats, and
        // the tracker needs a second or two before it reports its first.
        assert!(beats.len() > 35, "found {} beats", beats.len());
        assert!(
            (tempo.bpm() - 128.0).abs() < 5.0,
            "aubio said {}",
            tempo.bpm()
        );
        let period = 60.0 / 128.0;
        for beat in &beats {
            let nearest = (beat / period).round() * period;
            assert!(
                (beat - nearest).abs() < 0.02,
                "a beat at {beat} seconds is not on the grid"
            );
        }
        assert!((0.0..=1.0).contains(&tempo.confidence()));
        assert_eq!(tempo.fed(), samples.len() - samples.len() % 128);
    }

    #[test]
    fn silence_holds_no_beats() {
        let mut tempo = Tempo::new(1024, 128, 44_100).unwrap();
        let silence = [0.0f32; 128];
        for _ in 0..800 {
            assert_eq!(tempo.feed(&silence), None);
        }
        assert!((0.0..=1.0).contains(&tempo.confidence()));
    }

    #[test]
    fn impossible_settings_give_no_tracker() {
        assert!(Tempo::new(512, 1024, 44_100).is_none());
        assert!(Tempo::new(1024, 512, 0).is_none());
    }
}
