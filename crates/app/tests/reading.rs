//! Acceptance tests for the thread that reads the mix's files. A track
//! wanted after the thread started, which is what adding a track from the
//! library does, is read like a track the thread started with: its phrases
//! come from the index, its waveform overview is worked out, and its audio is
//! kept for the render. A coder agent makes these pass without editing them.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dermixen_analysis::Extent;
use dermixen_app::audio::{AudioCache, Finding, Reading, Wanted, kept};
use dermixen_core::{Anchors, BeatGrid, Beats, Bpm, ContentHash, Samples};
use dermixen_library::{
    Index, Metadata, MetadataSource, PhraseRecord, PhraseStartRecord, Release, TrackRecord,
};

/// How many frames the two-second fixtures hold at 44.1 kHz.
const FIXTURE_FRAMES: Samples = Samples(2 * 44_100);

/// How long a test waits for the thread. The fixtures decode in well under a
/// second, so a wait this long is a thread that is not answering.
const PATIENCE: Duration = Duration::from_secs(30);

/// Where the synthetic audio fixtures are.
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/audio")
        .join(name)
}

/// An empty store for the audio the thread decodes.
fn cache() -> Arc<Mutex<AudioCache>> {
    Arc::new(Mutex::new(AudioCache::default()))
}

/// A wake that counts how many times it was called, and the counter.
fn counting_wake() -> (impl Fn() + Send + 'static, Arc<AtomicUsize>) {
    let woken = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&woken);
    let wake = move || {
        counter.fetch_add(1, Ordering::SeqCst);
    };
    (wake, woken)
}

