//! Transforms (#18): select text in any app, press a chord, and the local model
//! rewrites the selection in place.
//!
//! The selection is grabbed with a real Ctrl+C, so the one correctness trap is
//! the user's clipboard: whatever was on it before the chord must be on it
//! after, whether the rewrite landed, was refused, or nothing was selected.
//! `run` owns that ordering; `main.rs`'s `run_transform` only feeds it the real
//! clipboard, keys and model, and shows the outcome on the pill.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// How a Transform ended — each one is a pill state.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The rewrite replaced the selection.
    Replaced,
    /// The copy came back empty: no selection.
    NothingSelected,
    /// The selection was left as it was: the model failed or was refused, or
    /// the copy or paste itself failed.
    Kept,
}

/// What `run` needs from a clipboard. `SystemClipboard` is the real one; the
/// tests use a fake that records the order of every call.
pub trait Clip {
    type Saved;
    fn save(&mut self) -> Self::Saved;
    fn restore(&mut self, saved: Self::Saved);
    fn clear(&mut self);
    /// The text a copy put here, waiting briefly for the app to deliver it.
    fn copied_text(&mut self) -> Option<String>;
}

/// One Transform, in the only order that leaves the clipboard as it was:
/// save → clear → copy → read → rewrite → paste → restore. The clear is what
/// tells "nothing selected" apart from "copied the same thing as before": an
/// app with no selection copies nothing, and the old clipboard would otherwise
/// be read back as the selection and pasted over wherever the caret is.
/// `rewrite` returns `None` to keep the selection unchanged.
pub fn run<C: Clip>(
    clip: &mut C,
    copy: impl FnOnce() -> anyhow::Result<()>,
    rewrite: impl FnOnce(&str) -> Option<String>,
    paste: impl FnOnce(&str) -> anyhow::Result<()>,
) -> Outcome {
    let saved = clip.save();
    clip.clear();
    let outcome = grab_and_replace(clip, copy, rewrite, paste);
    clip.restore(saved);
    outcome
}

fn grab_and_replace<C: Clip>(
    clip: &mut C,
    copy: impl FnOnce() -> anyhow::Result<()>,
    rewrite: impl FnOnce(&str) -> Option<String>,
    paste: impl FnOnce(&str) -> anyhow::Result<()>,
) -> Outcome {
    if let Err(err) = copy() {
        tracing::warn!(error = %err, "transform: copy failed");
        return Outcome::Kept;
    }
    let Some(selection) = clip.copied_text().filter(|s| !s.trim().is_empty()) else {
        return Outcome::NothingSelected;
    };
    let Some(out) = rewrite(&selection) else {
        return Outcome::Kept;
    };
    match paste(&out) {
        Ok(()) => Outcome::Replaced,
        Err(err) => {
            tracing::warn!(error = %err, "transform: paste failed");
            Outcome::Kept
        }
    }
}

/// Least share of the original's words a `keep_words` rewrite must keep.
/// Measured on the sidecar model: translations and summaries keep 0-20%, a
/// good polish that swaps a few words for synonyms keeps 55-80% (0.6 refused
/// one). ponytail: a fixed bar; a config knob if Polish proves too strict.
const KEEP_SHARE: f32 = 0.5;

/// The text to paste in place of `original`, or `None` to keep it. Empty
/// output is never pasted. With `keep_words` (Polish) the #65 lesson applies:
/// asked to tighten text, a small model will sometimes summarise it, drop a
/// clause, or pad it with its own, so a rewrite that keeps under KEEP_SHARE of
/// the original's words, or runs past twice its length, is refused.
/// Whitespace around the selection (a copied line's newline) is carried over,
/// so the paste doesn't join it to the next line.
pub fn accept(original: &str, rewritten: &str, keep_words: bool) -> Option<String> {
    let out = rewritten.trim();
    if out.is_empty() {
        return None;
    }
    if keep_words {
        let (n, m) = (original.split_whitespace().count(), out.split_whitespace().count());
        let share = kept_share(original, out);
        if share < KEEP_SHARE || m > n * 2 + 10 {
            tracing::info!(share, words = n, out_words = m, "transform: rewrite strayed from the original — kept it");
            return None;
        }
    }
    let lead = &original[..original.len() - original.trim_start().len()];
    let trail = &original[original.trim_end().len()..];
    Some(format!("{lead}{out}{trail}"))
}

/// Share of `original`'s words (3+ letters, case and punctuation aside) still
/// present in `rewritten`. 1.0 when there's nothing to measure.
fn kept_share(original: &str, rewritten: &str) -> f32 {
    let words = |s: &str| -> HashSet<String> {
        s.split_whitespace()
            .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase())
            .filter(|w| w.chars().count() >= 3)
            .collect()
    };
    let (a, b) = (words(original), words(rewritten));
    if a.is_empty() {
        return 1.0;
    }
    a.iter().filter(|w| b.contains(*w)).count() as f32 / a.len() as f32
}

