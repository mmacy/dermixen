#![allow(unsafe_code)]

//! The documents macOS asks the app to open, handed to the window as paths.
//!
//! When a person double-clicks a `.dmx` file in the Finder, or drops one on
//! the Dermixen icon, macOS does not pass the file on the command line. It
//! starts the application bundle with no arguments and sends the running
//! process an Apple event, the open-documents event, naming the files. The
//! window toolkit Dermixen is built on does not receive that event, so this
//! crate does: it registers a handler for the event with the Objective-C
//! runtime and sends every path the event names down a channel the window
//! reads on each repaint. On every platform but macOS the function returns a
//! channel that nothing ever sends on, so the window's code is the same
//! everywhere.
//!
//! The crates that make up the app itself forbid unsafe code, so the calls
//! into the Objective-C runtime sit here, in a file short enough to read in
//! one sitting. The other three crates that allow unsafe code are the
//! wrappers around the time-stretcher, the beat tracker, and the key
//! detector.

use std::path::PathBuf;
use std::sync::mpsc::Receiver;

/// Starts listening for the documents macOS asks the app to open, and
/// answers the channel their paths arrive on, in the order the events name
/// them. `wake` is called after each path is sent, so a window can repaint
/// and read the channel.
///
/// Call this once, before the event loop starts, so that a document the
/// app was started to open is received: macOS delivers that first event
/// while the application is finishing its launch, before the window exists.
/// A later event, for a file opened while the window is up, arrives on the
/// same channel. On every platform but macOS nothing is listened for and
/// the channel never receives.
///
/// Call this on the thread the program started on, which is the thread the
/// application dispatches the event on. Called on any other thread it
/// listens for nothing and answers a channel that never receives, because
/// the handler it would register belongs to the main thread. The caller
/// cannot tell that call from a call on a platform that is not macOS: both
/// answer the same kind of channel, and neither reports anything.
pub fn watch_opened_documents(wake: Box<dyn Fn() + Send + Sync>) -> Receiver<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        apple_events::watch(wake)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = wake;
        let (sender, receiver) = std::sync::mpsc::channel();
        // Nothing on this platform sends a path, and a file opened on the
        // desktop reaches the window as its argument instead.
        drop(sender);
        receiver
    }
}

#[cfg(target_os = "macos")]
mod apple_events {
    use std::path::PathBuf;
    use std::ptr::NonNull;
    use std::sync::mpsc::{self, Receiver, Sender};

    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::NSObject;
    use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
    use objc2_app_kit::NSApplicationWillFinishLaunchingNotification;
    use objc2_foundation::{
        NSAppleEventDescriptor, NSAppleEventManager, NSNotification, NSNotificationCenter,
    };