/// Collects the thread's findings until one satisfies `done`, and answers
/// everything collected, in the order it arrived. A wait that outlasts
/// [`PATIENCE`] fails the test.
fn collect_until(reading: &Reading, mut done: impl FnMut(&Finding) -> bool) -> Vec<Finding> {
    let deadline = Instant::now() + PATIENCE;
    let mut found = Vec::new();
    loop {
        for finding in reading.findings() {
            let last = done(&finding);
            found.push(finding);
            if last {
                return found;
            }
        }
        assert!(
            Instant::now() < deadline,
            "the reading thread sent nothing that was waited for within {PATIENCE:?}; it sent {} \
             findings",
            found.len()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Whether `finding` is the overview of the track with `wanted`.
fn is_overview_of(finding: &Finding, wanted: ContentHash) -> bool {
    matches!(finding, Finding::Overview { hash, .. } if *hash == wanted)
}

/// The hashes of the overviews among `found`, in order.
fn overviews_among(found: &[Finding]) -> Vec<ContentHash> {
    found
        .iter()
        .filter_map(|finding| match finding {
            Finding::Overview { hash, .. } => Some(*hash),
            _ => None,
        })
        .collect()
}

/// An index record for the fixture at `path` with the given phrases.
fn record(hash: ContentHash, path: &Path, phrases: PhraseRecord) -> TrackRecord {
    TrackRecord {
        hash,
        path: path.to_path_buf(),
        length: FIXTURE_FRAMES,
        grid: BeatGrid {
            first_beat: Samples(0),
            bpm: Bpm(140.0),
        },
        grid_confidence: 0.9,
        grid_analyzer: "pulse".to_owned(),
        key: None,
        extent: Extent {
            begins: Samples(0),
            ends: FIXTURE_FRAMES,
        },
        anchors: Anchors {
            intro: Beats(0.0),
            outro: Beats(4.0),
        },
        anchor_confidence: 0.7,
        anchor_analyzer: "kick".to_owned(),
        metadata: Metadata {
            artist: None,
            title: None,
            year: None,
            year_is_approximate: false,
            source: MetadataSource::Tags,
        },
        phrases: Some(phrases),
        loudness: None,
        release: Release::default(),
    }
}

#[test]
fn a_track_wanted_after_the_thread_started_gets_its_overview_and_is_read_ahead() {
    let cache = cache();
    let (wake, woken) = counting_wake();
    let reading = Reading::start(Vec::new(), None, Arc::clone(&cache), wake);
    let hash = ContentHash([1; 32]);
    reading.want(Wanted {
        hash,
        path: fixture("sine-440-44k.wav"),
    });

    let found = collect_until(&reading, |finding| is_overview_of(finding, hash));
    assert_eq!(
        found.len(),
        1,
        "the overview, and nothing else, was reported"
    );
    let Some(Finding::Overview { overview, .. }) = found.last() else {
        unreachable!("the last finding is the overview waited for");
    };
    assert_eq!(overview.length(), FIXTURE_FRAMES);
    let peak = overview.peak_over(Samples(0)..FIXTURE_FRAMES);
    assert!(
        peak.high > 0.2 && peak.low < -0.2,
        "the overview is of the fixture's tone, not of silence: {peak:?}"
    );
    let audio = kept(&cache, hash).expect("the audio is kept for the render");
    assert_eq!(audio.len(), FIXTURE_FRAMES);
    assert!(
        woken.load(Ordering::SeqCst) >= 1,
        "the window is woken so that it repaints and picks the overview up"
    );
}

#[test]
fn the_tracks_the_thread_started_with_are_read_before_a_track_wanted_later() {
    let first = ContentHash([2; 32]);
    let second = ContentHash([3; 32]);
    let reading = Reading::start(
        vec![Wanted {
            hash: first,
            path: fixture("sine-440-44k.mp3"),
        }],
        None,
        cache(),
        || {},
    );
    reading.want(Wanted {
        hash: second,
        path: fixture("sine-220-mono-44k.wav"),
    });

    let found = collect_until(&reading, |finding| is_overview_of(finding, second));
    assert_eq!(overviews_among(&found), vec![first, second]);
    assert_eq!(
        found.len(),
        2,
        "two overviews, and nothing else, were reported"
    );
}

#[test]
fn a_file_that_cannot_be_read_is_reported_by_path_and_the_next_wanted_track_is_still_read() {
    let cache = cache();
    let reading = Reading::start(Vec::new(), None, Arc::clone(&cache), || {});
    let missing = fixture("not-there.wav");
    let bad = ContentHash([4; 32]);
    let good = ContentHash([5; 32]);
    reading.want(Wanted {
        hash: bad,
        path: missing.clone(),
    });
    reading.want(Wanted {
        hash: good,
        path: fixture("sine-440-44k.wav"),
    });

    let found = collect_until(&reading, |finding| is_overview_of(finding, good));
    assert_eq!(found.len(), 2, "the trouble and then the overview");
    let Finding::Trouble(problem) = &found[0] else {
        panic!("the file that could not be read is reported before the next track is read");
    };
    assert!(
        problem.contains(&missing.display().to_string()),
        "the report names the file: {problem}"
    );
    assert!(
        kept(&cache, bad).is_none(),
        "nothing is kept for a file that could not be read"
    );
}

#[test]
fn a_track_wanted_later_gets_its_phrases_from_the_index_before_its_overview() {
    let dir = tempfile::tempdir().unwrap();
    let index_path = dir.path().join("library.sqlite");
    let hash = ContentHash([6; 32]);
    let path = fixture("sine-440-44k.wav");
    let phrases = PhraseRecord {
        analyzer: "shifts".to_owned(),
        confidence: 0.5,
        downbeat: 2,
        starts: vec![PhraseStartRecord {
            beat: Beats(2.0),
            bars: 8,
        }],
        sections: vec![Beats(66.0)],
    };
    {
        let mut index = Index::open(&index_path).unwrap();
        index.upsert(&record(hash, &path, phrases.clone())).unwrap();
    }
    let reading = Reading::start(Vec::new(), Some(index_path), cache(), || {});
    reading.want(Wanted { hash, path });

    let found = collect_until(&reading, |finding| is_overview_of(finding, hash));
    assert_eq!(
        found.len(),
        2,
        "the phrases and the overview, and nothing else"
    );
    let Finding::Phrases {
        hash: of,
        phrases: read,
    } = &found[0]
    else {
        panic!("the phrases come first, since they need no decoding");
    };
    assert_eq!(*of, hash);
    assert_eq!(*read, phrases);
}

#[test]
fn the_thread_ends_when_the_reading_is_dropped() {
    let (wake, woken) = counting_wake();
    let reading = Reading::start(Vec::new(), None, cache(), wake);
    // Give the thread time to finish what it was started with, so that it
    // is waiting for a request, not still starting, when the reading goes.
    std::thread::sleep(Duration::from_millis(100));
    drop(reading);

    // The thread owns the wake closure, so the counter has one other owner
    // for as long as the thread lives.
    let deadline = Instant::now() + PATIENCE;
    while Arc::strong_count(&woken) > 1 {
        assert!(
            Instant::now() < deadline,
            "the thread is still running with nothing left to read"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
