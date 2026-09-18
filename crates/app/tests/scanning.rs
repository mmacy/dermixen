//! Acceptance tests for the thread that scans the music folder into the
//! library: every file is reported as it is dealt with, the library
//! contains the tracks when the scan is over, a stop takes effect after the
//! file the scan is on, and a folder or a library that cannot be used is
//! reported rather than left silent. A coder agent makes these pass without
//! editing them.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use dermixen_app::{Report, Scan};
use dermixen_core::{Bpm, Seconds};
use dermixen_library::{Change, Index};
use dermixen_media::{WavDepth, write_wav};
use dermixen_testkit::synth;

/// How long a test waits for the thread. Analyzing a twenty-second kicks
/// file takes seconds, so a wait this long is a thread that is not
/// answering.
const PATIENCE: Duration = Duration::from_secs(120);

/// Writes twenty seconds of kicks at `bpm` under `dir`.
fn kicks_file(dir: &Path, relative: &str, bpm: f64) -> PathBuf {
    let path = dir.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let audio = synth::kicks(Bpm(bpm), Seconds(0.5), Seconds(20.0));
    write_wav(&path, &audio, WavDepth::Int16).unwrap();
    path
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

/// Collects the thread's reports until one satisfies `done`, and answers
/// everything collected in the order it arrived. A wait that outlasts
/// [`PATIENCE`] fails the test.
fn collect_until(scan: &Scan, mut done: impl FnMut(&Report) -> bool) -> Vec<Report> {
    let deadline = Instant::now() + PATIENCE;
    let mut found = Vec::new();
    loop {
        for report in scan.reports() {
            let last = done(&report);
            found.push(report);
            if last {
                return found;
            }
        }
        assert!(
            Instant::now() < deadline,
            "the scan thread did not answer in {PATIENCE:?}; reports so far: {found:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Everything the scan reports up to and including its finish.
fn collect_all(scan: &Scan) -> Vec<Report> {
    collect_until(scan, |report| matches!(report, Report::Finished(_)))
}

#[test]
fn every_file_is_reported_and_the_library_contains_the_tracks_when_the_scan_is_over() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    let alpha = kicks_file(&music, "Etnica - Alpha.wav", 130.0);
    let beta = kicks_file(&music, "Prana - Beta.wav", 140.0);
    let library = dir.path().join("data/dermixen/library.sqlite");
    let (wake, woken) = counting_wake();

    let scan = Scan::start(&library, &music, wake);
    let reports = collect_all(&scan);
    assert_eq!(
        reports.len(),
        3,
        "one report per file, then the finish: {reports:?}"
    );
    assert_eq!(
        reports[..2],
        [
            Report::File {
                done: 1,
                total: 2,
                path: alpha.clone(),
                change: Change::Added,
            },
            Report::File {
                done: 2,
                total: 2,
                path: beta.clone(),
                change: Change::Added,
            },
        ],
        "the files are reported in the order they were found"
    );
    match &reports[2] {
        Report::Finished(Ok(summary)) => {
            assert_eq!(summary.added, 2, "{summary:?}");
            assert_eq!(summary.moved, 0, "{summary:?}");
            assert_eq!(summary.unchanged, 0, "{summary:?}");
            assert_eq!(summary.completed, 0, "{summary:?}");
            assert!(summary.duplicates.is_empty(), "{summary:?}");
            assert!(summary.failed.is_empty(), "{summary:?}");
            assert!(summary.unreadable.is_empty(), "{summary:?}");
            assert!(!summary.stopped, "{summary:?}");
        }
        other => panic!("the last report is the summary, not {other:?}"),
    }
    assert!(
        woken.load(Ordering::SeqCst) >= 3,
        "the wake was called for every report, not {} times",
        woken.load(Ordering::SeqCst)
    );

    let deadline = Instant::now() + PATIENCE;
    while scan.is_running() {
        assert!(Instant::now() < deadline, "the thread did not return");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        library.exists(),
        "the library file and the folders above it were made"
    );
    let index = Index::open(&library).unwrap();
    assert_eq!(index.len().unwrap(), 2);
    let paths: Vec<PathBuf> = index
        .query(&dermixen_library::Query::default())
        .unwrap()
        .into_iter()
        .map(|record| record.path)
        .collect();
    assert_eq!(paths, vec![alpha, beta]);
}

#[test]
fn a_second_scan_of_the_same_folder_reports_every_file_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    kicks_file(&music, "Etnica - Alpha.wav", 130.0);
    let library = dir.path().join("library.sqlite");

    let first = Scan::start(&library, &music, || {});
    collect_all(&first);
    let second = Scan::start(&library, &music, || {});
    let reports = collect_all(&second);
    assert!(
        matches!(
            &reports[0],
            Report::File {
                done: 1,
                total: 1,
                change: Change::Unchanged,
                ..
            }
        ),
        "{reports:?}"
    );
    match &reports[1] {
        Report::Finished(Ok(summary)) => assert_eq!(summary.unchanged, 1, "{summary:?}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_stop_takes_effect_after_the_file_the_scan_is_on() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    for (name, bpm) in [
        ("01.wav", 130.0),
        ("02.wav", 132.0),
        ("03.wav", 134.0),
        ("04.wav", 136.0),
        ("05.wav", 138.0),
        ("06.wav", 140.0),
    ] {
        kicks_file(&music, name, bpm);
    }
    let library = dir.path().join("library.sqlite");

    let scan = Scan::start(&library, &music, || {});
    let mut before = collect_until(&scan, |report| matches!(report, Report::File { .. }));
    // Whatever the thread sent between that first report and this moment
    // was sent before the stop, so it is drained now and counted as seen.
    before.extend(scan.reports());
    scan.stop();
    scan.stop();
    let seen = before
        .iter()
        .filter(|report| matches!(report, Report::File { .. }))
        .count();
    let rest = collect_all(&scan);
    let files_after_the_stop = rest
        .iter()
        .filter(|report| matches!(report, Report::File { .. }))
        .count();
    assert!(
        files_after_the_stop <= 1,
        "at most the file the scan was on is reported after a stop, not {files_after_the_stop}: {rest:?}"
    );
    match rest.last().unwrap() {
        Report::Finished(Ok(summary)) => {
            assert!(summary.stopped, "{summary:?}");
            assert_eq!(summary.added, seen + files_after_the_stop, "{summary:?}");
        }
        other => panic!("{other:?}"),
    }
    let index = Index::open(&library).unwrap();
    assert_eq!(
        index.len().unwrap(),
        seen + files_after_the_stop,
        "the files after the stop were not looked at"
    );
    scan.stop();
}

#[test]
fn a_folder_that_is_not_there_is_reported_with_its_path() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("Music/Undefunktis");
    let library = dir.path().join("library.sqlite");
    let (wake, woken) = counting_wake();
    let scan = Scan::start(&library, &missing, wake);
    let reports = collect_all(&scan);
    assert_eq!(reports.len(), 1, "{reports:?}");
    match &reports[0] {
        Report::Finished(Err(reason)) => assert!(
            reason.contains(&missing.display().to_string()),
            "the reason names the folder: {reason}"
        ),
        other => panic!("{other:?}"),
    }
    assert!(woken.load(Ordering::SeqCst) >= 1);
}

#[test]
fn a_library_file_that_cannot_be_made_is_reported_with_its_path() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    kicks_file(&music, "Etnica - Alpha.wav", 130.0);
    // A library path below a file rather than a folder cannot be made.
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, "not a folder").unwrap();
    let library = blocker.join("library.sqlite");
    let scan = Scan::start(&library, &music, || {});
    let reports = collect_all(&scan);
    assert_eq!(reports.len(), 1, "{reports:?}");
    match &reports[0] {
        Report::Finished(Err(reason)) => assert!(
            reason.contains(&library.display().to_string()),
            "the reason names the library file: {reason}"
        ),
        other => panic!("{other:?}"),
    }
}