/// `--transform "<text>"`: every configured Transform over `text` through the
/// real sidecar, guard included. Checks the prompts without the app, like
/// `--polish`; nothing is copied or pasted.
pub fn run_once(text: &str) -> anyhow::Result<()> {
    let cfg = crate::config::Config::load_or_init().unwrap_or_default();
    let llm = crate::llm::Llm::new(cfg.llm.clone());
    println!("text => {text:?}");
    for t in &cfg.transforms {
        let started = Instant::now();
        let out = llm.rewrite(text, &t.prompt)?;
        let ms = started.elapsed().as_millis();
        match accept(text, &out, t.keep_words) {
            Some(_) => println!("{} => {out:?}  ({ms} ms)", t.name),
            None => println!("{} => KEPT the original, refused {out:?}  ({ms} ms)", t.name),
        }
    }
    Ok(())
}

/// The system clipboard. Saves the one richest thing on it: files, then rich
/// text (with its plain-text twin), then an image, then plain text.
/// ponytail: one format per restore; a clipboard holding several (say, a
/// browser's copied image plus its HTML) comes back as the first of those.
pub struct SystemClipboard(arboard::Clipboard);

pub enum Saved {
    Files(Vec<PathBuf>),
    Html(String, Option<String>),
    Image(arboard::ImageData<'static>),
    Text(String),
    Empty,
}

impl SystemClipboard {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self(arboard::Clipboard::new()?))
    }
}

impl Clip for SystemClipboard {
    type Saved = Saved;

    fn save(&mut self) -> Saved {
        let cb = &mut self.0;
        if let Ok(files) = cb.get().file_list() {
            if !files.is_empty() {
                return Saved::Files(files);
            }
        }
        if let Ok(html) = cb.get().html() {
            return Saved::Html(html, cb.get_text().ok());
        }
        if let Ok(img) = cb.get_image() {
            return Saved::Image(img);
        }
        cb.get_text().map_or(Saved::Empty, Saved::Text)
    }

    fn restore(&mut self, saved: Saved) {
        let cb = &mut self.0;
        let restored = match saved {
            Saved::Files(f) => cb.set().file_list(f.as_slice()),
            Saved::Html(html, alt) => cb.set_html(html, alt),
            Saved::Image(img) => cb.set_image(img),
            Saved::Text(t) => cb.set_text(t),
            Saved::Empty => cb.clear(),
        };
        if let Err(err) = restored {
            tracing::warn!(error = %err, "transform: couldn't restore the clipboard");
        }
    }

    fn clear(&mut self) {
        let _ = self.0.clear();
    }

