//! What the window needs from the audio files behind a mix: the decoded
//! audio it keeps for the preview, the thread that reads files without
//! holding up a repaint, and the loader, stretcher, and output the preview
//! is started with.
//!
//! The window makes the same choices `dermixen play` makes, which
//! `docs/cli.md` describes: the pitch-preserving stretcher for a track with
//! keylock when it is built in, plain resampling otherwise, and the
//! machine's default audio device for the sound.

use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, TryIter, TryRecvError, channel};
use std::sync::{Arc, Mutex, MutexGuard};

use dermixen_core::{ContentHash, Samples, Seconds, Track};
use dermixen_engine::{Output, Resampler, SendLoader, SendStretchers, Source, TimeStretcher};
use dermixen_library::{Index, PhraseRecord};
use dermixen_media::{Audio, Frame, Overview, decode};

/// How many tracks' decoded audio the window keeps.
///
/// A transition sounds two tracks at once, so four leaves room for the pair
/// being heard and the pair on either side of it. Whole tracks are large,
/// which is why the window keeps a few rather than all of them.
pub const TRACKS_KEPT: usize = 4;

/// How many frames of audio one column of a waveform overview covers.
///
/// At 44.1 kHz a bucket of 441 frames is a hundredth of a second, which is
/// finer than any screen draws a mix of several minutes and small enough to
/// keep for every track of a mix once its audio has been let go.
pub const OVERVIEW_BUCKET: Samples = Samples(441);

/// How much of the mix the preview keeps rendered ahead of the device, which
/// is what `dermixen play` keeps.
pub const LOOKAHEAD: Seconds = Seconds(2.0);

/// A track's decoded audio, shared between the window and the render thread.
///
/// The render holds one of these for as long as it is sounding the track, so
/// audio the window has since let go of stays alive until the render is done
/// with it.
struct SharedAudio(Arc<Audio>);

impl Source for SharedAudio {
    fn frames(&self) -> &[Frame] {
        &self.0.frames
    }
}

/// One track's decoded audio, kept for as long as the store keeps it.
struct Kept {
    /// The track's content hash.
    hash: ContentHash,
    /// Its decoded audio.
    audio: Arc<Audio>,
    /// Whether the render has ever asked for this track.
    ///
    /// A track read ahead that the render has never asked for ranks below
    /// every track it has, so read-ahead is always what the store lets go of
    /// first and a track that is sounding is never let go for one that is
    /// not.
    heard: bool,
}

/// The decoded audio of the tracks heard most recently.
///
/// The window and the transport's render thread share one of these, so a
/// track the render has already decoded is handed straight over when the
/// document is replaced or the playhead is moved, rather than decoded again.
/// At most [`TRACKS_KEPT`] tracks are kept.
#[derive(Default)]
pub struct AudioCache {
    /// The tracks kept, most recently used first, with every track the
    /// render has asked for ahead of every track only read ahead.
    kept: Vec<Kept>,
}

impl AudioCache {
    /// The audio of the track with this hash, if it is kept, which counts as
    /// the render asking for it.
    fn heard(&mut self, hash: ContentHash) -> Option<Arc<Audio>> {
        let found = self.kept.iter().position(|kept| kept.hash == hash)?;
        let mut entry = self.kept.remove(found);
        entry.heard = true;
        let audio = Arc::clone(&entry.audio);
        self.kept.insert(0, entry);
        Some(audio)
    }

    /// Keeps a track the render asked for as the most recently used.
    fn keep(&mut self, hash: ContentHash, audio: Arc<Audio>) {
        self.kept.retain(|kept| kept.hash != hash);
        self.kept.insert(
            0,
            Kept {
                hash,
                audio,
                heard: true,
            },
        );
        self.let_one_go();
    }

    /// Keeps a track the reading thread has decoded, while there is room for
    /// it, ranked below every track the render has asked for.
    fn read_ahead(&mut self, hash: ContentHash, audio: Arc<Audio>) {
        if self.kept.len() >= TRACKS_KEPT || self.kept.iter().any(|kept| kept.hash == hash) {
            return;
        }
        self.kept.push(Kept {
            hash,
            audio,
            heard: false,
        });
    }

