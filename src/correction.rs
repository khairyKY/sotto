//! Voice correction (#20): right after a dictation lands, saying "correction:
//! Claude, not clawed" swaps the misheard word in that text, types the fixed
//! text back over it, and teaches the pair to the vocabulary. The command
//! itself is never typed.
//!
//! The one correctness trap is typing over the wrong text: by the time the
//! command arrives the caret may have moved. `main.rs`'s `run_correction`
//! only retypes after a copy of the re-selected span proves it is that
//! dictation, unchanged (`same_text`); anything else leaves the document
//! alone and puts the fix on the clipboard. `reselect` is the walk back over
//! it, in either paragraph direction.
//!
//! Backtrack (#26) shares the memory and the guard: "scratch that" deletes
//! the last dictation, once the same copy proves it's still there.

use crate::inject::Arrow;
use crate::transform::Outcome;
use harper_core::spell::{Dictionary, FstDictionary};
use std::sync::Mutex;

/// The last delivered dictation and the window it went to.
static LAST: Mutex<Option<(String, isize)>> = Mutex::new(None);

pub fn remember(text: &str, hwnd: isize) {
    *LAST.lock().unwrap() = Some((text.to_string(), hwnd));
}

pub fn last() -> Option<(String, isize)> {
    LAST.lock().unwrap().clone()
}

/// A backtrack deleted it: nothing is left to correct or undo.
pub fn forget() {
    *LAST.lock().unwrap() = None;
}

/// `(right, wrong)` when the whole take is "correction: X, not Y", however
/// the ASR punctuated it ("Correction, Claude not clawed." too). X and Y are
/// 1-3 words and can't span a sentence break. X can't hold sentence glue
/// ("is", "the"): that is what keeps a real sentence that opens with the word
/// ("Correction: this is not right.") a dictation. The command words are
/// English; X and Y can be any script.
pub fn parse(take: &str) -> Option<(String, String)> {
    let words: Vec<(&str, &str)> = take
        .split_whitespace()
        .map(|w| (w, w.trim_matches(|c: char| !c.is_alphanumeric())))
        .filter(|(_, core)| !core.is_empty())
        .collect();
    let ((_, first), rest) = words.split_first()?;
    if !first.eq_ignore_ascii_case("correction") {
        return None;
    }
    let not = rest.iter().position(|(_, w)| w.eq_ignore_ascii_case("not"))?;
    let (right, wrong) = (&rest[..not], &rest[not + 1..]);
    let phrase = |ws: &[(&str, &str)]| {
        let breaks = ws.iter().rev().skip(1).any(|(raw, _)| raw.ends_with(['.', '!', '?']));
        (!ws.is_empty() && ws.len() <= 3 && !breaks).then(|| ws.iter().map(|(_, w)| *w).collect::<Vec<_>>().join(" "))
    };
    let (right, wrong) = (phrase(right)?, phrase(wrong)?);
    if right.split(' ').any(is_glue) || right.eq_ignore_ascii_case(&wrong) {
        return None;
    }
    Some((right, wrong))
}

/// Backtrack (#26): whether the whole take is "scratch that", "undo that" or
/// "delete that", however the ASR punctuated or cased it. Only the whole
/// take, like `parse`: "Scratch that idea." stays a dictation.
pub fn is_backtrack(take: &str) -> bool {
    const COMMANDS: [&str; 3] = ["scratch that", "undo that", "delete that"];
    let said: Vec<String> = take
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase())
        .filter(|w| !w.is_empty())
        .collect();
    COMMANDS.contains(&said.join(" ").as_str())
}

/// Backtrack (#26): the dictation "scratch that" said in window `focus` may
/// delete, or the pill state that says why not. Only the last one, only in
/// the window it went to, and never in a terminal (see `is_terminal`).
pub fn undo_target(last: Option<(String, isize)>, focus: isize, app_name: &str) -> Result<String, &'static str> {
    match last {
        Some((text, hwnd)) if hwnd == focus => if is_terminal(app_name) { Err("undoterminal") } else { Ok(text) },
        _ => Err("nothingtoundo"),
    }
}

