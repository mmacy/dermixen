//! A field a tempo is typed into, and the rules for when the window
//! writes what was typed: once, when the person finishes, to the target
//! settled when the typing began.
//!
//! The window has three such fields: the master tempo above the timeline,
//! the tempo of the selected tempo node beside it, and the grid editor's
//! exact tempo. Each reports what the window is to write as an undoable
//! step, so each must report at most once per visit to the field rather
//! than once per keystroke, and a field the person clicked into and left
//! alone must report nothing to write.

use std::num::NonZeroU32;

use dermixen_core::Bpm;

/// What a finished typing tells the window to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Finish<T> {
    /// The window is to write the typed tempo for the target.
    Written(T, Bpm),
    /// Nothing was typed, or the typed tempo is the one the field already
    /// showed, so the window has nothing to write.
    Unchanged,
    /// The text does not read as a positive finite tempo, and holds what
    /// was typed with the spaces around it removed, so the window can say
    /// what was refused, and an empty text stays empty.
    Refused(String),
}

/// The empty text a field shows for a tempo that is not a finite number.
const NO_TEMPO_TEXT: &str = "";

/// Formats `bpm` with `decimals` digits after the point, or as
/// [`NO_TEMPO_TEXT`] when `bpm` is not a finite number.
fn format_tempo(bpm: Bpm, decimals: usize) -> String {
    if bpm.0.is_finite() {
        format!("{:.decimals$}", bpm.0)
    } else {
        NO_TEMPO_TEXT.to_owned()
    }
}

/// Reads `text`, with the spaces around it ignored, as a tempo that is
/// positive and finite, or gives back `None` for text that does not read
/// as one.
fn parse_tempo(text: &str) -> Option<Bpm> {
    text.trim()
        .parse::<f64>()
        .ok()
        .map(Bpm)
        .filter(|bpm| bpm.is_valid())
}

/// A field showing a tempo, which a person may type a new tempo into.
///
/// `T` names what the tempo is for, such as which node it belongs to. The
/// target is settled when typing begins, so a selection that changes while
/// the person types does not change what the typed tempo goes to.
#[derive(Debug, Clone, PartialEq)]
pub struct TempoField<T> {
    /// How many digits after the point the field shows.
    decimals: usize,
    /// The latest tempo `show` was given, including one given while typing
    /// was under way, and one a finish reports as written. This is what the
    /// field goes back to showing after a finish that reports nothing to
    /// write.
    shown: Bpm,
    /// What the field shows when nobody is typing: `shown`, formatted, or
    /// the tempo a finish last reported as written, until the next `show`.
    resting_text: String,
    /// The tempo the field showed when the typing under way began.
    /// `finish` compares the typed tempo against this, so retyping the same
    /// tempo writes nothing.
    begin_tempo: Bpm,
    /// The target settled when typing began, and the text being typed,
    /// held together so that one is never set without the other; `None`
    /// while nobody is typing.
    editing: Option<(T, String)>,
}

impl<T: Clone> TempoField<T> {
    /// A field showing `shown`, written with `decimals` digits after the
    /// point. A tempo that is not a finite number is shown as empty text,
    /// here and in [`show`](TempoField::show).
    pub fn new(shown: Bpm, decimals: usize) -> TempoField<T> {
        TempoField {
            decimals,
            shown,
            resting_text: format_tempo(shown, decimals),
            begin_tempo: shown,
            editing: None,
        }
    }

    /// Gives the field the tempo to show when nobody is typing, as the
    /// window does on every repaint. While typing is under way the tempo
    /// is kept and shown once the typing is over, so the field is not
    /// rewritten under the person's fingers.
    pub fn show(&mut self, bpm: Bpm) {
        self.shown = bpm;
        self.resting_text = format_tempo(bpm, self.decimals);
    }

    /// What the field shows: the text being typed while typing is under
    /// way, and otherwise the shown tempo with the field's number of
    /// decimals.
    pub fn text(&self) -> &str {
        match &self.editing {
            Some((_, typed)) => typed,
            None => &self.resting_text,
        }
    }

    /// Begins typing for `target`, as the field gaining the keyboard does,
    /// settling the target and the tempo the field showed at that moment.
    /// A begin while typing is already under way changes nothing.
    pub fn begin(&mut self, target: T) {
        if self.editing.is_some() {
            return;
        }
        self.begin_tempo = self.shown;
        self.editing = Some((target, self.resting_text.clone()));
    }

    /// Replaces the text being typed. Text given while no typing is under
    /// way is ignored.
    pub fn edit(&mut self, text: &str) {
        if let Some((_, typed)) = &mut self.editing {
            *typed = text.to_owned();
        }
    }

    /// Ends the typing, as the return key or the field losing the keyboard
    /// does. The text, with the spaces around it ignored, is read as a
    /// tempo: one that is positive and finite and differs from the tempo
    /// the field showed when typing began is [`Written`](Finish::Written)
    /// with the settled target, and the field shows it until the next
    /// `show`; one equal to that tempo is [`Unchanged`](Finish::Unchanged);
    /// and text that does not read as such a tempo is
    /// [`Refused`](Finish::Refused) with the text as typed, the spaces
    /// around it removed. Text left exactly as the field showed it when
    /// typing began reports [`Unchanged`](Finish::Unchanged) outright,
    /// whatever tempo it reads as, so retyping the field's own rounded
    /// display, when the shown tempo has more precision than the field's
    /// decimals, reports nothing to write. In the last two cases the field
    /// goes back to showing the shown tempo, which is the latest tempo
    /// `show` was given, including one given while the typing was under
    /// way. A finish with no typing under way is `Unchanged`.
    pub fn finish(&mut self) -> Finish<T> {
        let Some((target, typed)) = self.editing.take() else {
            return Finish::Unchanged;
        };
        let begin_text = format_tempo(self.begin_tempo, self.decimals);
        if typed.trim() == begin_text {
            self.resting_text = format_tempo(self.shown, self.decimals);
            return Finish::Unchanged;
        }
        match parse_tempo(&typed) {
            Some(bpm) if bpm.0 != self.begin_tempo.0 => {
                self.show(bpm);
                Finish::Written(target, bpm)
            }
            Some(_) => {
                self.resting_text = format_tempo(self.shown, self.decimals);
                Finish::Unchanged
            }
            None => {
                self.resting_text = format_tempo(self.shown, self.decimals);
                Finish::Refused(typed.trim().to_owned())
            }
        }
    }

    /// Whether typing is under way.
    pub fn editing(&self) -> bool {
        self.editing.is_some()
    }
}

/// What the audio buffer field takes, in the words `docs/settings.md` uses
/// for the `audio_buffer_frames` setting.
const BUFFER_FRAMES_TAKES: &str = "a whole number of frames from 1 to 4294967295";

/// Reads the text of the settings dialog's audio buffer field as the
/// `audio_buffer_frames` setting. Spaces around the text are ignored.
/// Empty text is `None`, which leaves the size to the device, and a whole
/// number from 1 to 4294967295 is that many frames. Any other text is
/// refused with a message that quotes what was typed and says that the
/// field takes a whole number of frames from 1 to 4294967295.
pub fn parse_buffer_frames(text: &str) -> Result<Option<NonZeroU32>, String> {
    let typed = text.trim();
    if typed.is_empty() {
        return Ok(None);
    }
    typed
        .parse::<NonZeroU32>()
        .map(Some)
        .map_err(|_| format!("The audio buffer takes {BUFFER_FRAMES_TAKES}, not \"{typed}\""))
}
