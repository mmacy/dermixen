#![forbid(unsafe_code)]

//! The render graph: per-track time-stretch, EQ, and volume, summed to one
//! mix. Offline render and real-time preview share this one path: a preview
//! is the render of a span of the mix delivered to an audio device instead
//! of a file.

pub mod audition;
pub mod eq;
pub mod preview;
pub mod render;
pub mod stretch;
pub mod transport;

pub use audition::{
    AUDITION_LOOKAHEAD, Audition, AuditionState, AuditionStatus, TRACK_GAIN, click,
};
pub use eq::{HIGH_CROSSOVER_HZ, LOW_CROSSOVER_HZ, ThreeBandEq};
#[cfg(feature = "playback")]
pub use preview::CpalOutput;
pub use preview::{Feed, Output, PlayReport, device_sample, play};
pub use render::{
    BLOCK_FRAMES, Loader, Progress, RUN_IN, RenderError, Source, mix_length, render, render_range,
    render_to,
};
#[cfg(feature = "signalsmith")]
pub use stretch::SignalsmithStretcher;
pub use stretch::{Passthrough, Resampler, TimeStretcher};
pub use transport::{SendLoader, SendStretchers, Transport, TransportState, TransportStatus};