/// Polish's list of sentence-glue words, which it compares apostrophe-free.
fn is_glue(word: &str) -> bool {
    let plain = word.to_lowercase().replace(['\'', '’'], "");
    crate::polish::FUNCTION_WORDS.contains(&plain.as_str())
}

/// `text` with every whole-word, case-insensitive `wrong` swapped for
/// `right`, cased like the word it replaces: "Clawed" opening a sentence
/// gets "Claude", a shouted "CLAWED" gets "CLAUDE", anything else gets
/// `right` as spoken. `None` if `wrong` isn't in it.
pub fn replace_word(text: &str, wrong: &str, right: &str) -> Option<String> {
    if wrong.is_empty() {
        return None;
    }
    // ASCII-only lowering keeps byte offsets valid in `text`.
    let (hay, needle) = (text.to_ascii_lowercase(), wrong.to_ascii_lowercase());
    let edge = |c: Option<char>| !c.is_some_and(char::is_alphanumeric);
    let (mut out, mut done, mut at, mut hit) = (String::with_capacity(text.len()), 0, 0, false);
    while let Some(rel) = hay[at..].find(&needle) {
        let (start, end) = (at + rel, at + rel + needle.len());
        if edge(text[..start].chars().next_back()) && edge(text[end..].chars().next()) {
            out.push_str(&text[done..start]);
            out.push_str(&cased_like(&text[start..end], right));
            (done, at, hit) = (end, end, true);
        } else {
            at = start + text[start..].chars().next().map_or(1, char::len_utf8);
        }
    }
    hit.then(|| out + &text[done..])
}

fn cased_like(found: &str, right: &str) -> String {
    let letters = || found.chars().filter(|c| c.is_alphabetic());
    if letters().count() > 1 && letters().all(char::is_uppercase) {
        return right.to_uppercase();
    }
    let mut chars = right.chars();
    match chars.next() {
        Some(first) if found.starts_with(char::is_uppercase) => first.to_uppercase().chain(chars).collect(),
        _ => right.to_string(),
    }
}

/// Whether the pair is a mishearing worth teaching (the Pronunciation
/// trainer's `heard_as` + a dictionary entry that rewrites `wrong` in every
/// later dictation) rather than a one-off edit of this text. Not when `wrong`
/// is only sentence glue ("Kai, not I"): that entry would rewrite ordinary
/// words everywhere. Not with digits ("15, not 50"): a figure is content, not
/// a sound. And not when `right` is plain English ("affect, not effect"): a
/// name or term is what gets misheard, a real word traded for another is an
/// edit. ponytail: a name swapped for a name ("Anthropic, not OpenAI") still
/// teaches; the Dictionary page takes it back out.
pub fn harvests(right: &str, wrong: &str) -> bool {
    // Same "real English word" test as polish's phonetic corrector.
    let english = FstDictionary::curated();
    let real = |w: &str| english.get_word_metadata_str(w).is_some_and(|m| m.common || !m.is_proper_noun());
    !wrong.split(' ').all(is_glue)
        && !format!("{right}{wrong}").contains(|c: char| c.is_ascii_digit())
        && !right.split(' ').all(real)
}

/// Whether a copy of the re-selected span is the dictation. Editors that
/// store CRLF (Notepad) copy a typed "\n" back as "\r\n".
pub fn same_text(copied: &str, injected: &str) -> bool {
    copied.replace('\r', "") == injected.replace('\r', "")
}

