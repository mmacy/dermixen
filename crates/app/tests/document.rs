//! Acceptance tests for the document under the window: which file the mix
//! is, the title the window shows for it, and the question the window asks
//! before it lets unsaved changes go. A coder agent makes these pass without
//! editing them.
//!
//! These rules are what keeps **New**, **Open**, and **Quit** from losing
//! work, and they are tested here without a window because the window only
//! paints what this type says.

use std::path::{Path, PathBuf};

use dermixen_app::{Answer, Document, EDITED, Intent, Next, Step, UNTITLED};

fn set() -> PathBuf {
    PathBuf::from("/music/mixes/set.dmx")
}

#[test]
fn an_untitled_mix_is_named_untitled_and_a_file_by_its_name() {
    let untitled = Document::untitled();
    assert_eq!(untitled.path(), None);
    assert_eq!(untitled.name(), UNTITLED);
    assert_eq!(untitled.title(), "Dermixen · Untitled");
    assert!(!untitled.is_unsaved());
    assert!(!untitled.asking());

    let named = Document::at(set());
    assert_eq!(named.path(), Some(Path::new("/music/mixes/set.dmx")));
    assert_eq!(named.name(), "set.dmx");
    assert_eq!(named.title(), "Dermixen · set.dmx");
    assert!(!named.is_unsaved());
}

#[test]
fn the_title_says_edited_from_the_first_edit_until_the_next_save() {
    let mut document = Document::at(set());
    document.edited();
    assert!(document.is_unsaved());
    assert_eq!(document.title(), format!("Dermixen · set.dmx{EDITED}"));
    document.edited();
    assert_eq!(
        document.title(),
        format!("Dermixen · set.dmx{EDITED}"),
        "a second edit adds nothing to the title"
    );
    document.saved();
    assert!(!document.is_unsaved());
    assert_eq!(document.title(), "Dermixen · set.dmx");
    assert_eq!(
        document.path(),
        Some(Path::new("/music/mixes/set.dmx")),
        "a save keeps the file"
    );

    let mut untitled = Document::untitled();
    untitled.edited();
    assert_eq!(untitled.title(), format!("Dermixen · Untitled{EDITED}"));
    untitled.saved_as(PathBuf::from("/music/mixes/new.dmx"));
    assert_eq!(untitled.path(), Some(Path::new("/music/mixes/new.dmx")));
    assert_eq!(untitled.name(), "new.dmx");
    assert_eq!(untitled.title(), "Dermixen · new.dmx");
    assert!(!untitled.is_unsaved());
}

#[test]
fn an_intent_proceeds_at_once_when_nothing_is_unsaved() {
    let mut document = Document::at(set());
    assert_eq!(document.request(Intent::New), Step::Proceed(Intent::New));
    assert!(!document.asking());
    let open = Intent::Open(PathBuf::from("/music/mixes/other.dmx"));
    assert_eq!(document.request(open.clone()), Step::Proceed(open));
    assert_eq!(document.request(Intent::Quit), Step::Proceed(Intent::Quit));
    assert_eq!(document.take_pending(), None);
    assert_eq!(
        document.answered(Answer::Discard),
        Next::Stay,
        "an answer while nothing is pending changes nothing"
    );
}

#[test]
fn unsaved_changes_make_the_window_ask_and_the_answer_decides() {
    let mut document = Document::at(set());
    document.edited();

    // Cancel: the intent is dropped and the changes stay.
    assert_eq!(document.request(Intent::Quit), Step::Ask(Intent::Quit));
    assert!(document.asking());
    assert_eq!(document.pending(), Some(&Intent::Quit));
    assert_eq!(document.answered(Answer::Cancel), Next::Stay);
    assert!(!document.asking());
    assert!(document.is_unsaved(), "cancelling saves nothing");
    assert_eq!(document.take_pending(), None);

    // Discard: the intent proceeds and the changes are let go.
    assert_eq!(document.request(Intent::New), Step::Ask(Intent::New));
    assert_eq!(
        document.answered(Answer::Discard),
        Next::Proceed(Intent::New)
    );
    assert!(!document.asking());
    assert_eq!(document.take_pending(), None);

    // Save: the intent waits for the save, and proceeds once it succeeds.
    let open = Intent::Open(PathBuf::from("/music/mixes/other.dmx"));
    assert_eq!(document.request(open.clone()), Step::Ask(open.clone()));
    assert_eq!(
        document.answered(Answer::Save),
        Next::SaveThen(open.clone())
    );
    assert!(
        document.asking(),
        "the question stands until the save has succeeded"
    );
    assert_eq!(
        document.take_pending(),
        None,
        "nothing proceeds before the save"
    );
    document.saved();
    assert_eq!(document.take_pending(), Some(open));
    assert!(!document.asking());
    assert_eq!(document.take_pending(), None, "the intent is given once");
    assert!(!document.is_unsaved());
}

#[test]
fn a_save_as_on_an_untitled_mix_carries_the_waiting_intent_through() {
    let mut untitled = Document::untitled();
    untitled.edited();
    assert_eq!(untitled.request(Intent::Quit), Step::Ask(Intent::Quit));
    assert_eq!(
        untitled.answered(Answer::Save),
        Next::SaveThen(Intent::Quit),
        "the window asks for a file, since the mix has none"
    );
    untitled.saved_as(PathBuf::from("/music/mixes/first.dmx"));
    assert_eq!(untitled.take_pending(), Some(Intent::Quit));
    assert_eq!(untitled.name(), "first.dmx");
}

#[test]
fn a_failed_save_leaves_the_question_standing_and_a_cancel_withdraws_it() {
    let mut document = Document::at(set());
    document.edited();
    assert_eq!(document.request(Intent::Quit), Step::Ask(Intent::Quit));
    assert_eq!(
        document.answered(Answer::Save),
        Next::SaveThen(Intent::Quit)
    );
    // The save failed: the window called neither saved nor saved_as.
    assert!(document.asking());
    assert_eq!(document.pending(), Some(&Intent::Quit));
    assert_eq!(document.take_pending(), None);
    assert!(document.is_unsaved());
    // The person gives up on quitting for now.
    assert_eq!(document.answered(Answer::Cancel), Next::Stay);
    assert!(!document.asking());
    assert!(document.is_unsaved());
}

#[test]
fn a_newer_request_replaces_the_intent_that_was_waiting() {
    let mut document = Document::at(set());
    document.edited();
    assert_eq!(document.request(Intent::Quit), Step::Ask(Intent::Quit));
    let open = Intent::Open(PathBuf::from("/music/mixes/other.dmx"));
    assert_eq!(document.request(open.clone()), Step::Ask(open.clone()));
    assert_eq!(document.pending(), Some(&open));
    assert_eq!(document.answered(Answer::Discard), Next::Proceed(open));
}