    /// Lets one track go when more than [`TRACKS_KEPT`] are kept: the last
    /// track the render has never asked for, or, when it has asked for every
    /// one of them, the one it asked for longest ago.
    fn let_one_go(&mut self) {
        if self.kept.len() <= TRACKS_KEPT {
            return;
        }
        let going = self
            .kept
            .iter()
            .rposition(|kept| !kept.heard)
            .unwrap_or(self.kept.len() - 1);
        self.kept.remove(going);
    }
}

/// Locks the shared audio, taking it back from a thread that panicked while
/// holding it.
///
/// Nothing but a list of decoded tracks is kept behind this lock, so there is
/// no half-finished change for a panic to have left behind, and a window that
/// stopped decoding is better than a window that stops.
fn shared(cache: &Mutex<AudioCache>) -> MutexGuard<'_, AudioCache> {
    cache.lock().unwrap_or_else(|held| held.into_inner())
}

/// The audio of the track with `hash`, when the window already holds it,
/// which counts as the render asking for it and so keeps it from being the
/// next track let go.
///
/// The grid editor asks for its track's audio this way when it opens, since
/// the reading thread has usually decoded it already, and starts a
/// [`Decoding`] only when it has not.
pub fn kept(cache: &Mutex<AudioCache>, hash: ContentHash) -> Option<Arc<Audio>> {
    shared(cache).heard(hash)
}

/// The audio of a track, from the store when it is there and from the file
/// when it is not, keeping what was decoded.
fn audio_of(
    cache: &Mutex<AudioCache>,
    hash: ContentHash,
    path: &Path,
) -> Result<Arc<Audio>, String> {
    if let Some(audio) = shared(cache).heard(hash) {
        return Ok(audio);
    }
    let decoded = decode(path).map_err(|problem| problem.to_string())?;
    let audio = Arc::new(decoded.audio);
    shared(cache).keep(hash, Arc::clone(&audio));
    Ok(audio)
}

/// One track being read for the grid editor on a thread of its own, so that
/// opening the editor on a track whose audio the window has let go of costs
/// no pause in the window.
pub struct Decoding {
    /// Where the file is, so that a reading that came to nothing can name
    /// it.
    path: PathBuf,
    /// The audio, or the reason the file could not be read, once the thread
    /// is done.
    done: Receiver<Result<Arc<Audio>, String>>,
}

impl Decoding {
    /// Starts reading the track with `hash` from `path`, keeping what it
    /// decodes in `cache`, and calling `wake` when it is done so that the
    /// window repaints and picks the audio up.
    pub fn start(
        hash: ContentHash,
        path: PathBuf,
        cache: Arc<Mutex<AudioCache>>,
        wake: impl Fn() + Send + 'static,
    ) -> Decoding {
        let (send, done) = channel();
        let reading = path.clone();
        std::thread::spawn(move || {
            let read = audio_of(&cache, hash, &reading)
                .map_err(|problem| format!("{} could not be read: {problem}", reading.display()));
            // A send that fails means the editor has closed, which ends the
            // reading rather than being anything to report.
            let _ = send.send(read);
            wake();
        });
        Decoding { path, done }
    }

    /// The audio, or the reason the file could not be read, once the thread
    /// has finished; nothing while it is still reading.
    ///
    /// A thread that stopped without sending anything, which is a thread
    /// that panicked, is a reading that failed like any other: the strip
    /// would otherwise wait for a waveform that is never coming, with
    /// nothing said about why.
    pub fn finished(&self) -> Option<Result<Arc<Audio>, String>> {
        match self.done.try_recv() {
            Ok(read) => Some(read),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(format!(
                "{} could not be read: the thread reading it stopped",
                self.path.display()
            ))),
        }
    }
}

/// The loader the preview is started with.
///
/// It hands over audio the window already holds and decodes a track it does
/// not, keeping what it decoded, so that moving the playhead or handing the
/// transport an edited document costs no second read of a file that is
/// already in hand.
pub fn loader(cache: Arc<Mutex<AudioCache>>) -> SendLoader {
    Box::new(move |_position: usize, track: &Track| {
        let audio = audio_of(&cache, track.hash, &track.path)?;
        Ok(Box::new(SharedAudio(audio)) as Box<dyn Source>)
    })
}

