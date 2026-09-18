//! Acceptance tests for the typed tempo field: when what was typed is
//! written, to what, and what is refused. A coder agent makes these pass without
//! editing them.

use dermixen_app::{Finish, TempoField};
use dermixen_core::Bpm;

#[test]
fn a_field_shows_the_tempo_given_it_with_its_decimals_until_typing_begins() {
    let mut field: TempoField<()> = TempoField::new(Bpm(128.0), 1);
    assert_eq!(field.text(), "128.0");
    assert!(!field.editing());
    field.show(Bpm(135.87));
    assert_eq!(field.text(), "135.9");
    let three: TempoField<()> = TempoField::new(Bpm(138.25), 3);
    assert_eq!(three.text(), "138.250");
}

#[test]
fn a_typed_tempo_is_written_once_when_typing_finishes() {
    let mut field: TempoField<()> = TempoField::new(Bpm(128.0), 1);
    field.begin(());
    assert!(field.editing());
    assert_eq!(
        field.text(),
        "128.0",
        "the text starts as what the field showed"
    );
    field.edit("1");
    field.edit("13");
    field.edit("130");
    assert_eq!(field.text(), "130");
    assert_eq!(field.finish(), Finish::Written((), Bpm(130.0)));
    assert!(!field.editing());
    assert_eq!(
        field.text(),
        "130.0",
        "the field shows what was written until the next show"
    );
    assert_eq!(
        field.finish(),
        Finish::Unchanged,
        "a finish with no typing under way writes nothing"
    );
    field.show(Bpm(130.0));
    assert_eq!(field.text(), "130.0");
}

#[test]
fn a_field_clicked_into_and_left_alone_writes_nothing() {
    let mut field: TempoField<()> = TempoField::new(Bpm(128.0), 1);
    field.begin(());
    assert_eq!(field.finish(), Finish::Unchanged);
    assert_eq!(field.text(), "128.0");

    // Typing the tempo the field already showed writes nothing either, in
    // any spelling, with the spaces around it ignored, and a tempo that
    // differs by a hair is written.
    for same in ["128", "128.0", "128.00", " 128 "] {
        field.begin(());
        field.edit(same);
        assert_eq!(field.finish(), Finish::Unchanged, "{same:?} was written");
    }
    field.begin(());
    field.edit("128.001");
    assert_eq!(field.finish(), Finish::Written((), Bpm(128.001)));
    field.begin(());
    field.edit(" 129 ");
    assert_eq!(
        field.finish(),
        Finish::Written((), Bpm(129.0)),
        "the spaces around a tempo are ignored"
    );
}

#[test]
fn a_value_that_is_not_a_tempo_is_refused_and_the_field_goes_back() {
    let mut field: TempoField<()> = TempoField::new(Bpm(128.0), 1);
    for bad in ["fast", "", "0", "-5", "nan", "inf", "1e400", "130 bpm"] {
        field.begin(());
        field.edit(bad);
        assert_eq!(
            field.finish(),
            Finish::Refused(bad.to_owned()),
            "{bad:?} was not refused"
        );
        assert!(!field.editing());
        assert_eq!(field.text(), "128.0", "after {bad:?}");
    }

    // A tempo that is not a number cannot be shown, so the field is empty.
    let mut field: TempoField<()> = TempoField::new(Bpm(f64::NAN), 1);
    assert_eq!(field.text(), "");
    field.show(Bpm(f64::INFINITY));
    assert_eq!(field.text(), "");
    field.show(Bpm(128.0));
    assert_eq!(field.text(), "128.0");
}

#[test]
fn the_target_is_settled_when_typing_begins() {
    let mut field: TempoField<(usize, i64)> = TempoField::new(Bpm(128.0), 1);
    field.begin((0, 16));
    field.edit("130");
    field.begin((1, 32));
    assert_eq!(
        field.text(),
        "130",
        "a second begin while typing changes nothing"
    );
    assert_eq!(field.finish(), Finish::Written((0, 16), Bpm(130.0)));
    field.begin((1, 32));
    field.edit("131");
    assert_eq!(field.finish(), Finish::Written((1, 32), Bpm(131.0)));
}

#[test]
fn a_tempo_shown_while_typing_waits_until_the_typing_is_over() {
    let mut field: TempoField<()> = TempoField::new(Bpm(128.0), 1);
    field.begin(());
    field.edit("125");
    field.show(Bpm(140.0));
    assert_eq!(
        field.text(),
        "125",
        "the text is not rewritten under the person's fingers"
    );
    assert_eq!(field.finish(), Finish::Written((), Bpm(125.0)));
    assert_eq!(field.text(), "125.0");

    field.begin(());
    field.edit("abc");
    field.show(Bpm(150.0));
    assert_eq!(field.finish(), Finish::Refused("abc".to_owned()));
    assert_eq!(
        field.text(),
        "150.0",
        "the tempo shown during the typing is what the field goes back to"
    );
}

#[test]
fn a_refused_text_is_reported_without_the_spaces_around_it() {
    let mut field: TempoField<()> = TempoField::new(Bpm(128.0), 1);
    field.begin(());
    field.edit("  abc ");
    assert_eq!(field.finish(), Finish::Refused("abc".to_owned()));
    field.begin(());
    field.edit("   ");
    assert_eq!(
        field.finish(),
        Finish::Refused(String::new()),
        "only spaces is an empty text"
    );
    field.begin(());
    field.edit(" 130 bpm ");
    assert_eq!(field.finish(), Finish::Refused("130 bpm".to_owned()));
}