    /// An Apple event is named by four characters, which the Objective-C
    /// runtime takes as the number those four bytes spell.
    const fn four_characters(code: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*code)
    }

    /// The class of the Apple events the system sends an application about
    /// its own running, written `aevt`. The open-documents event is one of
    /// them, and so are the events for a launch, a print, and a quit.
    const SYSTEM_EVENTS: u32 = four_characters(b"aevt");

    /// The Apple event that names documents for the application to open,
    /// written `odoc`.
    const OPEN_DOCUMENTS: u32 = four_characters(b"odoc");

    /// The keyword of the parameter that holds what an Apple event is
    /// about, written `----`. For the open-documents event that parameter
    /// is the list of files.
    const DIRECT_PARAMETER: u32 = four_characters(b"----");

    /// Where the handler for the open-documents event puts what it reads.
    struct Opened {
        /// The channel the window reads the paths from.
        paths: Sender<PathBuf>,
        /// What repaints the window, called after each path is sent.
        wake: Box<dyn Fn() + Send + Sync>,
    }

    define_class!(
        // SAFETY:
        // - NSObject, the superclass, places no requirement on a subclass.
        // - This class implements no `Drop`.
        // - The class is main-thread only, which is where the application
        //   builds it and where the application dispatches the event to it.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "DermixenOpenedDocuments"]
        #[ivars = Opened]
        struct OpenedDocuments;

        impl OpenedDocuments {
            /// Sends every path the open-documents event names down the
            /// channel, and wakes the window after each one.
            ///
            /// The Apple Event Manager calls this when the system asks the
            /// application to open documents, which is the launch a
            /// double-click starts and every later double-click while the
            /// application runs. A window that has gone leaves the channel
            /// with no receiver, and the rest of the paths are dropped.
            ///
            /// An event this reads no path out of leaves one line on
            /// standard error, because the application comes forward on
            /// such an event with no mix and nothing said about why.
            #[unsafe(method(openDocuments:withReplyEvent:))]
            fn open_documents(
                &self,
                event: &NSAppleEventDescriptor,
                _reply: *mut NSAppleEventDescriptor,
            ) {
                let paths = paths_in(event);
                if paths.is_empty() {
                    eprintln!(
                        "macOS asked Dermixen to open a document it could not read as a file path"
                    );
                    return;
                }
                for path in paths {
                    if self.ivars().paths.send(path).is_err() {
                        return;
                    }
                    (self.ivars().wake)();
                }
            }
        }
    );

    impl OpenedDocuments {
        /// A handler that sends what it reads down `paths` and calls `wake`
        /// after each path.
        fn new(
            mtm: MainThreadMarker,
            paths: Sender<PathBuf>,
            wake: Box<dyn Fn() + Send + Sync>,
        ) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(Opened { paths, wake });
            // SAFETY: `init` is NSObject's own initializer, and the
            // instance variables are in place before it is called.
            unsafe { msg_send![super(this), init] }
        }
    }

    /// Every file the open-documents event names, in the order the event
    /// lists them.
    ///
    /// The Finder names a file as a file URL. `fileURLValue` coerces the
    /// descriptor to a file URL first, so it reads the other forms a
    /// program may name a file in as well: an alias record, a file
    /// reference, and a path written as text. A file the descriptor gives
    /// in some form that coerces to no file URL is left out, and so is one
    /// whose URL names no path, because the window opens a mix by path.
    fn paths_in(event: &NSAppleEventDescriptor) -> Vec<PathBuf> {
        let Some(files) = event.paramDescriptorForKeyword(DIRECT_PARAMETER) else {
            return Vec::new();
        };
        // The system sends a list of files, which is a list of one file for
        // one document. A descriptor that is no list counts no items and is
        // one file itself.
        let items = files.numberOfItems();
        if items == 0 {
            return path_of(&files).into_iter().collect();
        }
        // The runtime counts the items of a descriptor from one.
        (1..=items)
            .filter_map(|index| files.descriptorAtIndex(index))
            .filter_map(|file| path_of(&file))
            .collect()
    }

    /// The path of one file an Apple event names, or nothing when the
    /// descriptor gives the file in a form that coerces to no file URL, or
    /// the URL names no path.
    fn path_of(file: &NSAppleEventDescriptor) -> Option<PathBuf> {
        let url = file.fileURLValue()?;
        let path = url.path()?;
        Some(PathBuf::from(path.to_string()))
    }

    /// Starts listening for the open-documents event and answers the
    /// channel the paths arrive on. [`super::watch_opened_documents`]
    /// describes what the caller gets.
    pub(crate) fn watch(wake: Box<dyn Fn() + Send + Sync>) -> Receiver<PathBuf> {
        let (paths, receiver) = mpsc::channel();
        let Some(mtm) = MainThreadMarker::new() else {
            return receiver;
        };
        register_as_the_application_finishes_launching(OpenedDocuments::new(mtm, paths, wake));
        receiver
    }

    /// Registers `handler` for the open-documents event at the moment the
    /// application is about to finish launching, by asking the notification
    /// center for the notification that marks the moment.
    ///
    /// The moment is what makes `handler` the one the event reaches, and
    /// the two moments on either side of it each miss a document. A handler
    /// registered before the event loop starts misses every document,
    /// because NSApplication registers a handler of its own for the
    /// open-documents event as it finishes launching, in the place of the
    /// earlier one, and NSApplication's handler passes the documents to the
    /// application delegate, which the window toolkit's delegate does not
    /// answer. A handler registered once the application has finished
    /// launching misses the document the application was started to open,
    /// because the system dispatches that event before the window exists.
    /// The notification this asks for is posted between those two moments.
    fn register_as_the_application_finishes_launching(handler: Retained<OpenedDocuments>) {
        let register = RcBlock::new(move |_notification: NonNull<NSNotification>| {
            let manager = NSAppleEventManager::sharedAppleEventManager();
            // SAFETY: the handler answers the selector named here, with the
            // two descriptors the manager passes, and it lives for as long
            // as this block does, which is the life of the process.
            unsafe {
                manager.setEventHandler_andSelector_forEventClass_andEventID(
                    &handler,
                    sel!(openDocuments:withReplyEvent:),
                    SYSTEM_EVENTS,
                    OPEN_DOCUMENTS,
                );
            }
        });
        let center = NSNotificationCenter::defaultCenter();
        // SAFETY: the name is a constant AppKit defines.
        let launching = unsafe { NSApplicationWillFinishLaunchingNotification };
        // SAFETY: the block is not `Send`, because it holds a handler that
        // belongs to the main thread. No queue is named, so the notification
        // center runs the block on the thread that posts the notification,
        // and NSApplication posts this one on the main thread.
        let observer = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(launching),
                None,
                None,
                &register,
            )
        };
        // Nothing removes the observer, because the application listens for
        // documents for as long as it runs. The notification center holds
        // the block, and the handler the block holds, until a call to
        // `removeObserver:`, so the token here is of no further use.
        drop(observer);
    }
}