/// A stretcher for a track whose keylock is on, which keeps the track's pitch
/// where its speed changes.
#[cfg(feature = "signalsmith")]
fn keylock_stretcher() -> Box<dyn TimeStretcher> {
    Box::new(dermixen_engine::SignalsmithStretcher::new())
}

/// A stretcher for a track whose keylock is on, in a build with no
/// pitch-preserving stretcher, where plain resampling stands in and the
/// pitch moves with the speed.
#[cfg(not(feature = "signalsmith"))]
fn keylock_stretcher() -> Box<dyn TimeStretcher> {
    Box::new(Resampler::new())
}

/// Chooses each track's time stretcher, as `render` and `play` choose it: the
/// pitch-preserving one for a track with keylock, and plain resampling for a
/// track without it.
pub fn stretchers() -> SendStretchers {
    Box::new(|track: &Track| {
        if track.keylock {
            keylock_stretcher()
        } else {
            Box::new(Resampler::new())
        }
    })
}

/// What the status line says about the stretcher in a build that has the
/// pitch-preserving one, which is nothing.
#[cfg(feature = "signalsmith")]
pub fn keylock_note() -> Option<String> {
    None
}

/// What the status line says in a build with no pitch-preserving stretcher,
/// so that a person hearing a track with keylock knows why its pitch moves.
#[cfg(not(feature = "signalsmith"))]
pub fn keylock_note() -> Option<String> {
    Some(
        "This build has no pitch-preserving stretcher, so tracks with keylock are resampled and \
         their pitch moves with their speed"
            .to_owned(),
    )
}

/// Opens the machine's default audio output device for the preview and
/// the audition. `buffer_frames` is the `audio_buffer_frames` setting: how
/// many frames the device takes per pull, or `None` for the device's own
/// size.
#[cfg(feature = "playback")]
pub fn output(buffer_frames: Option<NonZeroU32>) -> Result<Box<dyn Output>, String> {
    Ok(Box::new(dermixen_engine::CpalOutput::open(buffer_frames)?))
}

/// Refuses to play, in a build made without the `playback` feature and so
/// with no way to reach an audio device.
#[cfg(not(feature = "playback"))]
pub fn output(_buffer_frames: Option<NonZeroU32>) -> Result<Box<dyn Output>, String> {
    Err(
        "This window was built without the playback feature, so it cannot open an audio device"
            .to_owned(),
    )
}

/// Something the reading thread finished and the window paints or reports.
pub enum Finding {
    /// A track's waveform overview, which the window hands the timeline.
    Overview {
        /// The track's content hash.
        hash: ContentHash,
        /// Its waveform overview.
        overview: Overview,
    },
    /// A track's phrase analysis as the library has it.
    Phrases {
        /// The track's content hash.
        hash: ContentHash,
        /// Its phrase analysis.
        phrases: PhraseRecord,
    },
    /// Something the person should read: a file that could not be decoded, or
    /// a library file that could not be opened.
    Trouble(
        /// What went wrong, in words.
        String,
    ),
}

/// One track for the reading thread to work on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    /// The track's content hash, which the timeline keys everything by.
    pub hash: ContentHash,
    /// Where its audio file is.
    pub path: PathBuf,
}

/// The thread that reads the mix's files while the window keeps repainting.
///
/// It looks each track up in the library for its phrase analysis, then
/// decodes each track and works out its waveform overview, in playlist order,
/// and sends each finding back as it is made. Once it has worked through the
/// tracks it was started with, it waits for tracks [`Reading::want`] sends it
/// and reads each of those the same way, in the order they were asked for.
/// This is how a track added from the library, or a track of a restored
/// document that the window has not read, gets its waveform after the window
/// has already been open a while. The window drains the findings on every
/// repaint, so a lane whose overview has not arrived yet is simply drawn
/// without one. When the `Reading` is dropped, the thread finishes the decode
/// it is in and then ends.
pub struct Reading {
    /// The findings the thread has sent so far.
    findings: Receiver<Finding>,
    /// Where a track wanted after the thread started is sent. Dropping this
    /// alongside the `Reading` is what ends the reading thread, once the
    /// thread has nothing left queued to read.
    wants: Sender<Wanted>,
}