    /// Up to 500 ms: the spike measured 9-23 ms in Notepad, VS Code and Edge,
    /// the rest is headroom for a busy machine.
    fn copied_text(&mut self) -> Option<String> {
        let t = Instant::now();
        loop {
            if let Ok(s) = self.0.get_text() {
                return Some(s);
            }
            if t.elapsed() >= Duration::from_millis(500) {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A clipboard plus the log of everything done to it or around it.
    struct Board {
        text: Option<String>,
        log: Vec<&'static str>,
    }
    struct Fake<'a>(&'a RefCell<Board>);
    impl Clip for Fake<'_> {
        type Saved = Option<String>;
        fn save(&mut self) -> Option<String> {
            let mut b = self.0.borrow_mut();
            b.log.push("save");
            b.text.clone()
        }
        fn restore(&mut self, saved: Option<String>) {
            let mut b = self.0.borrow_mut();
            b.log.push("restore");
            b.text = saved;
        }
        fn clear(&mut self) {
            let mut b = self.0.borrow_mut();
            b.log.push("clear");
            b.text = None;
        }
        fn copied_text(&mut self) -> Option<String> {
            let mut b = self.0.borrow_mut();
            b.log.push("read");
            b.text.clone()
        }
    }

    /// Run one Transform against `board`: Ctrl+C puts `selection` on it (if
    /// any), the model answers `model`, and the paste succeeds unless
    /// `paste_fails`. Returns the outcome and what was pasted.
    fn transform(
        board: &RefCell<Board>,
        selection: Option<&str>,
        model: Option<&str>,
        paste_fails: bool,
    ) -> (Outcome, Option<String>) {
        let pasted = RefCell::new(None);
        let outcome = run(
            &mut Fake(board),
            || {
                let mut b = board.borrow_mut();
                b.log.push("copy");
                if let Some(s) = selection {
                    b.text = Some(s.to_string());
                }
                Ok(())
            },
            |text| {
                board.borrow_mut().log.push("rewrite");
                assert_eq!(Some(text), selection);
                model.map(str::to_string)
            },
            |out| {
                board.borrow_mut().log.push("paste");
                *pasted.borrow_mut() = Some(out.to_string());
                if paste_fails { anyhow::bail!("blocked") } else { Ok(()) }
            },
        );
        (outcome, pasted.into_inner())
    }

    fn board(text: Option<&str>) -> RefCell<Board> {
        RefCell::new(Board { text: text.map(str::to_string), log: vec![] })
    }

    #[test]
    fn a_rewrite_lands_and_the_old_clipboard_comes_back_last() {
        let b = board(Some("an address the user copied earlier"));
        let (outcome, pasted) = transform(&b, Some("teh meeting moved"), Some("The meeting moved."), false);
        assert_eq!(outcome, Outcome::Replaced);
        assert_eq!(pasted.as_deref(), Some("The meeting moved."));
        let b = b.into_inner();
        assert_eq!(b.log, ["save", "clear", "copy", "read", "rewrite", "paste", "restore"]);
        assert_eq!(b.text.as_deref(), Some("an address the user copied earlier"));
    }

    #[test]
    fn nothing_selected_never_pastes_the_old_clipboard_back_as_the_selection() {
        // The trap the clear exists for: with no selection, Ctrl+C copies
        // nothing, and without the clear the old clipboard would read back
        // as "the selection" and get rewritten into the document.
        let b = board(Some("an address the user copied earlier"));
        let (outcome, pasted) = transform(&b, None, Some("should never run"), false);
        assert_eq!(outcome, Outcome::NothingSelected);
        assert_eq!(pasted, None);
        let b = b.into_inner();
        assert_eq!(b.log, ["save", "clear", "copy", "read", "restore"]);
        assert_eq!(b.text.as_deref(), Some("an address the user copied earlier"));
    }

    #[test]
    fn a_refused_rewrite_keeps_the_selection_and_restores_the_clipboard() {
        let b = board(Some("earlier copy"));
        let (outcome, pasted) = transform(&b, Some("some selected text"), None, false);
        assert_eq!(outcome, Outcome::Kept);
        assert_eq!(pasted, None);
        let b = b.into_inner();
        assert_eq!(b.log.last(), Some(&"restore"));
        assert_eq!(b.text.as_deref(), Some("earlier copy"));
    }

    #[test]
    fn a_failed_paste_or_copy_still_restores_the_clipboard() {
        let b = board(Some("earlier copy"));
        assert_eq!(transform(&b, Some("some text"), Some("Some text."), true).0, Outcome::Kept);
        assert_eq!(b.borrow().text.as_deref(), Some("earlier copy"));

        let b = board(Some("earlier copy"));
        let outcome = run(&mut Fake(&b), || anyhow::bail!("keys held"), |_| unreachable!(), |_| unreachable!());
        assert_eq!(outcome, Outcome::Kept);
        let b = b.into_inner();
        assert_eq!(b.log, ["save", "clear", "restore"]);
        assert_eq!(b.text.as_deref(), Some("earlier copy"));
    }

    #[test]
    fn an_empty_clipboard_comes_back_empty_not_holding_the_selection() {
        let b = board(None);
        transform(&b, Some("some selected text"), Some("Some selected text."), false);
        assert_eq!(b.borrow().text, None);
    }

    #[test]
    fn empty_output_is_never_pasted() {
        assert_eq!(accept("some text", "", false), None);
        assert_eq!(accept("some text", "  \n ", true), None);
    }

    #[test]
    fn keep_words_accepts_a_faithful_tightening() {
        let original = "so basically the meeting got moved to thursday because the room was booked";
        let out = accept(original, "The meeting moved to Thursday because the room was booked.", true);
        assert_eq!(out.as_deref(), Some("The meeting moved to Thursday because the room was booked."));
        // A real sidecar polish that swaps words for synonyms (58% kept): the
        // 0.6 bar refused it, which is why the bar is 0.5.
        let original = "can you check if the invoice went out i think i sent it but im not sure";
        let polished = "Can you confirm if the invoice was sent? I think I might have sent it, but I'm unsure.";
        assert!(accept(original, polished, true).is_some());
    }

    #[test]
    fn keep_words_refuses_a_summary_or_a_padded_rewrite() {
        let original = "so basically the meeting got moved to thursday because the room was booked";
        // Summarised: most of the wording gone (#65).
        assert_eq!(accept(original, "Meeting: Thursday.", true), None);
        // Padded: the model added a paragraph of its own.
        let padded = format!("{original}. {}", "Here is some extra context the writer never wrote at all. ".repeat(3));
        assert_eq!(accept(original, &padded, true), None);
        // Without keep_words (Prompt engineer, a custom one) both are fine.
        assert!(accept(original, "Meeting: Thursday.", false).is_some());
    }

    #[test]
    fn keep_words_measures_arabic_and_code_switched_text_too() {
        let original = "الاجتماع اتأجل ليوم الخميس عشان الـ room كانت محجوزة";
        let faithful = "الاجتماع اتأجل ليوم الخميس، عشان الـ room كانت محجوزة.";
        assert_eq!(accept(original, faithful, true).as_deref(), Some(faithful));
        assert_eq!(accept(original, "The meeting moved.", true), None); // translated away
    }

    #[test]
    fn whitespace_around_the_selection_is_carried_over() {
        assert_eq!(accept("  teh line\r\n", "The line.", false).as_deref(), Some("  The line.\r\n"));
    }
}
