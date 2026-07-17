//! Post-ASR text cleanup, applied between transcription and injection.
//!
//! Two tiers, matching the plan's "smart tiered" polish:
//!
//! * **Tier 0 (rules)** — always available, runs in well under a millisecond,
//!   uses no models and no GPU. Conservative and idempotent: it only removes
//!   unambiguous filler words, normalizes whitespace, and capitalizes the first
//!   letter. Parakeet already emits punctuation and casing, so Tier 0
//!   deliberately does *not* try to re-punctuate. On top of that it runs
//!   `harper-core`'s offline grammar checker, restricted to mechanical,
//!   single-suggestion lints (see `SAFE_LINT_KINDS`) — still instant, still
//!   never a guess at what the user meant.
//! * **Tier 1 (AI)** — for longer dictations, hands the text to a local LLM for
//!   Wispr-Flow-style rewriting. Not wired yet (Phase 2b); the seam is here and
//!   currently falls back to Tier 0 so behavior is already correct.

use crate::config::{LlmConfig, PolishMode};
use crate::llm::Llm;
use crate::Controls;
use harper_core::linting::{Lint, LintGroup, LintKind, Linter};
use harper_core::spell::FstDictionary;
use harper_core::{Dialect, Document, remove_overlaps};
use std::cell::RefCell;
use std::sync::atomic::Ordering;

/// Unambiguous spoken disfluencies. Kept intentionally short: every entry here
/// is something a user essentially never means to keep in written text.
/// Ambiguous ones ("like", "so", "you know", "well") are excluded on purpose —
/// removing them changes meaning too often.
const FILLERS: &[&str] = &["um", "uh", "uhh", "umm", "uhm", "erm", "er", "hmm"];

pub struct Polisher {
    /// Live tier, threshold, and dictionary — all editable from the tray /
    /// settings window without a restart.
    controls: Controls,
    llm: Option<Llm>,
    /// Harper's curated lint set — ~250 rule structs plus a lazily-loaded
    /// dictionary FST, expensive to build (see `apply_harper`). Built once,
    /// on first use, and reused for every dictation after. `RefCell` because
    /// `lint()` needs `&mut self` but `Polisher::polish` only gets `&self`;
    /// plain (not `Mutex`) because this only ever runs on the single
    /// dictation worker thread — never touched concurrently.
    harper: RefCell<Option<LintGroup>>,
}

impl Polisher {
    /// The LLM handle is built whenever the model is present — regardless of the
    /// starting tier — because it spawns lazily on first use (zero idle VRAM),
    /// so switching Polish to AI from the tray works without a restart.
    pub fn new(controls: Controls, llm_cfg: LlmConfig) -> Self {
        let llm = if Llm::is_available() {
            tracing::info!("AI polish available — llama.cpp sidecar ready (spawns on first use)");
            Some(Llm::new(llm_cfg))
        } else {
            tracing::info!(
                "AI polish unavailable — model or llama-server missing; rules-only until installed"
            );
            None
        };
        Self { controls, llm, harper: RefCell::new(None) }
    }

    pub fn mode(&self) -> PolishMode {
        PolishMode::from_u8(self.controls.polish_mode.load(Ordering::Relaxed))
    }

    fn ai_min_words(&self) -> usize {
        self.controls.ai_min_words.load(Ordering::Relaxed)
    }

    /// Clean up `raw` according to the configured mode. Never fails: the worst
    /// case is returning the trimmed raw transcript, so a dictation is never
    /// lost to a polish error. Alongside the text, reports how many words the
    /// cleanup changed and how many dictionary replacements fired — the
    /// "fixes made by Sotto" numbers on the Insights dashboard.
    pub fn polish(&self, raw: &str) -> PolishResult {
        let cleaned = match self.mode() {
            PolishMode::Off => raw.trim().to_string(),
            PolishMode::Rules => self.apply_harper(&tier0(raw)),
            PolishMode::Ai => self.polish_ai(raw),
        };
        // Diff BEFORE the dictionary pass so corrected-words and dict-hits
        // don't double-count the same word.
        let corrected_words = changed_words(raw, &cleaned);
        // Dictionary / snippet replacements apply on top of every tier.
        let dict = self.controls.dictionary.lock().unwrap();
        let (text, dict_hits) = if dict.is_empty() {
            (cleaned, 0)
        } else {
            apply_dictionary(&cleaned, &dict)
        };
        PolishResult { text, corrected_words, dict_hits }
    }