impl Reading {
    /// Starts the thread over `wanted`, reading phrases from the library file
    /// at `index` when there is one, keeping the audio it decodes in `cache`
    /// while there is room, and calling `wake` after each finding so that the
    /// window repaints and picks it up. The library file, when there is one,
    /// is opened once and kept open for the thread's whole life, so a track
    /// wanted later costs no second attempt to open it.
    pub fn start(
        wanted: Vec<Wanted>,
        index: Option<PathBuf>,
        cache: Arc<Mutex<AudioCache>>,
        wake: impl Fn() + Send + 'static,
    ) -> Reading {
        let (send, findings) = channel();
        let (want_send, want_recv) = channel();
        std::thread::spawn(move || {
            // A send that fails means the window has closed, which ends the
            // reading rather than being anything to report.
            let tell = |finding: Finding| send.send(finding).is_ok();
            let index = open_index(index.as_deref(), &tell, &wake);

            // Every track the thread was started with gets its phrases
            // looked up first, since a lookup needs no decoding and so
            // should not wait behind one, and only then is any of them
            // decoded.
            if let Some(index) = &index {
                for track in &wanted {
                    if let Some(finding) = phrases_for(index, track) {
                        if !tell(finding) {
                            return;
                        }
                        wake();
                    }
                }
            }
            for track in &wanted {
                if !tell(decode_finding(track, &cache)) {
                    return;
                }
                wake();
            }

            // Tracks wanted after the thread started, taken in the order
            // they were asked for. The loop, and the thread, end once the
            // `Reading` is dropped and there is nothing left to receive.
            while let Ok(track) = want_recv.recv() {
                if let Some(index) = &index
                    && let Some(finding) = phrases_for(index, &track)
                {
                    if !tell(finding) {
                        return;
                    }
                    wake();
                }
                if !tell(decode_finding(&track, &cache)) {
                    return;
                }
                wake();
            }
        });
        Reading {
            findings,
            wants: want_send,
        }
    }

    /// Asks the thread to read `track` after everything it was started with
    /// and everything wanted before it, reporting its phrases and its
    /// overview as it does for any other track. This is how a track added
    /// from the library, or one a restored document has that the window has
    /// not read, gets its waveform.
    pub fn want(&self, track: Wanted) {
        // A send that fails means the thread has stopped, and there is
        // nothing to report through this call.
        let _ = self.wants.send(track);
    }

    /// Everything the thread has finished since this was last asked.
    pub fn findings(&self) -> TryIter<'_, Finding> {
        self.findings.try_iter()
    }
}

/// Opens the library file at `path`, when there is one, reporting trouble
/// once through `tell` when the file cannot be opened, rather than once for
/// every track a phrase lookup is later tried for.
fn open_index(
    path: Option<&Path>,
    tell: &impl Fn(Finding) -> bool,
    wake: &impl Fn(),
) -> Option<Index> {
    let path = path?;
    match Index::open(path) {
        Ok(index) => Some(index),
        Err(problem) => {
            if tell(Finding::Trouble(format!(
                "No phrase marks are shown: {problem}"
            ))) {
                wake();
            }
            None
        }
    }
}

/// The phrase analysis of one track, from the library, when there is one to
/// report.
///
/// A track the library does not contain, and a track whose record has no
/// phrase analysis, yields nothing: the timeline then counts that track's
/// bars from beat zero.
fn phrases_for(index: &Index, track: &Wanted) -> Option<Finding> {
    match index.get(track.hash) {
        Ok(Some(record)) => record.phrases.map(|phrases| Finding::Phrases {
            hash: track.hash,
            phrases,
        }),
        Ok(None) => None,
        Err(problem) => Some(Finding::Trouble(format!(
            "No phrase marks are shown for {}: {problem}",
            track.path.display()
        ))),
    }
}