/// The fewest Shift+arrow presses that walk over `text`: its characters,
/// less the ones no editor stops at on their own. A letter and the marks on
/// it are one stop ("شغّل" is 4 characters and 3 stops), and so is a line
/// break however it is stored.
///
/// Editors disagree past that (#121 spike). Chromium, VS Code's editor and
/// the Win32 edit control take a marked letter in one press; RichEdit stops
/// at each Arabic mark; the edit control stops inside an emoji sequence the
/// others take whole. So this is the low count, and `more_stops` adds the
/// rest once a copy shows how far the walk got.
/// ponytail: the marks a dictation can hold, not Unicode's grapheme rules
/// (`unicode-segmentation`, already in the lock through Tauri, if it ever
/// matters). A miscount fails the copy check, so nothing is typed.
pub fn caret_stops(text: &str) -> usize {
    let mut prev = '\0';
    text.chars()
        .filter(|&c| {
            let rides = rides(c) || prev == '\u{200D}' || (prev == '\r' && c == '\n');
            prev = c;
            !rides
        })
        .count()
}

/// A character drawn on the one before it: Latin accents, Arabic harakat
/// (whole ranges: a sign counted as a mark only costs one more walk), the
/// joiners, variation selectors, emoji skin tones and flag letters.
fn rides(c: char) -> bool {
    matches!(c,
        '\u{0300}'..='\u{036F}'
        | '\u{0610}'..='\u{061A}' | '\u{064B}'..='\u{065F}' | '\u{0670}' | '\u{06D6}'..='\u{06ED}' | '\u{08D3}'..='\u{08FF}'
        | '\u{200C}' | '\u{200D}' | '\u{FE00}'..='\u{FE0F}' | '\u{1F3FB}'..='\u{1F3FF}' | '\u{1F1E6}'..='\u{1F1FF}')
}

/// How many more stops to walk when the copy is only the tail of the
/// dictation: the editor has more stops than `caret_stops` counted, and what
/// is missing says how far is left. `None` for any other copy.
pub fn more_stops(copied: &str, injected: &str) -> Option<usize> {
    let (copied, injected) = (copied.replace('\r', ""), injected.replace('\r', ""));
    let missing = injected.strip_suffix(copied.as_str()).filter(|m| !m.is_empty() && !copied.is_empty())?;
    Some(caret_stops(missing).max(1))
}

/// Re-select the dictation `old`, which should end at the caret, and replace
/// it. `walk(arrow, n)` grows the selection `n` stops with Shift+arrow,
/// copies it, and replaces it only if the copy is `old`; it returns how that
/// went, and the copy when it was something else. `press` is the bare arrow.
/// True once replaced.
///
/// Shift+Left walks back over text in a left-to-right paragraph, whatever
/// script the text is in. In a right-to-left paragraph (an Arabic chat box,
/// a field set to right-to-left) it walks forward and Shift+Right walks back
/// (#121 spike: Chromium, the Win32 edit control and RichEdit all agree).
/// Nothing outside the field says which it is, so Left goes first, then
/// Right from the same caret. A copy that is the tail of `old` means the walk
/// stopped short, and it goes on for as long as each copy is a longer tail.
///
/// A walk that replaced nothing puts the caret back:
/// - A span that isn't the dictation (`Kept`): the other arrow, once,
///   collapses it onto the end the walk started from.
/// - An empty copy after Shift+Left: nothing. Either the caret had nowhere to
///   go (the end of a right-to-left paragraph, where a press would walk one
///   stop into the dictation), or the app selected and couldn't copy, and the
///   Shift+Right walk retraces that stop for stop. A field that ignores Shift
///   gets its caret walked back the same way.
/// - An empty copy after Shift+Right: Left then Right, which collapses
///   whatever an app with no Ctrl+Insert copy still has selected, and
///   otherwise ends where it began.
///
/// ponytail: the second walk costs a right-to-left paragraph the ~0.5 s the
/// first waits on an empty clipboard; read the paragraph's direction (UIA)
/// if that ever matters. And an app with no copy, in a right-to-left
/// paragraph, ends one stop inside the dictation with nothing selected. The
/// console line editors skip the key path (`is_terminal`).
pub fn reselect(
    old: &str,
    mut walk: impl FnMut(Arrow, usize) -> (Outcome, String),
    mut press: impl FnMut(Arrow),
) -> bool {
    for arrow in [Arrow::Left, Arrow::Right] {
        let (mut steps, mut reached) = (caret_stops(old).max(1), 0);
        let outcome = loop {
            let (outcome, copied) = walk(arrow, steps);
            match more_stops(&copied, old) {
                Some(more) if outcome == Outcome::Kept && copied.len() > reached => (steps, reached) = (more, copied.len()),
                _ => break outcome,
            }
        };
        match outcome {
            Outcome::Replaced => return true,
            Outcome::Kept => press(arrow.other()),
            Outcome::NothingSelected if arrow == Arrow::Right => [Arrow::Left, Arrow::Right].into_iter().for_each(&mut press),
            Outcome::NothingSelected => {}
        }
    }
    false
}