    /// Pre-spawn the LLM sidecar so its load overlaps recording. Called when
    /// dictation starts, not at launch — the sidecar holds VRAM and gets
    /// idle-killed after `idle_kill_secs`, so keeping it resident from boot
    /// would defeat that. No-op outside the AI tier.
    pub fn prewarm(&self) {
        if self.mode() == PolishMode::Ai {
            if let Some(llm) = &self.llm {
                llm.prewarm();
            }
        }
    }

    /// Build Harper's curated lint set (~640 ms) ahead of time.
    ///
    /// Called once at worker startup, where it hides behind the ASR model load
    /// we already pay for and the user feels nothing. Doing it lazily instead
    /// costs that 640 ms at the worst possible moment: on `Start` it delays the
    /// listening pill past the hotkey press, and in the AI tier — which still
    /// routes short clips through these rules — it lands *after* the user has
    /// already spoken.
    ///
    /// Warmed regardless of the current mode: `polish.mode` is live-switchable
    /// from the tray, so "Off at launch" doesn't mean off at dictation time.
    pub fn warm_rules(&self) {
        let t = std::time::Instant::now();
        self.apply_harper("warm up");
        tracing::info!(warm_ms = t.elapsed().as_millis(), "Harper lint set ready");
    }

    /// True if `raw` would actually be sent through the LLM (AI mode selected,
    /// sidecar available, and long enough to clear the word threshold). Lets the
    /// overlay show the "Polishing" state only when a real AI pass will run.
    pub fn uses_ai_tier(&self, raw: &str) -> bool {
        self.mode() == PolishMode::Ai
            && self.llm.is_some()
            && word_count(raw) >= self.ai_min_words()
    }

    /// Tier 1: route long-enough dictations through the LLM, falling back to
    /// Tier 0 rules for short clips, a missing sidecar, or any LLM error.
    fn polish_ai(&self, raw: &str) -> String {
        let rules = tier0(raw);

        if word_count(raw) < self.ai_min_words() {
            return rules; // too short to be worth the round-trip
        }
        // Quality gate: if the rules pass didn't change anything (no fillers
        // to remove, spacing already clean), the transcript is already tidy —
        // the LLM would only introduce lossy paraphrasing. Skipping here also
        // saves 300-500ms on already-clean speech, which is the common case
        // once a user learns to speak fluently to the app.
        if rules.trim_end_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace())
            == raw.trim().trim_end_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace())
        {
            tracing::debug!("polish: skipped AI — rules pass was a no-op");
            return rules;
        }
        let Some(llm) = &self.llm else { return rules };

        let t = std::time::Instant::now();
        match llm.polish(raw) {
            Ok(text) if !text.trim().is_empty() => {
                tracing::info!(llm_ms = t.elapsed().as_millis(), "AI polish applied");
                text
            }
            Ok(_) => {
                tracing::warn!("AI polish returned empty text — using rules");
                rules
            }
            Err(err) => {
                tracing::warn!(error = %err, "AI polish failed — using rules");
                rules
            }
        }
    }

    /// Run Harper over `text` on the cached, lazily-built `LintGroup`.
    fn apply_harper(&self, text: &str) -> String {
        if text.trim().is_empty() {
            return text.to_string();
        }
        let mut slot = self.harper.borrow_mut();
        let linter = slot
            .get_or_insert_with(|| LintGroup::new_curated(FstDictionary::curated(), Dialect::American));
        run_harper(linter, text)
    }
}

/// Run Harper's curated lint set over `text` and apply only the unambiguous
/// fixes: mechanical `LintKind`s (see `SAFE_LINT_KINDS`) with exactly one
/// suggestion. A lint with 2-3 candidate spellings is a guess at what the
/// user said, so it's left alone rather than picking `[0]`. Split out of
/// `Polisher::apply_harper` so tests can drive it without building a full
/// `Polisher`.
fn run_harper(linter: &mut LintGroup, text: &str) -> String {
    let doc = Document::new_plain_english_curated(text);
    let mut lints = linter.lint(&doc);
    remove_overlaps(&mut lints); // drops overlapping spans, keeps higher priority

    let mut fixes: Vec<&Lint> = lints
        .iter()
        .filter(|l| SAFE_LINT_KINDS.contains(&l.lint_kind) && l.suggestions.len() == 1)
        .collect();
    // Back-to-front so an earlier edit can't shift a later span.
    fixes.sort_by(|a, b| b.span.start.cmp(&a.span.start));

    let mut chars: Vec<char> = text.chars().collect();
    for lint in fixes {
        lint.suggestions[0].apply(lint.span, &mut chars);
    }
    chars.into_iter().collect()
}

