//! The thread that scans the music folder into the library while the
//! window stays responsive.
//!
//! A first library is built this way when the window opens and there is
//! no library file yet, and **Library > Scan music folder** runs the same
//! scan whenever the person asks. The scan is the library crate's own, the
//! one `dermixen library scan` runs, so the window and the command build
//! the same records. The window reads the reports on every repaint, puts
//! the progress on the status line, reads the library again as tracks land,
//! and can stop the scan after the file it is on.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryIter};
use std::thread::JoinHandle;

use dermixen_library::{AnalyzerSet, Change, Index, Progress, ScanOptions, ScanSummary, scan_into};

/// One thing the scan has to report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Report {
    /// One file was dealt with.
    File {
        /// How many files have been dealt with, including this one.
        done: usize,
        /// How many files the scan found under the folder.
        total: usize,
        /// The file.
        path: PathBuf,
        /// What was done with it.
        change: Change,
    },
    /// The scan is over: what it did, or why it could not run at all. A
    /// folder that is not there, a folder that cannot be listed, and a
    /// library file that cannot be opened are each an `Err` naming the
    /// path and giving the reason.
    Finished(Result<ScanSummary, String>),
}

/// Opens the library file at `path`, making the file and the folders above
/// it when they are not there yet, so that a first scan needs no setup. A
/// failure names the library file, because that is the path a person set or
/// left at its default.
///
/// A scan opens the library this way before it lists the folder. The window
/// calls this as it opens, and again for every mix it puts under itself, to
/// make the library file the settings name, and then drops the index it gets
/// back, since the window opens the library again for each read. Ask
/// [`dermixen_library::Index::open`] instead where the file must already be
/// there, as a command that writes nothing does.
pub fn open_the_library(path: &Path) -> Result<Index, String> {
    if let Some(folder) = path.parent()
        && !folder.as_os_str().is_empty()
        && !folder.exists()
        && let Err(problem) = std::fs::create_dir_all(folder)
    {
        return Err(format!(
            "cannot make the library file {}: {problem}",
            path.display()
        ));
    }
    Index::open(path).map_err(|problem| problem.to_string())
}

/// A scan of one folder into one library file, running on its own thread.
#[derive(Debug)]
pub struct Scan {
    reports: Receiver<Report>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Scan {
    /// Starts scanning `folder` into the library file at `library`, which
    /// is made, with the folders above it, when it is not there yet. Every
    /// audio file under `folder` is dealt with as `docs/library.md`
    /// describes under "Scanning", with the built-in analyzers. The library
    /// file is opened before the folder is listed and before any file is
    /// dealt with, so a library file that cannot be opened is reported as
    /// the only report. `wake` is called after every report, so a window
    /// can repaint when one arrives.
    ///
    /// Keep what this gives back and read [`reports`](Scan::reports) on
    /// every repaint, which is where the progress and the summary come from.
    /// [`stop`](Scan::stop) ends a scan early, and
    /// [`is_running`](Scan::is_running) says whether the thread is still
    /// going. Dropping the scan leaves the thread running to the end, with
    /// nobody to hear what it reports, so a window that wants the scan over
    /// asks it to stop. Nothing here reads a library: the caller reads the
    /// library again as reports arrive, which is how a panel fills in while
    /// a scan runs.
    pub fn start(library: &Path, folder: &Path, wake: impl Fn() + Send + 'static) -> Scan {
        let (sender, reports) = std::sync::mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let asked_to_stop = Arc::clone(&stop);
        let library = library.to_path_buf();
        let folder = folder.to_path_buf();
        let thread = std::thread::spawn(move || {
            // A window that has gone away leaves nobody to receive the
            // report, which is not the scan's business, so a send that
            // fails is passed over and the scan goes on.
            let send = |report: Report| {
                let _ = sender.send(report);
                wake();
            };
            let mut index = match open_the_library(&library) {
                Ok(index) => index,
                Err(problem) => {
                    send(Report::Finished(Err(problem)));
                    return;
                }
            };
            // The analyzers hold the key detector this build has, which is
            // not one a thread may take from another, so this thread builds
            // its own set.
            let analyzers = AnalyzerSet::built_in();
            let mut report = |progress: &Progress<'_>| {
                send(Report::File {
                    done: progress.done,
                    total: progress.total,
                    path: progress.path.to_path_buf(),
                    change: progress.change,
                });
                !asked_to_stop.load(Ordering::SeqCst)
            };
            let outcome = scan_into(
                &mut index,
                &folder,
                &ScanOptions::default(),
                &analyzers.as_analyzers(),
                &mut report,
            )
            .map_err(|problem| problem.to_string());
            send(Report::Finished(outcome));
        });
        Scan {
            reports,
            stop,
            thread: Some(thread),
        }
    }

    /// Asks the scan to stop. It finishes the file it is on, reports that
    /// file, and then reports [`Report::Finished`] with the summary's
    /// `stopped` set. Asking a scan that is over to stop does nothing.
    pub fn stop(&self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// The reports that have arrived since the last look, in order. The
    /// last report is always [`Report::Finished`], after which no more
    /// come.
    pub fn reports(&self) -> TryIter<'_, Report> {
        self.reports.try_iter()
    }

    /// Whether the thread is still running, which is until it has sent
    /// [`Report::Finished`] and returned.
    pub fn is_running(&self) -> bool {
        self.thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
    }
}