/// Console hosts where Shift+Left is a screen selection, not an edit: typing
/// there appends instead of replacing. Names as `stats::app_name` gives them.
/// ponytail: known hosts only; any other terminal gets the key path, where
/// the copy check refuses whatever isn't a clean selection of the dictation.
pub fn is_terminal(app_name: &str) -> bool {
    const HOSTS: [&str; 9] =
        ["Terminal", "cmd", "powershell", "pwsh", "conhost", "OpenConsole", "wezterm-gui", "alacritty", "mintty"];
    HOSTS.iter().any(|h| h.eq_ignore_ascii_case(app_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(right: &str, wrong: &str) -> Option<(String, String)> {
        Some((right.to_string(), wrong.to_string()))
    }

    #[test]
    fn parses_the_command_however_the_asr_punctuates_it() {
        assert_eq!(parse("Correction: Claude, not clawed."), pair("Claude", "clawed"));
        assert_eq!(parse("Correction, Claude not clawed."), pair("Claude", "clawed"));
        assert_eq!(parse("correction claude not clawed"), pair("claude", "clawed"));
        assert_eq!(parse("Correction. Claude. Not clawed."), pair("Claude", "clawed"));
        assert_eq!(parse("  Correction: Sotto, not so to.  "), pair("Sotto", "so to"));
        assert_eq!(parse("Correction: Gemini CLI, not German ICLI."), pair("Gemini CLI", "German ICLI"));
        // What was heard can be glue, or another script.
        assert_eq!(parse("Correction: Kai, not I."), pair("Kai", "I"));
        assert_eq!(parse("Correction: Claude, not كلود."), pair("Claude", "كلود"));
    }

    #[test]
    fn a_sentence_that_merely_holds_the_word_stays_a_dictation() {
        for take in [
            "I made a correction to the doc, not the slides.",
            "The correction is not needed.",
            "Correction is not needed.",
            "Correction: this is not right.",
            "Correction: the meeting is not on Monday.",
            "Corrections: Claude, not clawed.",
            "Correction: Claude.",
            "Correction, not clawed.",
            "Correction: Claude, not",
            "Correction: Claude, not Claude.",
            "Correction: Claude, not clawed. Thanks.",
            "Correction: one two three four, not five.",
            "Correction: yes. Claude, not clawed.",
            "تصحيح: Claude مش clawed",
            "",
        ] {
            assert_eq!(parse(take), None, "{take:?}");
        }
    }

    #[test]
    fn backtrack_is_only_the_whole_take_being_the_command() {
        for take in ["Scratch that.", "scratch that", "Undo that!", "DELETE THAT", "  Scratch, that.  ", "Scratch. That."] {
            assert!(is_backtrack(take), "{take:?}");
        }
        for take in [
            "Scratch that idea.",
            "Please delete that.",
            "Delete that file.",
            "Scratch that, scratch that.",
            "Undo.",
            "Scratch this.",
            "Scratch-that.",
            "امسح ده",
            "",
        ] {
            assert!(!is_backtrack(take), "{take:?}");
        }
    }

    #[test]
    fn backtrack_undoes_only_the_last_dictation_in_its_own_window_and_never_in_a_terminal() {
        let last = || Some(("I asked clawed to help.".to_string(), 7));
        assert_eq!(undo_target(last(), 7, "Notepad"), Ok("I asked clawed to help.".to_string()));
        assert_eq!(undo_target(None, 7, "Notepad"), Err("nothingtoundo"), "nothing dictated, or already undone");
        assert_eq!(undo_target(last(), 8, "Notepad"), Err("nothingtoundo"), "said in another window");
        assert_eq!(undo_target(last(), 7, "Terminal"), Err("undoterminal"));
    }

    #[test]
    fn replaces_whole_words_in_the_casing_of_what_it_replaces() {
        let fix = |t: &str, w: &str, r: &str| replace_word(t, w, r);
        assert_eq!(fix("I asked clawed to help.", "clawed", "Claude").as_deref(), Some("I asked Claude to help."));
        assert_eq!(fix("Clawed said (clawed), CLAWED.", "clawed", "Claude").as_deref(), Some("Claude said (Claude), CLAUDE."));
        assert_eq!(fix("So to is live. I use so to.", "so to", "sotto").as_deref(), Some("Sotto is live. I use sotto."));
        assert_eq!(fix("run the German ICLI now", "german icli", "Gemini CLI").as_deref(), Some("run the Gemini CLI now"));
        assert_eq!(fix("قال كلود كده", "كلود", "Claude").as_deref(), Some("قال Claude كده"));
        // Only whole words: inside another word, Latin or Arabic, it's not there.
        assert_eq!(fix("unclawed clawedness", "clawed", "Claude"), None);
        assert_eq!(fix("والكلود", "كلود", "Claude"), None);
        assert_eq!(fix("nothing to fix", "clawed", "Claude"), None);
    }

    #[test]
    fn teaches_mishearings_but_not_edits_or_glue() {
        assert!(harvests("Claude", "clawed"));
        assert!(harvests("Sotto", "Soto"));
        assert!(harvests("Gemini CLI", "German ICLI"));
        assert!(harvests("Claude", "كلود"));
        assert!(!harvests("Kai", "I"));
        assert!(!harvests("Sotto", "so to"));
        assert!(!harvests("15", "50"));
        assert!(!harvests("affect", "effect"));
        assert!(!harvests("principal", "principle"));
    }

    #[test]
    fn a_copy_matches_the_dictation_across_line_endings_only() {
        assert!(same_text("First line.\r\nSecond.", "First line.\nSecond."));
        assert!(!same_text("I asked clawed to help. More", "I asked clawed to help."));
        assert!(!same_text("", "I asked clawed to help."));
    }

    #[test]
    fn caret_stops_count_a_letter_with_its_marks_once() {
        // The counts Chromium, VS Code's editor and the Win32 edit control
        // needed in the #121 spike.
        assert_eq!(caret_stops("I asked clawed to help."), 23);
        assert_eq!(caret_stops("قال كلود كده"), 12);
        assert_eq!(caret_stops("افتح الـ terminal وشغّل الـ build"), 32, "33 characters, one a shadda");
        assert_eq!(caret_stops("مَرحَباً"), 5, "8 characters, 3 of them harakat");
        assert_eq!(caret_stops("لا لا"), 5, "lam-alef is drawn as one shape and is still two stops");
        assert_eq!(caret_stops("cafe\u{301} ok"), 7);
        assert_eq!(caret_stops("ship it 🚀 now"), 13, "one stop, two UTF-16 units");
        assert_eq!(caret_stops("سطر\nتاني"), 8);
        assert_eq!(caret_stops("سطر\r\nتاني"), 8, "a stored CRLF is one stop");
        // Emoji sequences: never more than the editors that take them whole
        // (Chromium: 8; a flag's two letters both ride, so one under).
        assert_eq!(caret_stops("ok 👍🏽 👨\u{200D}👩\u{200D}👧 🇪🇬"), 7);
        assert_eq!(caret_stops(""), 0);
    }

    #[test]
    fn a_copy_that_is_the_dictations_tail_says_how_far_is_left() {
        let old = "مَرحَباً يا Claude";
        // RichEdit after 15 presses: the first letter, its mark and the next letter are missing.
        assert_eq!(more_stops("حَباً يا Claude", old), Some(2));
        // The walk landed between a letter and its mark: still one press to go.
        assert_eq!(more_stops("\u{64E}رحَباً يا Claude", old), Some(1));
        assert_eq!(more_stops("Second.", "First line.\nSecond."), Some(12));
        assert_eq!(more_stops("line.\r\nSecond.", "First line.\nSecond."), Some(6), "a CRLF editor's copy");
        // The dictation itself, nothing, or other text: no more walking.
        assert_eq!(more_stops(old, old), None);
        assert_eq!(more_stops("", old), None);
        assert_eq!(more_stops("يا Claude more", old), None);
    }

    /// A text field as the #121 spike found them. `rtl`: a right-to-left
    /// paragraph, where Left walks forward. `per_char`: RichEdit, which stops
    /// at each mark. `copies`: has a Ctrl+Insert copy.
    struct Field {
        text: String,
        anchor: usize,
        caret: usize,
        rtl: bool,
        per_char: bool,
        copies: bool,
    }

    impl Field {
        /// `before`, with the caret after it, then `after`.
        fn new(before: &str, after: &str, rtl: bool, per_char: bool) -> Self {
            Self { text: format!("{before}{after}"), anchor: before.len(), caret: before.len(), rtl, per_char, copies: true }
        }

        fn step(&mut self, arrow: Arrow) {
            let mark = |c: char| ('\u{064B}'..='\u{0652}').contains(&c);
            if (arrow == Arrow::Left) != self.rtl {
                let mut back = self.text[..self.caret].chars().rev();
                while let Some(c) = back.next() {
                    self.caret -= c.len_utf8();
                    if self.per_char || !mark(c) {
                        break;
                    }
                }
            } else {
                let mut ahead = self.text[self.caret..].chars().peekable();
                while let Some(c) = ahead.next() {
                    self.caret += c.len_utf8();
                    if self.per_char || !ahead.peek().is_some_and(|&c| mark(c)) {
                        break;
                    }
                }
            }
        }

        fn walk(&mut self, arrow: Arrow, n: usize, old: &str, fixed: &str) -> (Outcome, String) {
            (0..n).for_each(|_| self.step(arrow));
            let span = self.anchor.min(self.caret)..self.anchor.max(self.caret);
            let copied = if self.copies { self.text[span.clone()].to_string() } else { String::new() };
            if copied.trim().is_empty() {
                (Outcome::NothingSelected, String::new())
            } else if same_text(&copied, old) {
                self.text.replace_range(span.clone(), fixed);
                (self.anchor, self.caret) = (span.start + fixed.len(), span.start + fixed.len());
                (Outcome::Replaced, String::new())
            } else {
                (Outcome::Kept, copied)
            }
        }

        /// The bare arrow: collapses a selection to that side, else moves.
        fn press(&mut self, arrow: Arrow) {
            if self.anchor == self.caret {
                self.step(arrow);
            } else if (arrow == Arrow::Right) != self.rtl {
                self.caret = self.anchor.max(self.caret);
            } else {
                self.caret = self.anchor.min(self.caret);
            }
            self.anchor = self.caret;
        }

        fn retype(&mut self, old: &str, fixed: &str) -> bool {
            let field = std::cell::RefCell::new(self);
            reselect(old, |arrow, n| field.borrow_mut().walk(arrow, n, old, fixed), |arrow| field.borrow_mut().press(arrow))
        }
    }

    #[test]
    fn reselects_the_dictation_in_either_paragraph_direction_or_leaves_everything_as_it_was() {
        let old = "افتح الـ terminal وشغّل الـ build"; // one shadda: 32 stops, 33 in RichEdit
        let fixed = "افتح الـ terminal وشغّل الـ test";
        for (rtl, per_char) in [(false, false), (false, true), (true, false), (true, true)] {
            let case = format!("rtl={rtl} per_char={per_char}");
            // At the end of the text, and with more after the caret.
            for after in ["", " بعد كده"] {
                let mut field = Field::new(&format!("X {old}"), after, rtl, per_char);
                assert!(field.retype(old, fixed), "{case} after={after:?}");
                assert_eq!(field.text, format!("X {fixed}{after}"), "{case}");
            }
            // English in the same paragraph, and a backtrack (#26) of it.
            let mut field = Field::new("X I asked clawed to help.", "", rtl, per_char);
            assert!(field.retype("I asked clawed to help.", ""), "{case}");
            assert_eq!(field.text, "X ");
            // Something typed after it, or it's gone: nothing replaced,
            // nothing left selected, the caret where it was.
            for (before, after) in [(format!("X {old} more"), ""), (format!("X {old} more"), " tail"), ("X ".to_string(), "")] {
                for copies in [true, false] {
                    let case = format!("{case} copies={copies} before={before:?} after={after:?}");
                    let mut field = Field::new(&before, after, rtl, per_char);
                    field.copies = copies;
                    assert!(!field.retype(old, fixed), "{case}");
                    assert_eq!(field.text, format!("{before}{after}"), "{case}");
                    assert_eq!(field.anchor, field.caret, "{case}");
                    // The ceiling: no copy and right-to-left ends one stop in.
                    let off = if !copies && rtl { 1 } else { 0 };
                    assert_eq!(field.caret, before.len() - off, "{case}");
                }
            }
        }
    }

    /// `reselect` against the real controls, without touching focus or the
    /// clipboard: a Win32 edit control and a RichEdit, each in left-to-right
    /// and right-to-left reading order, in a window that is never shown. Keys
    /// go in as WM_KEYDOWN on the owning thread, Shift through that thread's
    /// key state, and the "copy" reads the selection. Run by hand:
    /// `cargo test --bin sotto reselects_in_real -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn reselects_in_real_edit_controls() {
        use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyboardState, SetKeyboardState};
        use windows::Win32::UI::WindowsAndMessaging::*;
        use windows::core::{HSTRING, PCWSTR, w};
        const EM_GETSEL: u32 = 0x00B0;
        const EM_SETSEL: u32 = 0x00B1;
        const EM_REPLACESEL: u32 = 0x00C2;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn LoadLibraryW(name: PCWSTR) -> isize;
        }

        fn text(edit: HWND) -> Vec<u16> {
            let mut buf = vec![0u16; 4096];
            let n = unsafe { GetWindowTextW(edit, &mut buf) };
            buf.truncate(n as usize);
            buf
        }
        fn selection(edit: HWND) -> (usize, usize) {
            let (mut start, mut end) = (0u32, 0u32);
            unsafe { SendMessageW(edit, EM_GETSEL, Some(WPARAM(&mut start as *mut u32 as usize)), Some(LPARAM(&mut end as *mut u32 as isize))) };
            (start as usize, end as usize)
        }
        fn key(edit: HWND, arrow: Arrow, n: usize, shift: bool) {
            let (vk, scan) = if arrow == Arrow::Left { (0x25, 0x4B) } else { (0x27, 0x4D) };
            let mut keys = [0u8; 256];
            unsafe {
                GetKeyboardState(&mut keys).unwrap();
                (keys[0x10], keys[0xA0]) = if shift { (0x80, 0x80) } else { (0, 0) };
                SetKeyboardState(&keys).unwrap();
                for _ in 0..n {
                    SendMessageW(edit, WM_KEYDOWN, Some(WPARAM(vk)), Some(LPARAM(1 | scan << 16 | 1 << 24)));
                    SendMessageW(edit, WM_KEYUP, Some(WPARAM(vk)), Some(LPARAM(1 | scan << 16 | 1 << 24 | 3 << 30)));
                }
                (keys[0x10], keys[0xA0]) = (0, 0);
                SetKeyboardState(&keys).unwrap();
            }
        }
        // `before`, the caret, `after`; then what `reselect` made of it: the
        // arrow and walks that replaced it, the text, the selection.
        let run = |edit: HWND, before: &str, after: &str, old: &str, fixed: &str| {
            let caret = before.encode_utf16().count();
            unsafe {
                SetWindowTextW(edit, &HSTRING::from(format!("{before}{after}"))).unwrap();
                SendMessageW(edit, EM_SETSEL, Some(WPARAM(caret)), Some(LPARAM(caret as isize)));
            }
            let (mut walks, mut via) = (0, None);
            reselect(
                old,
                |arrow, n| {
                    key(edit, arrow, n, true);
                    walks += 1;
                    let (all, (start, end)) = (text(edit), selection(edit));
                    let copied = String::from_utf16_lossy(&all[start.min(all.len())..end.min(all.len())]);
                    if copied.trim().is_empty() {
                        (Outcome::NothingSelected, String::new())
                    } else if same_text(&copied, old) {
                        unsafe { SendMessageW(edit, EM_REPLACESEL, Some(WPARAM(1)), Some(LPARAM(HSTRING::from(fixed).as_ptr() as isize))) };
                        via = Some((arrow, walks));
                        (Outcome::Replaced, String::new())
                    } else {
                        (Outcome::Kept, copied)
                    }
                },
                |arrow| key(edit, arrow, 1, false),
            );
            (via, String::from_utf16_lossy(&text(edit)), selection(edit))
        };

        let old = "افتح الـ terminal وشغّل الـ build"; // one shadda
        let fixed = "افتح الـ terminal وشغّل الـ test";
        let marked = "مَرحَباً يا Claude"; // three harakat
        unsafe { LoadLibraryW(w!("Msftedit.dll")) };
        for class in [w!("EDIT"), w!("RICHEDIT50W")] {
            for rtl in [false, true] {
                let case = format!("{} rtl={rtl}", unsafe { class.to_string().unwrap() });
                let edit = unsafe {
                    let host = CreateWindowExW(WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW, w!("STATIC"), w!(""), WS_POPUP, -10000, -10000, 600, 200, None, None, None, None).unwrap();
                    let ex = if rtl { WS_EX_RTLREADING | WS_EX_RIGHT } else { WINDOW_EX_STYLE(0) };
                    CreateWindowExW(ex, class, w!(""), WS_CHILD | WINDOW_STYLE(ES_MULTILINE as u32), 0, 0, 600, 200, Some(host), None, None, None).unwrap()
                };
                let at = |s: &str| s.encode_utf16().count();
                // At the end of the text, with more after the caret, and
                // text whose marks the two controls count differently.
                for (dictated, after) in [(old, ""), (old, " بعد كده"), (marked, "")] {
                    let before = format!("X {dictated}");
                    let (via, now, _) = run(edit, &before, after, dictated, fixed);
                    println!("{case} {dictated:?} after={after:?}: {via:?}");
                    // Left walks back in left-to-right reading order, Right in right-to-left.
                    assert_eq!(via.map(|(arrow, _)| arrow), Some(if rtl { Arrow::Right } else { Arrow::Left }), "{case} {dictated:?} after={after:?}");
                    assert_eq!(now, format!("X {fixed}{after}"), "{case}");
                }
                // Something typed after it: untouched, caret where it was.
                for after in ["", " tail"] {
                    let before = format!("X {old} more");
                    let (via, now, sel) = run(edit, &before, after, old, fixed);
                    println!("{case} typed-after after={after:?} caret={sel:?} (was {})", at(&before));
                    assert_eq!(via, None, "{case}");
                    assert_eq!(now, format!("{before}{after}"), "{case}");
                    assert_eq!(sel, (at(&before), at(&before)), "{case} after={after:?}");
                }
                unsafe { DestroyWindow(GetParent(edit).unwrap()).unwrap() };
            }
        }
    }
}
