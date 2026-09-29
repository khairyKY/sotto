//! Voice correction (#20): right after a dictation lands, saying "correction:
//! Claude, not clawed" swaps the misheard word in that text, types the fixed
//! text back over it, and teaches the pair to the vocabulary. The command
//! itself is never typed.
//!
//! The one correctness trap is typing over the wrong text: by the time the
//! command arrives the caret may have moved. `main.rs`'s `run_correction`
//! only retypes after a copy of the re-selected span proves it is that
//! dictation, unchanged (`same_text`); anything else leaves the document
//! alone and puts the fix on the clipboard.
//!
//! Backtrack (#26) shares the memory and the guard: "scratch that" deletes
//! the last dictation, once the same copy proves it's still there.

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

/// Right presses that put the caret back after a retype that didn't happen
/// (#96). One collapses whatever Shift+Left selected to its end, which is
/// where the caret was: a span that isn't the dictation (`Kept`), and one the
/// copy came back empty from (`NothingSelected`, an app without Ctrl+Insert or
/// a slow clipboard). Shift+Left selects in every text field, and the console
/// line editors where it only moves the caret skip the key path
/// (`is_terminal`). ponytail: an unknown field that ignores Shift is left
/// with the caret n-1 back; walking it back × n would instead push every
/// Ctrl+Insert failure n-1 past the caret, and Shift+Right × n can leave a
/// live selection the next dictation types over.
pub fn caret_back(outcome: &crate::transform::Outcome) -> usize {
    match outcome {
        crate::transform::Outcome::Replaced => 0,
        crate::transform::Outcome::Kept | crate::transform::Outcome::NothingSelected => 1,
    }
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
    fn a_retype_that_didnt_happen_collapses_the_selection_to_the_caret() {
        use crate::transform::Outcome;
        assert_eq!(caret_back(&Outcome::Replaced), 0);
        assert_eq!(caret_back(&Outcome::Kept), 1);
        // Shift+Left selected, the copy came back empty: one Right, not × n (#96).
        assert_eq!(caret_back(&Outcome::NothingSelected), 1);
    }
}
