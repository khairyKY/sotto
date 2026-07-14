//! Post-ASR text cleanup, applied between transcription and injection.
//!
//! Two tiers, matching the plan's "smart tiered" polish:
//!
//! * **Tier 0 (rules)** — always available, runs in well under a millisecond,
//!   uses no models and no GPU. Conservative and idempotent: it only removes
//!   unambiguous filler words, normalizes whitespace, and capitalizes the first
//!   letter. Parakeet already emits punctuation and casing, so Tier 0
//!   deliberately does *not* try to re-punctuate.
//! * **Tier 1 (AI)** — for longer dictations, hands the text to a local LLM for
//!   Wispr-Flow-style rewriting. Not wired yet (Phase 2b); the seam is here and
//!   currently falls back to Tier 0 so behavior is already correct.

use crate::config::{LlmConfig, PolishMode};
use crate::llm::Llm;
use crate::Controls;
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
        Self { controls, llm }
    }

    fn mode(&self) -> PolishMode {
        PolishMode::from_u8(self.controls.polish_mode.load(Ordering::Relaxed))
    }

    fn ai_min_words(&self) -> usize {
        self.controls.ai_min_words.load(Ordering::Relaxed)
    }

    /// Clean up `raw` according to the configured mode. Never fails: the worst
    /// case is returning the trimmed raw transcript, so a dictation is never
    /// lost to a polish error.
    pub fn polish(&self, raw: &str) -> String {
        let cleaned = match self.mode() {
            PolishMode::Off => raw.trim().to_string(),
            PolishMode::Rules => tier0(raw),
            PolishMode::Ai => self.polish_ai(raw),
        };
        // Dictionary / snippet replacements apply on top of every tier.
        let dict = self.controls.dictionary.lock().unwrap();
        if dict.is_empty() {
            cleaned
        } else {
            apply_dictionary(&cleaned, &dict)
        }
    }

    /// Pre-spawn the LLM sidecar if AI polish is the active tier, so its model
    /// load overlaps recording + transcription rather than blocking afterward.
    /// No-op in Off/Rules mode or when the sidecar isn't available.
    pub fn prewarm(&self) {
        if self.mode() == PolishMode::Ai {
            if let Some(llm) = &self.llm {
                llm.prewarm();
            }
        }
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
}

fn word_count(s: &str) -> usize {
    s.split_whitespace().count()
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
/// inside "arrows".
fn apply_dictionary(text: &str, dict: &[(String, String)]) -> String {
    let mut out = text.to_string();
    for (spoken, replacement) in dict {
        if !spoken.trim().is_empty() {
            out = replace_whole_ci(&out, spoken, replacement);
        }
    }
    out
}

fn replace_whole_ci(hay: &str, needle: &str, rep: &str) -> String {
    let hay_lc = hay.to_ascii_lowercase();
    let needle_lc = needle.to_ascii_lowercase();
    let hb = hay_lc.as_bytes();
    let is_word = |b: u8| b.is_ascii_alphanumeric();

    let mut out = String::with_capacity(hay.len());
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
    out
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

    #[test]
    fn dictionary_replaces_whole_phrases_case_insensitively() {
        let dict = vec![
            ("gee pee tee".to_string(), "GPT".to_string()),
            ("arrow".to_string(), "→".to_string()),
        ];
        assert_eq!(apply_dictionary("use Gee Pee Tee now", &dict), "use GPT now");
        assert_eq!(apply_dictionary("arrow key", &dict), "→ key");
        // Whole-word only: "arrows" must not become "→s".
        assert_eq!(apply_dictionary("two arrows here", &dict), "two arrows here");
        // No entries → untouched.
        assert_eq!(apply_dictionary("nothing", &[]), "nothing");
    }
}