/// Mechanical `LintKind`s safe to auto-apply without a human glancing at
/// them — spelling/typo/casing/punctuation/repetition slips a user never
/// means to keep. Deliberately excludes opinionated kinds (`Style`,
/// `Enhancement`, `WordChoice`, `Readability`, `Usage`, `Regionalism`, ...)
/// that would rewrite meaning rather than fix a mechanical slip.
const SAFE_LINT_KINDS: &[LintKind] = &[
    LintKind::Capitalization,
    LintKind::Punctuation,
    LintKind::Repetition,
    LintKind::Spelling,
    LintKind::Typo,
    LintKind::BoundaryError,
];

/// What a polish pass produced: the final text plus the fix counts the
/// Insights dashboard aggregates (see stats.rs).
pub struct PolishResult {
    pub text: String,
    /// Words the cleanup tier changed vs. the raw transcript (fillers
    /// removed, self-corrections resolved, casing/punctuation edits).
    pub corrected_words: usize,
    /// Dictionary / snippet replacements that fired.
    pub dict_hits: usize,
}

fn word_count(s: &str) -> usize {
    s.split_whitespace().count()
}

/// How many words differ between `a` and `b`, as `max(len) - LCS` over
/// punctuation-stripped, lowercased tokens. Dictations are at most a few
/// hundred words, so the O(n·m) table is trivially cheap.
pub fn changed_words(a: &str, b: &str) -> usize {
    let norm = |s: &str| -> Vec<String> {
        s.split_whitespace()
            .map(|t| t.trim_matches(|c: char| !c.is_alphanumeric()).to_ascii_lowercase())
            .filter(|t| !t.is_empty())
            .collect()
    };
    let aw = norm(a);
    let bw = norm(b);
    let (n, m) = (aw.len(), bw.len());
    if n == 0 || m == 0 {
        return n.max(m);
    }
    let mut dp = vec![0usize; m + 1];
    for i in 1..=n {
        let mut prev = 0; // dp[i-1][j-1]
        for j in 1..=m {
            let tmp = dp[j];
            dp[j] = if aw[i - 1] == bw[j - 1] { prev + 1 } else { dp[j].max(dp[j - 1]) };
            prev = tmp;
        }
    }
    n.max(m) - dp[m]
}

/// Tier 0 rules cleanup. `split_whitespace` also collapses runs of spaces and
/// trims, so filtering + rejoining handles whitespace normalization for free.
fn tier0(raw: &str) -> String {
    let kept: Vec<&str> = raw.split_whitespace().filter(|t| !is_filler(t)).collect();
    capitalize_first(&kept.join(" "))
}

