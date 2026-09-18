//! Acceptance tests for the settings dialog's audio buffer field: what its
//! text reads as, and what is refused. A coder agent makes these pass without
//! editing them.

use dermixen_app::parse_buffer_frames;

#[test]
fn empty_text_leaves_the_size_to_the_device() {
    assert_eq!(parse_buffer_frames(""), Ok(None));
    assert_eq!(parse_buffer_frames("   "), Ok(None));
}

#[test]
fn a_whole_number_of_frames_is_that_many() {
    assert_eq!(parse_buffer_frames("256").unwrap().unwrap().get(), 256);
    assert_eq!(parse_buffer_frames(" 64 ").unwrap().unwrap().get(), 64);
    assert_eq!(parse_buffer_frames("1").unwrap().unwrap().get(), 1);
}

#[test]
fn anything_else_is_refused_with_what_was_typed() {
    for typed in ["0", "-1", "1.5", "many", "4294967296", "256 frames"] {
        let said = parse_buffer_frames(typed).expect_err(typed);
        assert!(said.contains(typed.trim()), "{typed}: {said}");
        assert!(
            said.contains("a whole number of frames from 1 to 4294967295"),
            "{typed}: the message says what the field takes: {said}"
        );
    }
}