/// Decodes one track, working out its waveform overview and keeping its
/// audio in `cache` as a read-ahead, or reporting the file by path when it
/// cannot be read.
fn decode_finding(track: &Wanted, cache: &Mutex<AudioCache>) -> Finding {
    match decode(&track.path) {
        Ok(decoded) => {
            let overview = Overview::of(&decoded.audio, OVERVIEW_BUCKET);
            shared(cache).read_ahead(track.hash, Arc::new(decoded.audio));
            Finding::Overview {
                hash: track.hash,
                overview,
            }
        }
        Err(problem) => Finding::Trouble(format!(
            "{} could not be read: {problem}",
            track.path.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hash whose first byte is `byte`, to tell tracks apart in the tests.
    fn hash(byte: u8) -> ContentHash {
        let mut bytes = [0u8; 32];
        bytes[0] = byte;
        ContentHash(bytes)
    }

    /// Some audio to keep, whose contents do not matter here.
    fn audio() -> Arc<Audio> {
        Arc::new(Audio {
            frames: vec![[0.0, 0.0]; 8],
        })
    }

    #[test]
    fn the_track_used_longest_ago_is_let_go_first() {
        let mut cache = AudioCache::default();
        for byte in 0..=TRACKS_KEPT as u8 {
            cache.keep(hash(byte), audio());
        }
        assert!(
            cache.heard(hash(0)).is_none(),
            "the first track kept should have been let go"
        );
        assert!(
            cache.heard(hash(TRACKS_KEPT as u8)).is_some(),
            "the track kept last should still be there"
        );
    }

    #[test]
    fn hearing_a_track_keeps_it_from_being_let_go() {
        let mut cache = AudioCache::default();
        for byte in 0..TRACKS_KEPT as u8 {
            cache.keep(hash(byte), audio());
        }
        assert!(cache.heard(hash(0)).is_some());
        cache.keep(hash(9), audio());
        assert!(
            cache.heard(hash(0)).is_some(),
            "the track heard again should have outlived the one after it"
        );
        assert!(
            cache.heard(hash(1)).is_none(),
            "the track used longest ago should have been let go"
        );
    }

    #[test]
    fn a_full_store_has_no_room_for_a_track_read_ahead() {
        let mut cache = AudioCache::default();
        for byte in 0..TRACKS_KEPT as u8 {
            cache.keep(hash(byte), audio());
        }
        cache.read_ahead(hash(9), audio());
        assert!(
            cache.heard(hash(9)).is_none(),
            "a full store should have no room for a track read ahead"
        );
        for byte in 0..TRACKS_KEPT as u8 {
            assert!(cache.heard(hash(byte)).is_some());
        }
    }

    #[test]
    fn a_track_read_ahead_is_let_go_before_a_track_the_render_asked_for() {
        let mut cache = AudioCache::default();
        // The oldest entry of all is one the render asked for, and every
        // entry after it was only read ahead, so letting go by age alone
        // would take the one the render asked for.
        cache.keep(hash(0), audio());
        for byte in 1..TRACKS_KEPT as u8 {
            cache.read_ahead(hash(byte), audio());
        }
        cache.keep(hash(9), audio());
        assert!(
            cache.heard(hash(0)).is_some(),
            "the track the render asked for should have outlived the read-ahead ones"
        );
        assert!(
            cache.heard(hash(TRACKS_KEPT as u8 - 1)).is_none(),
            "the last track read ahead should have been let go"
        );
    }

    #[test]
    fn a_track_read_ahead_that_the_render_asks_for_is_kept_like_any_other() {
        let mut cache = AudioCache::default();
        for byte in 0..TRACKS_KEPT as u8 {
            cache.read_ahead(hash(byte), audio());
        }
        // The render asks for the track read first, which makes it a track
        // the render has heard and moves it out of the way of the next
        // letting go.
        assert!(cache.heard(hash(0)).is_some());
        cache.keep(hash(9), audio());
        assert!(
            cache.heard(hash(0)).is_some(),
            "a track the render asked for should not be let go for a read-ahead one"
        );
        assert!(
            cache.heard(hash(TRACKS_KEPT as u8 - 1)).is_none(),
            "the last track still only read ahead should have been let go"
        );
    }
}