/// True if `token`, stripped of surrounding punctuation and lowercased, is a
/// filler word.
fn is_filler(token: &str) -> bool {
    let core = token.trim_matches(|c: char| !c.is_alphanumeric());
    if core.is_empty() {
        return false;
    }
    let lower = core.to_ascii_lowercase();
    FILLERS.contains(&lower.as_str())
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Apply each `spoken → replacement` entry as a case-insensitive, whole-phrase
/// substitution. ASCII-folded so byte indices stay aligned (dictionary terms
/// are effectively ASCII), and word-boundary-checked so "arrow" doesn't hit
/// inside "arrows". Returns the rewritten text plus how many replacements
/// fired (the "dictionary fixes" stat).
fn apply_dictionary(text: &str, dict: &[(String, String)]) -> (String, usize) {
    let mut out = text.to_string();
    let mut hits = 0;
    for (spoken, replacement) in dict {
        if !spoken.trim().is_empty() {
            let (next, n) = replace_whole_ci(&out, spoken, replacement);
            out = next;
            hits += n;
        }
    }
    (out, hits)
}

fn replace_whole_ci(hay: &str, needle: &str, rep: &str) -> (String, usize) {
    let hay_lc = hay.to_ascii_lowercase();
    let needle_lc = needle.to_ascii_lowercase();
    let hb = hay_lc.as_bytes();
    let is_word = |b: u8| b.is_ascii_alphanumeric();

    let mut out = String::with_capacity(hay.len());
    let mut count = 0;
    let mut i = 0;
    while i <= hay_lc.len() {
        match hay_lc[i..].find(&needle_lc) {
            Some(rel) => {
                let start = i + rel;
                let end = start + needle_lc.len();
                let left_ok = start == 0 || !is_word(hb[start - 1]);
                let right_ok = end == hb.len() || !is_word(hb[end]);
                if left_ok && right_ok {
                    out.push_str(&hay[i..start]);
                    out.push_str(rep);
                    count += 1;
                    i = end;
                } else {
                    // Boundary failed — emit one char and keep scanning.
                    let ch_len = hay[start..].chars().next().map_or(1, |c| c.len_utf8());
                    out.push_str(&hay[i..start + ch_len]);
                    i = start + ch_len;
                }
            }
            None => {
                out.push_str(&hay[i..]);
                break;
            }
        }
    }
    (out, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(s: &str) -> String {
        tier0(s)
    }

    #[test]
    fn removes_fillers_and_tidies() {
        assert_eq!(rules("um so uh the plan is good"), "So the plan is good");
    }

    #[test]
    fn collapses_whitespace() {
        assert_eq!(rules("hello    world"), "Hello world");
    }

    #[test]
    fn keeps_ambiguous_words() {
        // "like" and "so" must survive — they carry meaning.
        assert_eq!(rules("I like it, so yes"), "I like it, so yes");
    }

    #[test]
    fn strips_filler_with_trailing_punctuation() {
        assert_eq!(rules("Um, let's go"), "Let's go");
    }

    #[test]
    fn idempotent() {
        let once = rules("um hello there");
        assert_eq!(rules(&once), once);
    }

    #[test]
    fn empty_stays_empty() {
        assert_eq!(rules("   "), "");
    }

    /// Fresh `LintGroup` per call — simpler than sharing one across tests,
    /// and construction cost is a non-issue for a handful of test cases (see
    /// `Polisher::apply_harper` for the real, cached-once path).
    fn harper(s: &str) -> String {
        let mut linter = LintGroup::new_curated(FstDictionary::curated(), Dialect::American);
        run_harper(&mut linter, s)
    }

    #[test]
    fn harper_removes_doubled_word() {
        // Repetition lint, exactly one suggestion ("the") — unambiguous.
        assert_eq!(harper("I went to the the store."), "I went to the store.");
    }

    #[test]
    fn harper_leaves_ambiguous_spelling_alone() {
        // "recieve" gets 3 candidate corrections (receive/relieve/recipe) —
        // picking [0] would be a guess at what the user actually said.
        let s = "I recieve packages daily.";
        assert_eq!(harper(s), s);
    }

    #[test]
    fn harper_runs_after_filler_stripping() {
        // Tier 0 (filler + capitalization) still runs, and Harper's fix
        // applies on top of its output — the two tiers compose.
        let cleaned = tier0("um I went to the the store");
        assert_eq!(harper(&cleaned), "I went to the store");
    }

    #[test]
    fn dictionary_replaces_whole_phrases_case_insensitively() {
        let dict = vec![
            ("gee pee tee".to_string(), "GPT".to_string()),
            ("arrow".to_string(), "→".to_string()),
        ];
        assert_eq!(apply_dictionary("use Gee Pee Tee now", &dict), ("use GPT now".into(), 1));
        assert_eq!(apply_dictionary("arrow key", &dict), ("→ key".into(), 1));
        // Whole-word only: "arrows" must not become "→s".
        assert_eq!(apply_dictionary("two arrows here", &dict), ("two arrows here".into(), 0));
        // No entries → untouched.
        assert_eq!(apply_dictionary("nothing", &[]), ("nothing".into(), 0));
    }

    #[test]
    fn changed_words_counts_edits_not_reorderings_of_identical_text() {
        // Removing one filler = 1 change.
        assert_eq!(changed_words("um hello there", "Hello there"), 1);
        // Identical after case/punct normalization = 0 changes.
        assert_eq!(changed_words("hello there", "Hello, there."), 0);
        // Word substitution = 1.
        assert_eq!(changed_words("ship the crate", "ship the create"), 1);
        // Empty raw vs text.
        assert_eq!(changed_words("", "three new words"), 3);
    }
}
