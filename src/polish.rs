//! Post-ASR text cleanup, applied between transcription and injection.
//!
//! Two tiers, matching the plan's "smart tiered" polish:
//!
//! * **Tier 0 (rules)** — always available, runs in well under a millisecond,
//!   uses no models and no GPU. Conservative and idempotent: it only removes
//!   unambiguous filler words, collapses back-to-back word repeats
//!   ("uh uh uh", "the the store"), normalizes whitespace, and capitalizes
//!   the first letter. Parakeet already emits punctuation and casing, so
//!   Tier 0 deliberately does *not* try to re-punctuate. On top of that it runs
//!   `harper-core`'s offline grammar checker, restricted to mechanical,
//!   single-suggestion lints (see `SAFE_LINT_KINDS`) — still instant, still
//!   never a guess at what the user meant.
//! * **Tier 1 (AI)** — for longer dictations, hands the text to a local LLM for
//!   Wispr-Flow-style rewriting. Not wired yet (Phase 2b); the seam is here and
//!   currently falls back to Tier 0 so behavior is already correct.

use crate::config::{EntryKind, LlmConfig, PolishMode, VocabEntry};
use crate::llm::Llm;
use crate::Controls;
use harper_core::linting::{Lint, LintGroup, LintKind, Linter, Suggestion};
use harper_core::spell::{Dictionary, FstDictionary};
use harper_core::{Dialect, Document, remove_overlaps};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::Ordering;

/// Unambiguous spoken disfluencies. Kept intentionally short: every entry here
/// is something a user essentially never means to keep in written text.
/// Ambiguous ones ("like", "so", "you know", "well") are excluded on purpose —
/// removing them changes meaning too often.
const FILLERS: &[&str] =
    &["um", "uh", "uhh", "umm", "uhm", "erm", "er", "hmm", "ah", "ahh", "eh", "mm", "mmm"];

/// Words exempt from stutter collapse, because doubling them is legitimate
/// often enough that removing the repeat is the bigger error: emphasis
/// ("very very important", "no no no"), and real grammar ("he had had
/// enough", "I know that that book is..."). Same bar as `FILLERS` in
/// reverse — only words where the user plausibly *meant* the repeat.
const LEGIT_DOUBLES: &[&str] = &["very", "really", "so", "no", "yes", "had", "that", "ha"];

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

    /// The AI tier's sidecar handle, lent to Transforms (#18) so they never
    /// spawn a second model. `None` when the model isn't installed.
    pub fn llm(&self) -> Option<&Llm> {
        self.llm.as_ref()
    }

    fn ai_min_words(&self) -> usize {
        self.controls.ai_min_words.load(Ordering::Relaxed)
    }

    /// Clean up `raw` according to the configured mode, using the default
    /// tone (no app context — this is `repolish_copy`'s path, re-running a
    /// history row with only its text). Never fails: the worst case is
    /// returning the trimmed raw transcript, so a dictation is never lost to
    /// a polish error. Alongside the text, reports how many words the
    /// cleanup changed and how many dictionary replacements fired — the
    /// "fixes made by Sotto" numbers on the Insights dashboard.
    pub fn polish(&self, raw: &str) -> PolishResult {
        let tone = self.controls.tone.lock().unwrap().clone();
        self.polish_with_tone(raw, &tone, "")
    }

    /// Same as `polish`, but resolves a tone for `app` first: an exact
    /// case-insensitive match in the per-app overrides wins, else the field's
    /// own text (`context`, auto tone #28), else the default tone, else no
    /// tone at all. Used on the live delivery path, where the focused app is
    /// known.
    pub fn polish_for(&self, raw: &str, app: &str, context: Option<&str>) -> PolishResult {
        let tone = self.resolve_tone(app, context);
        self.polish_with_tone(raw, &tone, app)
    }

    /// Tone-lookup order: per-app override → field context → default tone →
    /// "" (no instruction, prompt stays byte-identical to before tones
    /// existed). The context replaces the default rather than joining it: a
    /// 1.5B model told "formal" while shown a chat thread follows neither.
    fn resolve_tone(&self, app: &str, context: Option<&str>) -> String {
        if !app.is_empty() {
            let app_tones = self.controls.app_tones.lock().unwrap();
            if let Some((_, tone)) = app_tones.iter().find(|(a, _)| a.eq_ignore_ascii_case(app)) {
                return tone.clone();
            }
        }
        match context {
            Some(context) => context_clause(context),
            None => self.controls.tone.lock().unwrap().clone(),
        }
    }

    fn polish_with_tone(&self, raw: &str, tone: &str, app: &str) -> PolishResult {
        // Formatting commands run FIRST, on the raw transcript, before both
        // the mode branch and the dictionary pass below: before the mode
        // branch so the AI tier receives text already broken into paragraphs
        // (see SYSTEM_PROMPT's "preserve existing line breaks"); before the
        // dictionary so a snippet whose *replacement* text happens to contain
        // the words "new line" can never get chopped — the command can only
        // ever match what the user actually said. Its own toggle, independent
        // of polish mode: "new paragraph" always means the break, never the
        // words, so it applies in Off/Rules/Ai alike. Quote commands (F2)
        // share the same toggle and the same reasoning, and run right after
        // line breaks so a literal "new line" spoken inside a quoted span
        // still becomes a real break there too.
        let formatted;
        let raw = if self.controls.formatting_commands.load(Ordering::Relaxed) {
            let style = QuoteStyle::from_config_str(&self.controls.quote_style.lock().unwrap());
            formatted = apply_quote_commands(&apply_formatting_commands(raw), style);
            formatted.as_str()
        } else {
            raw
        };

        // Spoken numbers -> digits, on the pre-mode text so the LLM receives
        // figures it will keep rather than re-verbalize. Own toggle, applies
        // in Off/Rules/Ai alike (same reasoning as formatting commands above).
        let numbered;
        let raw = if self.controls.number_formatting.load(Ordering::Relaxed) {
            numbered = normalize_numbers(raw);
            numbered.as_str()
        } else {
            raw
        };

        // Variable recognition (#27): "camel case user name" -> userName, only
        // in a listed code editor. After numbers, so "version two" can become
        // version2. Each identifier made is masked from here on (see
        // `apply_casing_commands`), so no later pass can respell it: the
        // dictionary, the phonetic corrector, tier0 and Harper only ever see
        // an opaque placeholder, and the AI tier must hand every one back
        // intact (`remask`) or the rules result is kept.
        let cased;
        let mut made: Vec<String> = Vec::new();
        let raw = if !app.is_empty()
            && self.controls.code_editors.lock().unwrap().iter().any(|a| a.eq_ignore_ascii_case(app))
        {
            (cased, made) = apply_casing_commands(raw);
            cased.as_str()
        } else {
            raw
        };

        // Dictionary / snippet replacements apply on top of every tier —
        // unless the master switch is off, in which case nothing fires and
        // the entries are left untouched on disk. Split by kind: `Word`
        // entries (short glossary terms — proper nouns, abbreviations like
        // "E and" -> "e&") run BEFORE polish, so the LLM sees the correct
        // token. Left for AI polish to see the raw phrase, a short fragment
        // like "E and" reads as a dangling conjunction to a grammar-cleanup
        // pass and gets silently dropped — by the time a post-polish
        // dictionary pass ran, the phrase it was matching for no longer
        // existed. `Snippet` entries (longer expansions — emails, addresses,
        // text blocks) stay AFTER polish, unchanged, so grammar rewriting
        // never gets a chance to mangle the expansion itself.
        // A copy (#127): the guard would be held through the AI tier below,
        // and a dictionary saved in Settings meanwhile hangs the window on it.
        let dict = self.controls.dictionary.lock().unwrap().clone();
        let replacements_on = self.controls.replacements_enabled.load(Ordering::Relaxed);
        let worded;
        let (raw, word_hits) = if dict.is_empty() || !replacements_on {
            (raw, 0)
        } else {
            let (fixed, hits) = apply_dictionary(raw, &dict, EntryKind::Word);
            worded = fixed;
            (worded.as_str(), hits)
        };

        // Phonetic correction: after the exact Word-dictionary pass (so it only
        // handles mishearings the exact entries didn't already fix), before the
        // mode branch (so the LLM sees the intended proper noun). Targets are
        // the trained-vocabulary entries, single words and phrases — see
        // `apply_phonetic_corrections`.
        let phoneticized;
        let mut notable: Vec<(String, String)> = Vec::new();
        let raw = if self.controls.phonetic_correction.load(Ordering::Relaxed) {
            let targets: Vec<String> =
                self.controls.vocabulary.lock().unwrap().iter().map(|e| e.word.clone()).collect();
            if targets.is_empty() {
                raw
            } else {
                let (fixed, fired) = apply_phonetic_corrections(raw, &targets);
                notable = fired;
                phoneticized = fixed;
                phoneticized.as_str()
            }
        } else {
            raw
        };

        // The user's own spellings (#113): each trained word and Word-entry
        // replacement in the take is masked like an identifier while the
        // tiers run, so tier0 and Harper can't respell it ("Heliboard" ->
        // "Headboard") and the AI tier has to hand it back as is. It comes
        // back as trained, however it was heard, and before the snippet
        // pass, which may key on it. Off cleans nothing, so masks nothing.
        let idents = made.len();
        let protected;
        let raw = if self.mode() == PolishMode::Off {
            raw
        } else {
            let mut terms: Vec<String> = self.controls.vocabulary.lock().unwrap().iter().map(|e| e.word.clone()).collect();
            if replacements_on {
                terms.extend(dict.iter().filter(|(_, _, kind)| *kind == EntryKind::Word).map(|(_, to, _)| to.clone()));
            }
            protected = mask_terms(raw, &terms, &mut made);
            protected.as_str()
        };

        let (cleaned, tier, fallback) = match self.mode() {
            // Tone rewrites voice, which only the AI tier can do — Rules just
            // strips/fixes, it can't re-voice a sentence. `tone` is unused on
            // these two branches on purpose, so Off/Rules stay byte-identical
            // to before tones existed.
            PolishMode::Off => (raw.trim().to_string(), "off", ""),
            PolishMode::Rules => (self.apply_harper(&tier0(raw)), "rules", ""),
            PolishMode::Ai => {
                let rules = tier0(raw);
                // The model gets the real identifiers, for context.
                match self.polish_ai(&unmask(raw, &made), &unmask(&rules, &made), tone).and_then(|t| remask(&t, &made)) {
                    // No Harper on the model's output: the model already makes
                    // the mechanical fixes Harper targets, with the whole
                    // sentence in view, and on invented model outputs Harper's
                    // only change was a wrong one ("Hi Sam," -> "Hi SAM,").
                    Ok(text) => (text, "ai", ""),
                    // Every fallback gets exactly what Rules mode gives (#55).
                    Err(reason) => (self.apply_harper(&rules), "rules", reason),
                }
            }
        };
        // Diff against the word-dictionary-corrected raw, not the ASR-
        // original one, so a Word-entry substitution is never miscounted as
        // a tier0/AI correction — and BEFORE the snippet pass below, so
        // corrected-words and dict-hits don't double-count the same word.
        let corrected_words = changed_words(raw, &cleaned);
        let cleaned = unmask_from(&cleaned, &made, idents);
        let (text, snippet_hits) = if dict.is_empty() || !replacements_on {
            (cleaned, 0)
        } else {
            apply_dictionary(&cleaned, &dict, EntryKind::Snippet)
        };
        let text = unmask(&text, &made);
        PolishResult { text, corrected_words, dict_hits: word_hits + snippet_hits, notable, tier, fallback }
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
    /// listening pill past the hotkey press, and in the AI tier — whose rules
    /// fallback (short clips, tidy ones, LLM failures) runs these too — it
    /// lands *after* the user has already spoken.
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
            && !raw.chars().any(is_arabic)
    }

    /// Tier 1: route long-enough dictations through the LLM. `Err` names why
    /// the caller keeps the `rules` result instead: a short clip, an already
    /// tidy one, a missing sidecar, an LLM error, or an output
    /// `rewrite_guard` rejects.
    fn polish_ai(&self, raw: &str, rules: &str, tone: &str) -> Result<String, &'static str> {
        // The model can't do Arabic (#112). Measured: with a trained
        // vocabulary it answered in English on 22 of 22 Arabic or
        // code-switched takes; without one the guard had to refuse 16 of 22;
        // and the one Arabic word of an English take was dropped or
        // translated, and delivered, 4 of 4. Any Arabic script keeps the
        // rules result.
        if raw.chars().any(is_arabic) {
            return Err("arabic");
        }
        if word_count(raw) < self.ai_min_words() {
            return Err("short"); // too short to be worth the round-trip
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
            return Err("no-op");
        }
        let llm = self.llm.as_ref().ok_or("unavailable")?;

        let vocabulary = self.controls.vocabulary.lock().unwrap().clone();
        let t = std::time::Instant::now();
        // The known mishearings this take holds, for the worked example,
        // then the other mishearings of those same words.
        let (mut heard, mut spare) = (Vec::new(), Vec::new());
        for e in &vocabulary {
            let known = e.heard_as.iter().filter(|h| !h.trim().is_empty());
            let (found, other): (Vec<_>, Vec<_>) = known.partition(|h| find_whole_ci(rules, h).is_some());
            if !found.is_empty() {
                heard.extend(found.into_iter().map(|h| (h.clone(), e.word.clone())));
                spare.extend(other.into_iter().map(|h| (h.clone(), e.word.clone())));
            }
        }
        heard.extend(spare);
        let text = match llm.polish(rules, tone, &vocabulary_clause(&vocabulary), &heard) {
            Ok(text) if !text.trim().is_empty() => text,
            Ok(_) => {
                tracing::warn!("AI polish returned empty text — using rules");
                return Err("empty");
            }
            Err(err) => {
                tracing::warn!(error = %err, "AI polish failed — using rules");
                return Err("llm-error");
            }
        };
        // What the model wrote, refused or not: debug only, like the
        // transcript trail (#48). `--polish-qa` reads a refusal from it.
        tracing::debug!(model = %text, "AI polish output");
        if let Some(reason) = rewrite_guard(rules, &text, &vocabulary) {
            return Err(reason);
        }
        let text = if rules.contains('\n') { restore_breaks(rules, &text).ok_or("line-breaks")? } else { text };
        tracing::info!(llm_ms = t.elapsed().as_millis(), "AI polish applied");
        Ok(text)
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

    let mut chars: Vec<char> = text.chars().collect();
    let english = FstDictionary::curated();
    let mut fixes: Vec<&Lint> = lints
        .iter()
        .filter(|l| SAFE_LINT_KINDS.contains(&l.lint_kind) && l.suggestions.len() == 1)
        .filter(|l| !respells_a_name(l, &chars, &english))
        .collect();
    // Back-to-front so an earlier edit can't shift a later span.
    fixes.sort_by(|a, b| b.span.start.cmp(&a.span.start));

    for lint in fixes {
        lint.suggestions[0].apply(lint.span, &mut chars);
    }
    chars.into_iter().collect()
}

/// Would `lint` rewrite a word Harper's dictionary doesn't know into other
/// letters (#113)? Such a word is a name, not a typo: a speech engine writes
/// words, it doesn't mistype them. Measured: "Heliboard" -> "Headboard",
/// "qwen" -> "q wen", "vanto" -> "van to", "basha" -> "bash a". Fixing its
/// casing ("github" -> "GitHub") is still fine.
fn respells_a_name(lint: &Lint, chars: &[char], english: &FstDictionary) -> bool {
    let word: String = lint.span.get_content(chars).iter().collect();
    let real = |w: &str| is_real_word(english, w);
    if word.is_empty() || !word.chars().all(char::is_alphanumeric) || real(&word) || real(&word.to_lowercase()) {
        return false;
    }
    !matches!(&lint.suggestions[0], Suggestion::ReplaceWith(to) if to.iter().collect::<String>().eq_ignore_ascii_case(&word))
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
    /// Notable substitutions worth surfacing to the user (currently the
    /// phonetic corrector's `(heard, corrected)` pairs) — consumed by the
    /// "something smart just happened" flyout, ignored elsewhere.
    pub notable: Vec<(String, String)>,
    /// The tier that actually produced `text` ("off" | "rules" | "ai") —
    /// not the configured one — and, when AI mode kept the rules result,
    /// why ("short", "no-op", "unavailable", "empty", "llm-error",
    /// "line-breaks", "identifiers", or a `rewrite_guard` reason); ""
    /// otherwise (#67).
    pub tier: &'static str,
    pub fallback: &'static str,
}

fn word_count(s: &str) -> usize {
    s.split_whitespace().count()
}

/// Arabic script, presentation forms included.
pub(crate) fn is_arabic(c: char) -> bool {
    matches!(c, '\u{0600}'..='\u{06FF}' | '\u{0750}'..='\u{077F}' | '\u{08A0}'..='\u{08FF}' | '\u{FB50}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFC}')
}

/// An AI rewrite must keep at least this share of its input's content words
/// (#65). Measured on the recordings review (#5): every clean AI take lost
/// at most one content word (97%+ kept); the three that dropped clauses kept
/// 72%, 84% and 95%.
const MIN_KEPT_PCT: usize = 96;
/// ...and may bring in at most this many content words its input never had
/// (vocabulary words aside). Clean takes added at most one (a respelling).
const MAX_NEW_WORDS: usize = 2;

/// The words a rewrite has to keep, lowercased. Skips what cleanup may
/// legitimately change: function words, fillers (the prompt's own "like" /
/// "يعني" too), numbers in digits or words (the model may reformat them),
/// contractions, and one-letter ASR fragments.
pub(crate) fn content_words(s: &str) -> Vec<String> {
    s.split(|c: char| !(c.is_alphanumeric() || matches!(c, '\'' | '’')))
        .filter(|w| w.chars().count() > 1 && !w.contains(['\'', '’']) && !w.contains(|c: char| c.is_ascii_digit()))
        .map(str::to_lowercase)
        .filter(|w| {
            !FUNCTION_WORDS.contains(&w.as_str())
                && !FILLERS.contains(&w.as_str())
                && !["like", "يعني"].contains(&w.as_str())
                && classify_number(w).is_none()
        })
        .collect()
}

/// Why an AI rewrite can't be trusted over the rules result, or `None` if it
/// can (#65): it dropped words the speaker said, brought in words they
/// didn't, answered with the worked example's words (`llm::VOCAB_EXAMPLE`),
/// or lost a word of another script (#112). Casing, punctuation, line breaks,
/// fillers and number formatting never count (see `content_words`). A trained
/// word may appear from nowhere and its known mishearings may vanish — that
/// swap is the model's job. Logs counts only, never text (#48).
///
/// ponytail: a bag of words — it catches dropped clauses and invented text,
/// not a self-correction flipped using the same words. Upgrade path: an
/// order-aware diff (`changed_words`' LCS) if that shows up again.
fn rewrite_guard(input: &str, output: &str, vocabulary: &[VocabEntry]) -> Option<&'static str> {
    let trained: Vec<String> = vocabulary.iter().flat_map(|e| content_words(&e.word)).collect();
    let misheard: Vec<String> = vocabulary.iter().flat_map(|e| e.heard_as.iter().flat_map(|h| content_words(h))).collect();
    let mut have: HashMap<String, usize> = HashMap::new();
    for w in content_words(input).into_iter().filter(|w| !misheard.contains(w)) {
        *have.entry(w).or_default() += 1;
    }
    let total: usize = have.values().sum();
    // The example only goes out with a vocabulary (see `llm::Llm::polish`).
    let example: Vec<String> = if vocabulary.is_empty() {
        Vec::new()
    } else {
        crate::llm::VOCAB_EXAMPLE.iter().flat_map(|(_, a)| content_words(a)).collect()
    };
    let (mut kept, mut new, mut leak) = (0, 0, false);
    for w in content_words(output) {
        match have.get_mut(&w) {
            Some(n) if *n > 0 => {
                *n -= 1;
                kept += 1;
            }
            Some(_) => {} // said again: a repeat, not a new word
            None if trained.contains(&w) || misheard.contains(&w) => {}
            None => {
                new += 1;
                leak |= example.contains(&w);
            }
        }
    }
    // One lost English word may be a filler the model was right to drop. A
    // lost word in another script was dropped or translated (#112): "the
    // build failed بكرة" came back without it, "the client قال" as "said".
    let foreign_lost = have.iter().any(|(w, n)| *n > 0 && w.contains(|c: char| c.is_alphabetic() && c > '\u{024F}'));
    let reason = if leak {
        "example-leak"
    } else if foreign_lost || (total - kept >= 2 && kept * 100 < total * MIN_KEPT_PCT) {
        "dropped-words"
    } else if new > MAX_NEW_WORDS {
        "new-words"
    } else {
        return None;
    };
    tracing::warn!(reason, kept, total, new, "AI polish rewrote the take — using rules");
    Some(reason)
}

/// Auto tone (#28): the text already in the field, as a style-only tone
/// sentence. It steers register and format, never content: the model is told
/// not to copy, continue or answer it, and `rewrite_guard` is unchanged, so
/// an output that picks up the field's words (they aren't in the take) still
/// falls back to rules.
fn context_clause(context: &str) -> String {
    format!(
        "Match the register and formatting of the text the speaker is writing into, which so far \
         ends: \"{context}\". Use it only for style; never copy, continue or answer it."
    )
}

/// Build the LLM vocabulary clause: "Word (heard as a, b, c), Word2, ..."
/// for each configured entry, skipping any with an empty `word`. An entry
/// with no `heard_as` still names the word alone — less reliable per the
/// note on `PolishConfig::vocabulary`, but still better than nothing.
fn vocabulary_clause(entries: &[VocabEntry]) -> String {
    entries
        .iter()
        .filter(|e| !e.word.trim().is_empty())
        .map(|e| {
            if e.heard_as.is_empty() {
                e.word.clone()
            } else {
                format!("{} (heard as {})", e.word, e.heard_as.join(", "))
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
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

/// Spoken line-break commands (F1). Deliberately just these two — Parakeet
/// and Whisper already punctuate competently, so "period"/"comma"/"dash"
/// commands would only add false-positive risk ("a period of time") for
/// something ASR already does. Order doesn't matter: neither phrase is a
/// substring of the other.
const FORMATTING_COMMANDS: &[(&str, &str)] = &[("new paragraph", "\n\n"), ("new line", "\n")];

/// Rewrite "new line"/"new paragraph" into real breaks. Reuses `replace_whole_ci`
/// for the actual phrase match (case-insensitive, whole-word) — the only new
/// logic is absorbing the spaces left dangling around the inserted break, so
/// "hello new line world" becomes "hello\nworld", not "hello \n world".
fn apply_formatting_commands(raw: &str) -> String {
    let mut text = raw.to_string();
    for (phrase, brk) in FORMATTING_COMMANDS {
        text = replace_whole_ci(&text, phrase, brk).0;
    }
    // An ASR that hears the command as its own sentence punctuates it
    // ("list. New line. Apples."), which left ". Apples." on the new line
    // (#29). A mark alone at the start of a line was the command's, so it
    // goes with it; one that starts a word (".env") stays.
    let text = collapse_space_around_breaks(&text);
    let mut lines = text.split('\n');
    let mut out = lines.next().unwrap_or_default().to_string();
    for line in lines {
        out.push('\n');
        out.push_str(match line.strip_prefix(['.', ',', ';', ':', '!', '?']) {
            Some(rest) if rest.is_empty() || rest.starts_with(' ') => rest.trim_start(),
            _ => line,
        });
    }
    out
}

/// Rewrite `text` word by word without losing its line breaks. `pass` gets
/// one line's whitespace tokens and returns the words to put back; they're
/// rejoined with single spaces and the lines with the exact breaks between
/// them. Every word-level pass (tier0, numbers, phonetic) goes through here:
/// each used to split and rejoin the whole text itself, flattening the
/// breaks "new line"/"new paragraph" had just made (#85). Per line, a number
/// run, a stutter or a phonetic match also never reaches across a break.
fn rewrite_words<'a, S: std::borrow::Borrow<str>>(text: &'a str, mut pass: impl FnMut(&[&'a str]) -> Vec<S>) -> String {
    text.split('\n')
        .map(|line| pass(&line.split_whitespace().collect::<Vec<_>>()).join(" "))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Put `input`'s line breaks back into the model's rewrite of it. The model
/// joins lines whatever the prompt says (measured: 0 of 5 multi-line takes
/// kept a single break, with or without a stronger instruction or a marker
/// glyph), so each break goes back between the same two words — the last of
/// one line, the first of the next — found in order. `None` if a pair didn't
/// survive the rewrite; the caller then keeps the rules result (#85).
///
/// ponytail: exact word pairs, so a boundary the model reworded ("I will" ->
/// "I'll") falls back to rules. Upgrade path: fuzzy-match the pair.
fn restore_breaks(input: &str, output: &str) -> Option<String> {
    let norm = |t: &str| split_affixes(t).1.to_lowercase();
    let toks: Vec<(usize, &str)> =
        output.split_whitespace().map(|t| (t.as_ptr() as usize - output.as_ptr() as usize, t)).collect();
    let mut breaks = vec![0; toks.len()]; // '\n's after each output token
    let (mut lead, mut pending, mut pos) = (0, 0, 0);
    let mut last: Option<&str> = None; // last word of the previous non-empty line
    for (i, line) in input.split('\n').enumerate() {
        pending += usize::from(i > 0);
        let Some(first) = line.split_whitespace().next() else { continue };
        match last {
            None => lead = pending,
            Some(before) => {
                let (b, f) = (norm(before), norm(first));
                let k = (pos..toks.len().saturating_sub(1))
                    .find(|&k| norm(toks[k].1) == b && norm(toks[k + 1].1) == f)?;
                breaks[k] = pending;
                pos = k + 1;
            }
        }
        pending = 0;
        last = line.split_whitespace().last();
    }
    let mut out = "\n".repeat(lead);
    for (k, &(start, tok)) in toks.iter().enumerate() {
        if k > 0 {
            let (prev_start, prev) = toks[k - 1];
            match breaks[k - 1] {
                0 => out.push_str(&output[prev_start + prev.len()..start]),
                n => out.push_str(&"\n".repeat(n)),
            }
        }
        out.push_str(tok);
    }
    out.push_str(&"\n".repeat(pending));
    Some(out)
}

/// Drop spaces/tabs immediately touching a `\n` just inserted above. Raw
/// dictation transcripts never contain real newlines, so any `\n` seen here
/// came from `apply_formatting_commands` and is safe to trim around.
fn collapse_space_around_breaks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\n' {
            while matches!(out.chars().last(), Some(' ') | Some('\t')) {
                out.pop();
            }
            out.push('\n');
            while matches!(chars.peek(), Some(' ') | Some('\t')) {
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

// ── spoken-number normalization (inverse text normalization) ─────────────
// Parakeet emits numbers as words ("twenty three", "seven fifteen a.m.") and
// neither Harper nor the 1.5B LLM reliably turns them back into figures, so a
// dictated "23" lands as "twenty three". This deterministic pass fixes that.
//
// The one real hazard: a naive left-fold turns "seven fifteen" into 22 (7+15).
// It isn't — "seven fifteen" is not how anyone says 22. So the cardinal parser
// only *combines* words that form a genuine cardinal (a ten may take a
// following unit, a unit/teen/ten may precede a scale); anything else ends the
// run. "seven fifteen" therefore parses as two numbers, 7 and 15, which the
// clock rule below can then read as 7:15 when a meridiem follows.

/// One number word's role in a cardinal.
#[derive(Clone, Copy, PartialEq)]
enum NumTok {
    Unit(u64), // one..nine (1-9)
    Teen(u64), // ten..nineteen (10-19)
    Ten(u64),  // twenty..ninety (20,30,..,90)
    Hundred,
    Scale(u64),        // thousand/million/billion
    And,               // connector, only valid mid-number
    OrdUnit(u64),      // first..ninth — terminal, but may follow a ten (twenty third)
    OrdTerm(u64),      // tenth/twelfth/twentieth/hundredth... — fully terminal
}

fn classify_number(word: &str) -> Option<NumTok> {
    let w = word.to_ascii_lowercase();
    use NumTok::*;
    Some(match w.as_str() {
        "one" => Unit(1), "two" => Unit(2), "three" => Unit(3), "four" => Unit(4),
        "five" => Unit(5), "six" => Unit(6), "seven" => Unit(7), "eight" => Unit(8),
        "nine" => Unit(9), "zero" => Unit(0),
        "ten" => Teen(10), "eleven" => Teen(11), "twelve" => Teen(12), "thirteen" => Teen(13),
        "fourteen" => Teen(14), "fifteen" => Teen(15), "sixteen" => Teen(16),
        "seventeen" => Teen(17), "eighteen" => Teen(18), "nineteen" => Teen(19),
        "twenty" => Ten(20), "thirty" => Ten(30), "forty" => Ten(40), "fifty" => Ten(50),
        "sixty" => Ten(60), "seventy" => Ten(70), "eighty" => Ten(80), "ninety" => Ten(90),
        "hundred" => Hundred,
        "thousand" => Scale(1_000), "million" => Scale(1_000_000), "billion" => Scale(1_000_000_000),
        "and" => And,
        "first" => OrdUnit(1), "second" => OrdUnit(2), "third" => OrdUnit(3),
        "fourth" => OrdUnit(4), "fifth" => OrdUnit(5), "sixth" => OrdUnit(6),
        "seventh" => OrdUnit(7), "eighth" => OrdUnit(8), "ninth" => OrdUnit(9),
        "tenth" => OrdTerm(10), "eleventh" => OrdTerm(11), "twelfth" => OrdTerm(12),
        "thirteenth" => OrdTerm(13), "fourteenth" => OrdTerm(14), "fifteenth" => OrdTerm(15),
        "sixteenth" => OrdTerm(16), "seventeenth" => OrdTerm(17), "eighteenth" => OrdTerm(18),
        "nineteenth" => OrdTerm(19), "twentieth" => OrdTerm(20), "thirtieth" => OrdTerm(30),
        "fortieth" => OrdTerm(40), "fiftieth" => OrdTerm(50), "sixtieth" => OrdTerm(60),
        "seventieth" => OrdTerm(70), "eightieth" => OrdTerm(80), "ninetieth" => OrdTerm(90),
        "hundredth" => OrdTerm(100), "thousandth" => OrdTerm(1_000),
        _ => return None,
    })
}

/// English ordinal suffix for `n` (1 -> "st", 2 -> "nd", 3 -> "rd", 11-13 -> "th").
fn ordinal_suffix(n: u64) -> &'static str {
    if (11..=13).contains(&(n % 100)) {
        return "th";
    }
    match n % 10 {
        1 => "st",
        2 => "nd",
        3 => "rd",
        _ => "th",
    }
}

/// A parsed number run: its value, whether it was ordinal, and how many
/// whitespace tokens it consumed.
struct NumberRun {
    value: u64,
    ordinal: bool,
    consumed: usize,
}

/// Parse the longest genuine cardinal/ordinal number starting at `words[0]`.
/// `words` are bare lowercased cores (punctuation already stripped). Returns
/// `None` if the first word isn't a number word, or the run is only a lone
/// "and". Only *valid* cardinal transitions extend the run — see the module
/// note on why "seven fifteen" must parse as 7 then 15, not 22.
fn parse_number_run(words: &[&str]) -> Option<NumberRun> {
    use NumTok::*;
    let mut total: u64 = 0; // accumulated across scale words (thousand+)
    let mut group: u64 = 0; // current 0..999 group
    let mut group_has_small = false; // a unit/teen/ten already placed in this group
    let mut consumed = 0usize;
    let mut ordinal = false;
    let mut produced = false;
    let mut trailing_and = false; // last consumed token was a bare "and"

    for &word in words {
        let Some(tok) = classify_number(word) else { break };
        match tok {
            Unit(n) => {
                // A unit extends a group only right after a ten ("twenty three")
                // or at the start of a fresh group; a unit after a unit/teen is
                // a new number ("seven fifteen" -> stop).
                if group_has_small && group % 10 != 0 {
                    break;
                }
                // `% 100`: the ten may sit on a hundred. Without it "one
                // hundred twenty eight" came out "120 8" (#29).
                if group_has_small && !(20..=99).contains(&(group % 100)) {
                    break;
                }
                group += n;
                group_has_small = true;
                produced = true;
            }
            Teen(n) | Ten(n) => {
                if group_has_small {
                    break; // "thirteen fifteen", "twenty thirty" — not one number
                }
                group += n;
                group_has_small = true;
                produced = true;
            }
            Hundred => {
                if group == 0 || group > 9 {
                    break; // "hundred" needs a preceding 1-9 ("two hundred")
                }
                group *= 100;
                group_has_small = false;
                produced = true;
            }
            Scale(mult) => {
                let g = if group == 0 { 1 } else { group };
                total += g * mult;
                group = 0;
                group_has_small = false;
                produced = true;
            }
            And => {
                // Only a connector *inside* a number ("one hundred and five").
                if !produced {
                    break;
                }
                // don't count "and" as producing; keep scanning
            }
            OrdUnit(n) => {
                if group_has_small && (group % 10 != 0 || !(20..=99).contains(&(group % 100))) {
                    break;
                }
                group += n;
                ordinal = true;
                produced = true;
                consumed += 1;
                break; // ordinal is terminal
            }
            OrdTerm(n) => {
                if group_has_small || group != 0 {
                    break;
                }
                group = n;
                ordinal = true;
                produced = true;
                consumed += 1;
                break;
            }
        }
        // Only a token that actually made it into the run (didn't `break`
        // above) reaches here — so this tracks the last *consumed* token, not
        // the one that ended the run.
        trailing_and = matches!(tok, And);
        consumed += 1;
    }

    // A connector is never the last word of a number ("three and four" is a
    // list, not 3-and-4) — back it off so it's re-emitted as the word "and".
    if trailing_and && consumed > 0 {
        consumed -= 1;
    }

    if !produced || consumed == 0 {
        return None;
    }
    Some(NumberRun { value: total + group, ordinal, consumed })
}

/// Split a whitespace token into (leading, core, trailing) where core is the
/// alphanumeric middle used for number matching and the affixes are punctuation
/// to reattach.
fn split_affixes(tok: &str) -> (&str, &str, &str) {
    let start = tok.find(|c: char| c.is_alphanumeric()).unwrap_or(tok.len());
    // End after the last alphanumeric char's full width — `+ 1` would slice
    // inside a multi-byte one ("café", any Arabic word) and panic.
    let end = tok.char_indices().rev().find(|(_, c)| c.is_alphanumeric()).map_or(start, |(i, c)| i + c.len_utf8());
    (&tok[..start], &tok[start..end], &tok[end..])
}

/// Is this token a meridiem marker (a.m./p.m./am/pm, any casing/punctuation)?
/// Returns the normalized-lowercase core if so.
fn meridiem(tok: &str) -> Option<String> {
    let core: String = tok.chars().filter(|c| c.is_ascii_alphabetic()).collect();
    match core.to_ascii_lowercase().as_str() {
        "am" | "pm" => Some(core.to_ascii_lowercase()),
        _ => None,
    }
}

/// Turn spoken numbers into digits. Runs as its own pass (see the config
/// `number_formatting` toggle), independent of polish mode.
fn normalize_numbers(text: &str) -> String {
    rewrite_words(text, number_words)
}

/// `normalize_numbers` over one line's tokens.
fn number_words(toks: &[&str]) -> Vec<String> {
    // Bare lowercased cores, for the run parser.
    let cores: Vec<String> = toks.iter().map(|t| split_affixes(t).1.to_ascii_lowercase()).collect();
    let core_refs: Vec<&str> = cores.iter().map(|s| s.as_str()).collect();

    let mut out: Vec<String> = Vec::with_capacity(toks.len());
    let mut i = 0;
    while i < toks.len() {
        let Some(run) = parse_number_run(&core_refs[i..]) else {
            out.push(toks[i].to_string());
            i += 1;
            continue;
        };
        let (lead, _, _) = split_affixes(toks[i]);
        let (_, _, trail_last) = split_affixes(toks[i + run.consumed - 1]);

        // Try a following cardinal run for the year-pair and clock-time rules.
        let next = if i + run.consumed < toks.len() {
            parse_number_run(&core_refs[i + run.consumed..])
        } else {
            None
        };

        // Clock time: hour(1-12) minute(0-59) <meridiem>. Anchored on the
        // meridiem so a bare "seven fifteen" isn't forced into a time.
        if !run.ordinal {
            if let Some(min) = &next {
                let after = i + run.consumed + min.consumed;
                if !min.ordinal
                    && (1..=12).contains(&run.value)
                    && min.value <= 59
                    && after < toks.len()
                    && meridiem(toks[after]).is_some()
                {
                    out.push(format!("{lead}{}:{:02}", run.value, min.value));
                    i += run.consumed + min.consumed;
                    continue;
                }
            }
        }

        // Year pair: "nineteen ninety nine" / "twenty twenty six" -> 1999 / 2026.
        // Constrained to 19xx/20xx so ordinary adjacent numbers aren't merged.
        if !run.ordinal {
            if let Some(yr2) = &next {
                if !yr2.ordinal
                    && (run.value == 19 || run.value == 20)
                    && yr2.value <= 99
                {
                    let (_, _, y_trail) = split_affixes(toks[i + run.consumed + yr2.consumed - 1]);
                    out.push(format!("{lead}{}{:02}{y_trail}", run.value, yr2.value));
                    i += run.consumed + yr2.consumed;
                    continue;
                }
            }
        }

        // "the next one", "that one", "a new one": the pronoun, not a count (#66).
        // Nor is it a count before an ordinal ("one second", "one third"), or
        // as the one English word between Arabic ones (#114).
        if run.consumed == 1 && core_refs[i] == "one" {
            let ordinal_next = core_refs.get(i + 1).is_some_and(|w| matches!(classify_number(w), Some(NumTok::OrdUnit(_) | NumTok::OrdTerm(_))));
            if pronoun_one(&core_refs[..i]) || ordinal_next || arabic_around(toks, i) {
                out.push(toks[i].to_string());
                i += 1;
                continue;
            }
        }

        // A lone ordinal is a figure only where one is written (#114): see
        // `ordinal_figure`. After a number word it is a compound ("twenty
        // second") and never gets here.
        if run.ordinal && run.consumed == 1 && !ordinal_figure(toks, &core_refs, i) {
            out.push(toks[i].to_string());
            i += 1;
            continue;
        }

        let suffix = if run.ordinal { ordinal_suffix(run.value) } else { "" };
        out.push(format!("{lead}{}{suffix}{trail_last}", run.value));
        i += run.consumed;
    }
    out
}

const MONTHS: [&str; 12] = [
    "january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november",
    "december",
];

/// Nouns that are numbered, so an ordinal before one is written as a figure:
/// "3rd floor", "2nd quarter", "5th Avenue".
const NUMBERED: &[&str] =
    &["floor", "grade", "quarter", "century", "edition", "anniversary", "birthday", "avenue", "street", "percentile"];

/// Is the lone ordinal at `i` written as a figure (#114)? Only next to its
/// month ("March fifteenth", "the fifteenth of March") or before a numbered
/// noun. Anywhere else it is a word: "wait a second", "at first", "first
/// draft", "First, the budget". A month named "May" has to be capitalized to
/// count on its own: "you may first want to" has no date in it.
fn ordinal_figure(toks: &[&str], cores: &[&str], i: usize) -> bool {
    let month = |j: usize| cores.get(j).is_some_and(|w| MONTHS.contains(w) && (*w != "may" || split_affixes(toks[j]).1 == "May"));
    (i > 0 && month(i - 1))
        || month(i + 1)
        || (cores.get(i + 1) == Some(&"of") && cores.get(i + 2).is_some_and(|w| MONTHS.contains(w)))
        || cores.get(i + 1).is_some_and(|w| NUMBERED.contains(w))
}

/// Is the token at `i` an island in Arabic: every neighbour it has is in
/// Arabic script?
fn arabic_around(toks: &[&str], i: usize) -> bool {
    let mut sides = [i.checked_sub(1).map(|j| toks[j]), toks.get(i + 1).copied()].into_iter().flatten().peekable();
    sides.peek().is_some() && sides.all(|t| t.chars().any(is_arabic))
}

/// Words right after which a lone "one" is the pronoun, never a count.
const ONE_PRONOUN_AFTER: &[&str] =
    &["the", "that", "this", "which", "next", "last", "other", "another", "each", "every", "no", "any"];

/// Is a lone "one" after `prev` (the lowercased words before it) the
/// pronoun? After a determiner ("that one", "no one") or an ordinal ("the
/// third one"), or after an article and one plain word ("a new one", "the
/// big one"). Everything else stays a count: "one of three", "I have one".
fn pronoun_one(prev: &[&str]) -> bool {
    match prev {
        [.., p] if ONE_PRONOUN_AFTER.contains(p) || matches!(classify_number(p), Some(NumTok::OrdUnit(_) | NumTok::OrdTerm(_))) => {
            true
        }
        [.., a, p] => {
            matches!(*a, "the" | "a" | "an") && !p.is_empty() && !FUNCTION_WORDS.contains(p) && classify_number(p).is_none()
        }
        _ => false,
    }
}

/// Which quote-mark glyphs spoken quote commands produce. Straight is the
/// default: it's the one choice that's safe everywhere, including code
/// editors and terminals, where curly quotes are a syntax error waiting to
/// happen. Curly is the opt-in for anyone who mostly dictates prose.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum QuoteStyle {
    Straight,
    Curly,
}

impl QuoteStyle {
    fn from_config_str(s: &str) -> Self {
        if s.eq_ignore_ascii_case("curly") { QuoteStyle::Curly } else { QuoteStyle::Straight }
    }
    fn glyphs(self) -> (&'static str, &'static str) {
        match self {
            QuoteStyle::Straight => ("\"", "\""),
            QuoteStyle::Curly => ("\u{201C}", "\u{201D}"), // “ ”
        }
    }
}

/// Rewrite spoken quote commands (F2) into literal quote-mark glyphs. Two
/// independent forms, both whole-phrase and case-insensitive:
/// - "open quote" / "close quote" — two standalone tokens (the Dragon/Apple
///   convention), each a plain phrase swap via `replace_whole_ci`.
/// - "quote ... unquote" (also "quote on quote ...", ASR's common rendering
///   of a doubled "quote, unquote" filler) — wraps the SPAN between the two
///   in quote marks; see `apply_paired_quote` for the closer-must-exist
///   guard.
///
/// Token form runs first: "open quote" and "close quote" both contain the
/// bare word "quote", so resolving them before the paired form's scan for a
/// standalone "quote" keeps the two forms from colliding with each other.
fn apply_quote_commands(raw: &str, style: QuoteStyle) -> String {
    let (open, close) = style.glyphs();
    let text = replace_and_trim_after(raw, "open quote", open);
    let text = replace_and_trim_before(&text, "close quote", close);
    apply_paired_quote(&text, open, close)
}

/// Like `replace_whole_ci`, but also consumes one whitespace/comma character
/// immediately AFTER the match — "open quote hello" -> `"hello`, not
/// `" hello`, and "open quote, hello" (Parakeet punctuating the discourse
/// marker) -> `"hello`, not `", hello`. A quote mark hugs its content.
fn replace_and_trim_after(hay: &str, needle: &str, rep: &str) -> String {
    let mut out = String::new();
    let mut rest = hay;
    loop {
        let Some((start, end)) = find_whole_ci(rest, needle) else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..start]);
        out.push_str(rep);
        let after = &rest[end..];
        rest = after.strip_prefix([' ', ',']).unwrap_or(after).trim_start_matches(' ');
    }
    out
}

/// Mirror of `replace_and_trim_after`, trimming one trailing whitespace/comma
/// character immediately BEFORE the match instead.
fn replace_and_trim_before(hay: &str, needle: &str, rep: &str) -> String {
    let mut out = String::new();
    let mut rest = hay;
    loop {
        let Some((start, end)) = find_whole_ci(rest, needle) else {
            out.push_str(rest);
            break;
        };
        let mut before = &rest[..start];
        before = before.strip_suffix(' ').unwrap_or(before);
        before = before.strip_suffix(',').unwrap_or(before);
        before = before.strip_suffix(' ').unwrap_or(before);
        out.push_str(before);
        out.push_str(rep);
        rest = &rest[end..];
    }
    out
}

/// First whole-word, case-insensitive occurrence of `needle` in `hay`, as a
/// byte range into `hay`. Same boundary rule as `replace_whole_ci` (kept
/// independent rather than sharing code with it — `replace_whole_ci`'s
/// left-boundary check relies on scanning the ORIGINAL string with a single
/// global index, and slicing into it mid-scan would silently break that
/// check at the slice's own start).
pub(crate) fn find_whole_ci(hay: &str, needle: &str) -> Option<(usize, usize)> {
    let hay_lc = hay.to_ascii_lowercase();
    let needle_lc = needle.to_ascii_lowercase();
    let mut i = 0;
    while i <= hay_lc.len() {
        let start = i + hay_lc[i..].find(&needle_lc)?;
        let end = start + needle_lc.len();
        if is_edge(&hay[..start], &hay[end..]) {
            return Some((start, end));
        }
        let ch_len = hay[start..].chars().next().map_or(1, |c| c.len_utf8());
        i = start + ch_len;
    }
    None
}

/// Trim whitespace AND a leading/trailing comma. Parakeet punctuates a
/// spoken "quote"/"unquote" used as a discourse marker with a comma of its
/// own ("and I quote, I am the leader" — the comma is part of saying
/// "quote", not part of what's quoted), so a plain `.trim()` leaks it into
/// the span. Real dictation showed this; the hand-typed unit test inputs
/// never had commas to catch it.
fn trim_span(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() || c == ',')
}

/// Wrap the span after a "quote" trigger in `open_glyph`/`close_glyph`.
/// Closes on the next "unquote" if one is said. Otherwise, ONLY when
/// "quote" is immediately followed by a comma (Parakeet punctuating the
/// natural pause of "quote" used as a discourse marker — "and I quote, ..."),
/// infers the close at the next sentence-ending punctuation, or the end of
/// the text if there isn't one. Confirmed live: saying "and I quote ...
/// unquote" isn't how Kai actually talks — twice in a row he said "and I
/// quote, [sentence]." with no closer at all, so requiring "unquote" alone
/// left every real attempt untouched.
///
/// The comma check is load-bearing, not decoration — an earlier version of
/// this closed at the next terminal punctuation unconditionally whenever
/// there was no "unquote", which fires on nearly every sentence eventually
/// (nearly all of them end with SOME punctuation) and broke the exact case
/// this function's false-positive guard exists for: "I found a quote from
/// the paper" flows straight from "quote" into "from" with no pause,
/// because there it's an ordinary noun, not an introduction — no comma, no
/// inferred close, left alone, same as before this feature existed.
///
/// ponytail: a single left-to-right greedy pass — a standalone "quote" that
/// appears *before* a genuine, separate pair later in the same take gets
/// wrongly treated as that pair's opener. Sentence-boundary scoping would
/// fix it; not worth it until Kai actually dictates two separate quoted
/// spans in one breath.
fn apply_paired_quote(text: &str, open_glyph: &str, close_glyph: &str) -> String {
    const OPENERS: &[&str] = &["quote on quote", "quote"]; // longest first
    let mut out = String::new();
    let mut rest = text;
    loop {
        // Earliest opener match; prefer the longest alias when several start
        // at the same position (mirrors DictEntry::phrases' longest-first
        // rule, for the same reason — "quote" is a valid whole-word match
        // inside "quote on quote" too).
        let mut best: Option<(usize, usize, usize)> = None; // (start, end, alias_idx)
        for (idx, opener) in OPENERS.iter().enumerate() {
            if let Some((start, end)) = find_whole_ci(rest, opener) {
                let better = match best {
                    None => true,
                    Some((bstart, _, bidx)) => start < bstart || (start == bstart && idx < bidx),
                };
                if better {
                    best = Some((start, end, idx));
                }
            }
        }
        let Some((ostart, oend, _)) = best else {
            out.push_str(rest);
            break;
        };
        if let Some((cstart_rel, cend_rel)) = find_whole_ci(&rest[oend..], "unquote") {
            let cstart = oend + cstart_rel;
            let cend = oend + cend_rel;
            out.push_str(&rest[..ostart]);
            out.push_str(open_glyph);
            out.push_str(trim_span(&rest[oend..cstart]));
            out.push_str(close_glyph);
            // "unquote" can carry the same trailing discourse-marker comma
            // "quote" does ("...unquote, to me") -- strip ONLY the comma,
            // not the space after it: unlike the opener/token forms, what
            // follows "unquote" is the surrounding sentence continuing, not
            // quoted content, so it keeps its normal word-separating space
            // ("hello there" + "unquote, to me" -> ...hello there" to me,
            // not ...hello there"to me).
            let after = &rest[cend..];
            rest = after.strip_prefix(',').unwrap_or(after);
            continue;
        }
        // No "unquote" said. Only infer a close when "quote" is immediately
        // followed by a comma — Parakeet punctuating the natural pause of
        // "quote" used as a discourse marker ("and I quote, ..."), the same
        // signal `trim_span` already strips. Its ABSENCE is exactly what
        // marks the false-positive case this guard exists for: "a quote
        // from the paper" flows straight from "quote" into "from" with no
        // pause, because there "quote" is an ordinary noun, not an
        // introduction. Without this check, "close at the next terminal
        // punctuation" fires on nearly every sentence eventually, since
        // nearly every sentence ends with one — which is exactly what broke
        // this guard the first time this was written.
        if rest[oend..].trim_start_matches(' ').starts_with(',') {
            let tail = &rest[oend..];
            let close_at = tail.find(['.', '!', '?']).map(|i| i + 1).unwrap_or(tail.len());
            out.push_str(&rest[..ostart]);
            out.push_str(open_glyph);
            out.push_str(trim_span(&tail[..close_at]));
            out.push_str(close_glyph);
            rest = &tail[close_at..];
            continue;
        }
        // Neither signal present — not a pair. Emit the opener as plain
        // text and keep scanning after it.
        out.push_str(&rest[..oend]);
        rest = &rest[oend..];
    }
    out
}

// ── variable recognition (#27) ───────────────────────────────────────────
// In a code editor, "camel case user name" types userName. Explicit spoken
// commands only: nothing is guessed from context, which would need the
// editor's own symbol list.

/// The casing styles, each said as "<style> case".
const CASINGS: &[&str] = &["camel", "pascal", "snake", "kebab", "constant"];
/// Words one casing command takes at most.
const CASING_MAX_WORDS: usize = 4;

/// The casing command starting at `toks[i]`, if one does: "<style> case",
/// any capitalization, with no punctuation inside or after it. "I like camel
/// case, mostly" has a pause there, so it stays prose.
fn casing_at(toks: &[&str], i: usize) -> Option<&'static str> {
    let (_, style, trail) = split_affixes(toks.get(i)?);
    if !trail.is_empty() || !toks.get(i + 1)?.eq_ignore_ascii_case("case") {
        return None;
    }
    CASINGS.iter().find(|s| s.eq_ignore_ascii_case(style)).copied()
}

/// Join plain ASCII `words` in `style`: camel userName, pascal UserName,
/// snake user_name, kebab user-name, constant USER_NAME.
fn case_words(style: &str, words: &[&str]) -> String {
    let lower: Vec<String> = words.iter().map(|w| w.to_ascii_lowercase()).collect();
    let cap = |w: &String| w[..1].to_ascii_uppercase() + &w[1..];
    match style {
        "camel" => lower[0].clone() + &lower[1..].iter().map(cap).collect::<String>(),
        "pascal" => lower.iter().map(cap).collect(),
        "snake" => lower.join("_"),
        "kebab" => lower.join("-"),
        _ => lower.join("_").to_ascii_uppercase(),
    }
}

/// Stands in for identifier `i` after `apply_casing_commands`: one Private
/// Use Area char, which no ASR emits, no dictionary entry holds, the phonetic
/// corrector and tier0 skip (it isn't alphanumeric), and Harper lexes as an
/// unlintable token, never a word, so it neither respells it nor starts a
/// sentence on it.
fn placeholder(i: usize) -> char {
    char::from_u32(0xE000 + i as u32).unwrap_or('\u{E000}')
}

/// Rewrite each casing command and the 1-4 words after it as one
/// identifier. A command takes plain ASCII words (letters, digits) until a
/// natural break: a word with punctuation after it is the last one taken
/// (the punctuation stays after the identifier); punctuation before a word, a
/// word that isn't plain ASCII (Arabic, "it's"), the next command or the end
/// of the line stop it before that word. A command with no word to take is
/// left as said. Returns the text with each identifier masked by its
/// `placeholder`, and the identifiers in order (`unmask` puts them back).
fn apply_casing_commands(text: &str) -> (String, Vec<String>) {
    let mut made = Vec::new();
    let out = rewrite_words(text, |toks| {
        let mut out = Vec::with_capacity(toks.len());
        let mut i = 0;
        while i < toks.len() {
            let Some(style) = casing_at(toks, i) else {
                out.push(toks[i].to_string());
                i += 1;
                continue;
            };
            let (mut words, mut trail, mut j) = (Vec::new(), "", i + 2);
            while j < toks.len() && words.len() < CASING_MAX_WORDS && casing_at(toks, j).is_none() {
                let (lead, core, after) = split_affixes(toks[j]);
                if !lead.is_empty() || core.is_empty() || !core.chars().all(|c| c.is_ascii_alphanumeric()) {
                    break;
                }
                words.push(core);
                j += 1;
                if !after.is_empty() {
                    trail = after;
                    break;
                }
            }
            if words.is_empty() {
                out.push(toks[i].to_string());
                i += 1;
                continue;
            }
            out.push(format!("{}{}{trail}", split_affixes(toks[i]).0, placeholder(made.len())));
            made.push(case_words(style, &words));
            i = j;
        }
        out
    });
    (out, made)
}

/// Put `apply_casing_commands`' identifiers back in place of their
/// placeholders.
fn unmask(text: &str, made: &[String]) -> String {
    unmask_from(text, made, 0)
}

/// `unmask` for the placeholders from `first` on, leaving the earlier ones
/// masked: the trained words `mask_terms` added after the identifiers.
fn unmask_from(text: &str, made: &[String], first: usize) -> String {
    if made.len() <= first {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match (c as u32).checked_sub(0xE000).map(|i| i as usize).filter(|&i| i >= first).and_then(|i| made.get(i)) {
            Some(ident) => out.push_str(ident),
            None => out.push(c),
        }
    }
    out
}

/// Mask every whole-word occurrence of a `term` (#113), one placeholder
/// each, and push the term so `unmask` writes it back as trained. Any casing
/// of it matches, unless the term is one ordinary English word ("Cursor",
/// "Notion"): then only the trained casing does, or every "cursor" would come
/// out capitalized, `dictionary_safe`'s trap again. Longest term first, so
/// "Nuvanto Flow" is taken whole before "Flow".
fn mask_terms(text: &str, terms: &[String], made: &mut Vec<String>) -> String {
    let mut terms: Vec<&str> = terms.iter().map(|t| t.trim()).filter(|t| !t.is_empty()).collect();
    terms.sort_unstable();
    terms.dedup();
    terms.sort_by_key(|t| std::cmp::Reverse(t.len()));
    let mut text = text.to_string();
    if terms.is_empty() {
        return text;
    }
    let english = FstDictionary::curated();
    for term in terms {
        let exact = !term.contains(' ') && is_real_word(&english, &term.to_lowercase());
        loop {
            let found = if exact {
                text.match_indices(term).map(|(s, _)| (s, s + term.len())).find(|&(s, e)| is_edge(&text[..s], &text[e..]))
            } else {
                find_whole_ci(&text, term)
            };
            let Some((start, end)) = found else { break };
            text.replace_range(start..end, placeholder(made.len()).encode_utf8(&mut [0; 4]));
            made.push(term.to_string());
        }
    }
    text
}

/// Mask the AI tier's rewrite again, so the snippet pass can't touch the
/// identifiers either. Each must come back exactly as made: the model
/// capitalizing, splitting or dropping one would undo the command, so that
/// keeps the rules result instead. The one thing the model does add is
/// backticks around it (measured: both identifiers of a two-identifier take),
/// which an editor doesn't want typed, so a pair hugging it goes.
///
/// ponytail: a substring find, longest first, so an identifier inside a
/// longer word the model wrote counts as kept. Unmasking restores the same
/// characters, so only the guard is looser. Upgrade path: whole-word match.
fn remask(text: &str, made: &[String]) -> Result<String, &'static str> {
    let mut order: Vec<usize> = (0..made.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(made[i].len()));
    let mut text = text.to_string();
    for i in order {
        let mut from = text.find(made[i].as_str()).ok_or("identifiers")?;
        let mut to = from + made[i].len();
        if text[..from].ends_with('`') && text[to..].starts_with('`') {
            (from, to) = (from - 1, to + 1);
        }
        text.replace_range(from..to, placeholder(i).encode_utf8(&mut [0; 4]));
    }
    Ok(text)
}

/// Tier 0 rules cleanup. `rewrite_words` also collapses runs of spaces and
/// trims each line, so filtering + rejoining handles whitespace
/// normalization for free.
fn tier0(raw: &str) -> String {
    capitalize_first(&rewrite_words(raw, |toks| {
        let kept: Vec<&str> = toks.iter().copied().filter(|t| !is_filler(t)).collect();
        // Stutter collapse runs AFTER filler stripping: by now "uh uh uh" is
        // already gone, so any run of identical tokens left is genuine word
        // repetition ("the the store", "I I I think"), not disfluency.
        collapse_stutters(&kept)
    }))
}

/// Collapse a run of the same token repeated back-to-back to one occurrence,
/// case-insensitively but keeping the first occurrence's own spelling/casing
/// (so "we should go to The the store" keeps "The", not "the").
///
/// Words in `LEGIT_DOUBLES` are exempt — see there for why.
///
/// ponytail: no grammar-awareness here, so the exemption is a flat word list
/// rather than a real judgement about the sentence. Harper's Repetition lint
/// runs right after tier0 and independently flags some doubled words, so this
/// isn't the only safety net. Upgrade path if a stutter slips through on an
/// exempt word: gate the exemption on the repeat being comma-free.
fn collapse_stutters<'a>(tokens: &[&'a str]) -> Vec<&'a str> {
    let mut out: Vec<&str> = Vec::with_capacity(tokens.len());
    for &tok in tokens {
        let core = tok.trim_matches(|c: char| !c.is_alphanumeric());
        // Numbers are never collapsed: "row 1 1 of 2" is two digits the user
        // actually said, not a stutter — repeated digits read aloud are
        // common and meaningful, unlike a repeated word, so they don't clear
        // the "user never means to keep this" bar the rest of this pass does.
        let is_numeric = !core.is_empty() && core.chars().all(|c| c.is_ascii_digit());
        // A sentence-ending mark on the *previous* token means this token
        // starts a new sentence ("I saw him. Him and I left"), not a
        // stutter — commas don't count, since a stutter is often
        // transcribed with a pause comma in between ("the, the store").
        let new_sentence = out.last().is_some_and(|p: &&str| p.ends_with(['.', '!', '?', '؟', '؛']));
        // Arabic says a word twice on purpose (#111): "كده كده" is "either
        // way" where "كده" is "like this", and "شوية شوية", "واحدة واحدة",
        // "جدا جدا", "لا لا" all mean the pair. The doubling is open-ended
        // (any noun can be dealt out "بيت بيت"), so a list like
        // `LEGIT_DOUBLES` can't hold it, and the two mistakes aren't equal: a
        // real stutter left in ("في في مشكلة") is a repeat the user sees and
        // deletes, a collapsed pair is a changed meaning they don't. So an
        // Arabic-script token is never collapsed.
        let arabic = core.chars().any(is_arabic);
        // A token that OPENS a quoted span is a hard boundary too, same
        // idea as new_sentence: "and I <quote>I am the leader..." tokenizes
        // to "I" then a quote-glued "\"I" (no space between the glyph
        // `apply_quote_commands` inserts and the word it wraps) — after
        // `core` strips the leading `"`, that looks identical to a genuine
        // stutter ("I I"), but it's a real word starting deliberately
        // quoted material, not a repeat. Checked on the raw token, not
        // `core`, since the quote glyph is exactly the punctuation `core`
        // strips away — confirmed live: this silently ate an entire quoted
        // opening, glyph included, the first time quote commands + stutter
        // collapse combined on real input.
        let starts_quoted = tok.starts_with('"') || tok.starts_with('\u{201C}');
        let exempt = LEGIT_DOUBLES.iter().any(|w| w.eq_ignore_ascii_case(core));
        if !is_numeric && !new_sentence && !starts_quoted && !exempt && !arabic {
            if let Some(prev) = out.last() {
                let prev_core = prev.trim_matches(|c: char| !c.is_alphanumeric());
                if !prev_core.is_empty() && prev_core.eq_ignore_ascii_case(core) {
                    continue; // drop the repeat; first occurrence's token (casing, punctuation) survives
                }
            }
        }
        out.push(tok);
    }
    out
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

/// Capitalize the first letter — after any leading break, so a take that
/// opens with "new paragraph" still starts with a capital.
fn capitalize_first(s: &str) -> String {
    let body = s.trim_start();
    let mut chars = body.chars();
    match chars.next() {
        Some(first) => format!("{}{}{}", &s[..s.len() - body.len()], first.to_uppercase(), chars.as_str()),
        None => s.to_string(),
    }
}

/// Apply each `spoken → replacement` entry as a case-insensitive, whole-phrase
// ── phonetic correction ──────────────────────────────────────────────────
// Exact dictionary/vocabulary entries fix mishearings the user has already
// seen and listed. This catches the *unseen* ones: a token that sounds like a
// trained proper noun but was misheard a new way ("clode"/"claud" for
// "Claude"). Classic American Soundex keys both. It runs before the LLM, so
// the model still sees the intended word.
//
// Precision over recall (#63): the first version matched any word with the
// same first letter and a 4-char code within one edit, and on real takes it
// fired wrongly ~10x more often than rightly ("said"/"sure"/"soon" ->
// "Sotto", "called"/"class" -> "Claude"). Now a token must key EXACTLY like
// the target over its whole skeleton, must not be a real English word (names
// aside), and must be plain letters, not code (a filename, path or number).

/// American Soundex code — first letter + EVERY consonant digit (no 4-char
/// cut or zero padding: the whole skeleton must match) — for the alphabetic
/// part of `word`. "" for a wordless token.
fn soundex(word: &str) -> String {
    let letters: Vec<char> =
        word.chars().filter(|c| c.is_ascii_alphabetic()).map(|c| c.to_ascii_uppercase()).collect();
    if letters.is_empty() {
        return String::new();
    }
    let code = |c: char| -> u8 {
        match c {
            'B' | 'F' | 'P' | 'V' => b'1',
            'C' | 'G' | 'J' | 'K' | 'Q' | 'S' | 'X' | 'Z' => b'2',
            'D' | 'T' => b'3',
            'L' => b'4',
            'M' | 'N' => b'5',
            'R' => b'6',
            _ => 0, // vowels + H, W, Y carry no digit
        }
    };
    let mut out = String::with_capacity(letters.len());
    out.push(letters[0]);
    let mut last = code(letters[0]);
    for &c in &letters[1..] {
        let d = code(c);
        if d != 0 && d != last {
            out.push(d as char);
        }
        // H and W are transparent (don't reset the "same code merges" run);
        // a vowel does reset it, so a repeated code across a vowel is kept.
        if c != 'H' && c != 'W' {
            last = d;
        }
    }
    out
}

/// A token that is code, not speech — a filename, path, identifier or number
/// ("claude.md", "src/sotto", "sotto_v2"). Trailing sentence punctuation
/// doesn't count, so "soto." is still a word.
fn looks_like_code(tok: &str) -> bool {
    tok.trim_end_matches(|c: char| c.is_ascii_punctuation() && !matches!(c, '/' | '\\' | '_'))
        .contains(|c: char| matches!(c, '.' | '/' | '\\' | '_') || c.is_ascii_digit())
}

/// Pull tokens that *sound like* a trained target word back to that word.
/// `targets` are the words and phrases (proper nouns/jargon) the user
/// trained. Multi-word spans go first (`apply_span_corrections`), then single
/// tokens toward the single-word targets. Returns the corrected text plus the
/// `(heard, corrected)` pairs that fired.
fn apply_phonetic_corrections(text: &str, targets: &[String]) -> (String, Vec<(String, String)>) {
    let (text, mut fired) = apply_span_corrections(text, targets);
    // Only single-word targets of 4+ letters — short words are too collision-
    // prone to phonetic-match safely, and exact dictionary entries cover
    // those anyway.
    let coded: Vec<(&str, String)> = targets
        .iter()
        .filter(|t| t.split_whitespace().count() == 1)
        .filter(|t| t.chars().filter(|c| c.is_ascii_alphabetic()).count() >= 4)
        .map(|t| (t.as_str(), soundex(t)))
        .filter(|(_, c)| !c.is_empty())
        .collect();
    if coded.is_empty() {
        return (text, fired);
    }
    // Harper's curated dictionary (already loaded for Rules mode) knows which
    // tokens are real English words.
    let english = FstDictionary::curated();

    let out = rewrite_words(&text, |toks| {
        toks.iter().map(|&tok| {
            let (lead, core, trail) = split_affixes(tok);
            // Plain letters only: skips code ("std::io"), contractions and
            // possessives ("they're", "Claude's") and non-English words.
            if core.len() < 4 || looks_like_code(tok) || !core.chars().all(|c| c.is_ascii_alphabetic()) {
                return tok.to_string();
            }
            // Already the intended word (any casing) — never touch it.
            if coded.iter().any(|(t, _)| t.eq_ignore_ascii_case(core)) {
                return tok.to_string();
            }
            // A real English word is what the speaker said, not a mishearing.
            if is_real_word(&english, core) {
                return tok.to_string();
            }
            let tc = soundex(core);
            for (target, gc) in &coded {
                // The whole skeleton, exactly — including the real first
                // letter Soundex keeps.
                if tc == *gc {
                    fired.push((core.to_string(), (*target).to_string()));
                    return format!("{lead}{target}{trail}");
                }
            }
            tok.to_string()
        })
        .collect()
    });
    (out, fired)
}

/// Harper's curated dictionary knows `word` as real English: common ("said",
/// "called") or not ("culled", "trie"). Non-words ("clode") and names
/// ("Soto") aren't.
fn is_real_word(english: &FstDictionary, word: &str) -> bool {
    english.get_word_metadata_str(word).is_some_and(|m| m.common || !m.is_proper_noun())
}

/// May a trainer correction also write an exact dictionary entry (#94)?
/// An entry rewrites its phrase everywhere, in every polish mode, so it may
/// only key on something the user never says. "clode" -> yes. "cloud" -> no:
/// an entry would rewrite every "cloud" (#63's failure again), so it stays a
/// `heard_as` hint, which the Whisper prompt and the AI polish use in context.
///
/// A phrase gets an entry only if NONE of its words is real. ASR hears an
/// unknown name as real words ("cloud code", "adding gravity"), and those
/// are phrases the user can say. Mixed ones ("clode code") stay hints too:
/// Harper's "not a word" also means "a word Harper lacks" (Egyptian
/// transliterations, jargon), so one unknown word among real ones is weak
/// proof the phrase can't be real speech. A deliberate entry is still one
/// click away on the Dictionary page.
pub fn dictionary_safe(heard: &str) -> bool {
    let english = FstDictionary::curated();
    heard.split_whitespace().all(|tok| !is_real_word(&english, split_affixes(tok).1))
}

/// A span's code must be at least this long (onset letter + 5 consonant
/// digits) to match at all. Shorter skeletons are ordinary phrases too:
/// "Kai's Flow" keys K214, exactly like "keys fell".
const SPAN_MIN_CODE: usize = 6;
/// ...and this long before one slip (`one_slip`) is forgiven; shorter codes
/// must match exactly.
const SPAN_SLIP_CODE: usize = 7;

/// Common function words. Short, frequent and weakly stressed: ASR drops,
/// adds and swaps them freely, and their 0–2 digit codes would let a window
/// slide a match across ordinary sentence glue. A span holding one never
/// matches, unless the target itself contains that word. Compared with the
/// apostrophe removed ("it's" -> "its").
pub(crate) const FUNCTION_WORDS: &[&str] = &[
    "a", "an", "the", "and", "or", "but", "nor", "so", "if", "as", "at", "by", "for", "from", "in", "into", "of",
    "off", "on", "onto", "out", "over", "to", "up", "with", "is", "am", "are", "was", "were", "be", "been", "do",
    "does", "did", "has", "have", "had", "i", "me", "my", "we", "us", "our", "you", "your", "he", "him", "his",
    "she", "her", "it", "its", "they", "them", "their", "this", "that", "these", "those", "there", "here", "what",
    "which", "who", "when", "where", "how", "all", "then", "than", "not", "no", "can", "will", "just", "im", "ive",
    "dont", "cant", "thats", "lets",
];

/// Multi-word half of `apply_phonetic_corrections`: pull a RUN of words that
/// sounds like a trained target back to it — "adding gravity" ->
/// "Antigravity", "hugging phase" -> "Hugging Face". Every run of words is a
/// candidate, so the guard is far stricter than a naive match: see
/// `span_matches`, and the negative tests for the phrases each check stops.
/// Unlike the single-token pass, a span of real English words CAN match —
/// ASR hears an unknown name as real words — so the phonetic key itself
/// carries the precision.
fn apply_span_corrections(text: &str, targets: &[String]) -> (String, Vec<(String, String)>) {
    let keyed: Vec<(&str, Vec<&str>, String, usize)> = targets
        .iter()
        .filter_map(|t| {
            let words: Vec<&str> = t.split_whitespace().collect();
            let (code, beats) = span_key(&words);
            (code.len() >= SPAN_MIN_CODE).then_some((t.as_str(), words, code, beats))
        })
        .collect();
    let mut fired = Vec::new();
    if keyed.is_empty() {
        return (text.to_string(), fired);
    }
    // Each token with its byte offset, so the text between tokens (line
    // breaks included) is copied through untouched.
    let toks: Vec<(usize, &str)> =
        text.split_whitespace().map(|t| (t.as_ptr() as usize - text.as_ptr() as usize, t)).collect();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut i = 0;
    while i < toks.len() {
        // Window = the target's word count ±1, longest first: ASR moves word
        // boundaries freely ("adding gravity" for "Antigravity"). 1 word -> a
        // 1-word target is the single-token pass's job.
        let hit = keyed.iter().find_map(|(target, words, code, beats)| {
            let n = words.len();
            [n + 1, n, n - 1]
                .into_iter()
                .filter(|&k| k >= 1 && !(k == 1 && n == 1) && i + k <= toks.len())
                .find(|&k| span_matches(&toks[i..i + k], words, code, *beats))
                .map(|k| (k, *target))
        });
        let Some((k, target)) = hit else {
            i += 1;
            continue;
        };
        let (start, first) = toks[i];
        let (end, last) = toks[i + k - 1];
        let (lead, _, _) = split_affixes(first);
        let (_, _, trail) = split_affixes(last);
        let heard = &text[start + lead.len()..end + last.len() - trail.len()];
        fired.push((heard.to_string(), target.to_string()));
        out.push_str(&text[copied..start]);
        out.push_str(&format!("{lead}{target}{trail}"));
        copied = end + last.len();
        i += k;
    }
    out.push_str(&text[copied..]);
    (out, fired)
}

/// The guard for one candidate span against one target.
fn span_matches(span: &[(usize, &str)], words: &[&str], code: &str, beats: usize) -> bool {
    // Same onset letter first: it's cheap, and nearly every window fails it.
    if split_affixes(span[0].1).1.bytes().next().map(|b| b.to_ascii_uppercase()) != code.bytes().next() {
        return false;
    }
    let mut cores = Vec::with_capacity(span.len());
    for (j, (_, tok)) in span.iter().enumerate() {
        let (lead, core, trail) = split_affixes(tok);
        // Punctuation INSIDE the run is a pause the speaker made — never
        // bridge it ("adding, gravity").
        if (j > 0 && !lead.is_empty()) || (j + 1 < span.len() && !trail.is_empty()) {
            return false;
        }
        // Plain English-letter words only, never code: a number or an Arabic
        // word adds nothing to the code, so it would be swallowed unseen.
        if looks_like_code(tok)
            || core.is_empty()
            || !core.chars().all(|c| c.is_ascii_alphabetic() || matches!(c, '\'' | '’' | '-'))
        {
            return false;
        }
        let plain = core.chars().filter(char::is_ascii_alphabetic).collect::<String>().to_ascii_lowercase();
        if FUNCTION_WORDS.contains(&plain.as_str()) && !words.iter().any(|w| w.eq_ignore_ascii_case(core)) {
            return false;
        }
        cores.push(core);
    }
    // Already the intended words (any casing) — never touch them.
    if cores.len() == words.len() && cores.iter().zip(words).all(|(c, w)| c.eq_ignore_ascii_case(w)) {
        return false;
    }
    let (sc, sb) = span_key(&cores);
    // Same beat count (Soundex is vowel-blind: "Andy grabbed" IS A532613,
    // Antigravity's code, but 4 beats to its 5), then an exact skeleton — or
    // one slip on a long one. The onset already matched above, so a slip is
    // never in the first letter.
    sb == beats && (sc == code || (code.len() >= SPAN_SLIP_CODE && one_slip(&sc, code)))
}

/// Phonetic key of a run of words: the Soundex skeleton of the words run
/// together (word boundaries aren't audible, so 2 heard words can key like 1
/// target word) + its beat count (vowel groups per word, a syllable proxy).
fn span_key(words: &[&str]) -> (String, usize) {
    let beats = words
        .iter()
        .map(|w| w.to_ascii_lowercase().split(|c: char| !"aeiouy".contains(c)).filter(|s| !s.is_empty()).count())
        .sum();
    (soundex(&words.concat()), beats)
}

/// Same length and at most one slip: one sound substituted, or two
/// neighbouring sounds swapped ("adding" D-N vs "anti" N-T). No dropped or
/// extra sound — that's how an unrelated phrase gets close.
fn one_slip(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let diff: Vec<usize> = (0..a.len()).filter(|&i| a[i] != b[i]).collect();
    match diff[..] {
        [] | [_] => true,
        [i, j] => j == i + 1 && a[i] == b[j] && a[j] == b[i],
        _ => false,
    }
}

/// Apply replacements. Each entry is `(phrases, replacement, kind)` where
/// `phrases` is every way you say it (primary + aliases), already
/// longest-first; disabled entries have been dropped before we get here.
/// Only entries matching `only` fire — see `polish_with_tone` for why Word
/// and Snippet entries run at different points in the pipeline.
fn apply_dictionary(text: &str, dict: &[(Vec<String>, String, EntryKind)], only: EntryKind) -> (String, usize) {
    let mut out = text.to_string();
    let mut hits = 0;
    for (phrases, replacement, kind) in dict {
        if *kind != only {
            continue;
        }
        for spoken in phrases {
            if !spoken.trim().is_empty() {
                let (next, n) = replace_whole_ci(&out, spoken, replacement);
                out = next;
                hits += n;
            }
        }
    }
    (out, hits)
}

fn replace_whole_ci(hay: &str, needle: &str, rep: &str) -> (String, usize) {
    let hay_lc = hay.to_ascii_lowercase();
    let needle_lc = needle.to_ascii_lowercase();

    let mut out = String::with_capacity(hay.len());
    let mut count = 0;
    let mut i = 0;
    while i <= hay_lc.len() {
        match hay_lc[i..].find(&needle_lc) {
            Some(rel) => {
                let start = i + rel;
                let end = start + needle_lc.len();
                if is_edge(&hay[..start], &hay[end..]) {
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

/// A match between `before` and `after` is a whole word: no letter or digit
/// of any script touches it. ASCII-only letters let an Arabic entry fire
/// inside a longer Arabic word (#98). ASCII lowering keeps the offsets valid
/// in the original text.
fn is_edge(before: &str, after: &str) -> bool {
    let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    !word(before.chars().next_back()) && !word(after.chars().next())
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

    // ── E0: stutter collapse ─────────────────────────────────────────

    #[test]
    fn collapses_the_exact_ten_uh_report_case() {
        // Khairy's real report: a long run of "uh" plus a genuine word
        // stutter ("the the") in the same dictation. Fillers strip first
        // (each "uh" matches individually, run length doesn't matter), then
        // stutter collapse cleans up what's left.
        let raw = "uh uh uh uh uh uh uh uh uh uh I think we should go with the the plan";
        assert_eq!(rules(raw), "I think we should go with the plan");
    }

    #[test]
    fn collapses_repeated_words() {
        assert_eq!(rules("the the store"), "The store");
        assert_eq!(rules("I I I think"), "I think");
    }

    #[test]
    fn stutter_collapse_preserves_first_occurrence_casing() {
        // The survivor keeps the first occurrence's own casing, not the
        // repeat's — checked mid-sentence where `capitalize_first` (which
        // only touches index 0) can't paper over the difference.
        assert_eq!(rules("we should go to The the store"), "We should go to The store");
    }

    #[test]
    fn stutter_collapse_respects_sentence_boundaries() {
        // Punctuation between the repeats means a new sentence, not a
        // stutter — must survive untouched.
        assert_eq!(rules("I saw him. Him and I left"), "I saw him. Him and I left");
    }

    #[test]
    fn stutter_collapse_keeps_legitimate_doubles() {
        // Emphasis and real grammar — collapsing these changes meaning, which
        // is a worse failure than leaving a stutter in.
        assert_eq!(rules("this is very very important"), "This is very very important");
        assert_eq!(rules("he had had enough"), "He had had enough");
        assert_eq!(rules("no no that's wrong"), "No no that's wrong");
    }

    #[test]
    fn stutter_collapse_never_touches_arabic() {
        // #111: each pair means the pair. Through the whole Rules tier, so
        // Harper's own repetition lint is covered too.
        let (p, _) = trained(&[]);
        for same in [
            "كده كده هنخلص الشغل النهارده",
            "شوية شوية هتتعود على الموضوع",
            "امشي واحدة واحدة يا معلم",
            "لا لا مش كده خالص",
            "لا، لا مش كده",
            "الموضوع ده مهم جدا جدا",
            "يلا يلا نمشي",
            "بس بس كفاية",
            "ايوه ايوه صح",
            "ليه؟ ليه عملت كده",
            // What never collapsing gives up: a real stutter stays, in view.
            "في في مشكلة في الـ server",
        ] {
            assert_eq!(p.polish(same).text, same);
        }
        // ؟ and ؛ end a sentence for Latin words too; English is as before.
        assert_eq!(rules("okay؟ okay you said that"), "Okay؟ okay you said that");
        assert_eq!(rules("done؛ done means merged"), "Done؛ done means merged");
        assert_eq!(rules("الـ build build فشل"), "الـ build فشل");
        assert_eq!(rules("we should go to the the store"), "We should go to the store");
    }

    #[test]
    fn stutter_collapse_skips_numeric_tokens() {
        // Repeated digits are meaningful, not disfluency — never collapsed.
        assert_eq!(rules("row 1 1 of 2"), "Row 1 1 of 2");
    }

    #[test]
    fn stutter_collapse_does_not_eat_a_quote_glyph_glued_to_a_repeated_word() {
        // The real bug, found live: apply_quote_commands glues the opening
        // glyph directly to the quoted content's first word with no space
        // ("and I" + quote of "I am the leader..." -> "...and I \"I am...").
        // tier0's stutter-collapse strips punctuation before comparing
        // tokens, so "I" then "\"I" looked like a genuine "I I" stutter and
        // silently deleted the entire quoted opening, glyph included.
        assert_eq!(
            tier0("and I \"I am the leader of this group.\" and everyone believed him"),
            "And I \"I am the leader of this group.\" and everyone believed him"
        );
    }

    #[test]
    fn quote_commands_survive_rules_mode_end_to_end() {
        // Integration-level regression for the same bug: this exact input,
        // run through the real pipeline (quote commands, THEN tier0), used
        // to come out with the whole quoted opening deleted.
        let cfg = crate::config::Config::default();
        let controls = crate::Controls::from_config(&cfg);
        controls.polish_mode.store(PolishMode::Rules.as_u8(), Ordering::Relaxed);
        let p = Polisher::new(controls, cfg.llm.clone());
        let out = p.polish("he said, and I quote, I am the leader of this group. and everyone believed him");
        assert!(out.text.contains("\"I am the leader of this group."), "quote was eaten: {:?}", out.text);
    }

    // ── F1: voice formatting commands ───────────────────────────────

    #[test]
    fn formatting_commands_produce_the_right_breaks() {
        assert_eq!(apply_formatting_commands("hello new line world"), "hello\nworld");
        assert_eq!(apply_formatting_commands("hello new paragraph world"), "hello\n\nworld");
    }

    #[test]
    fn formatting_commands_absorb_surrounding_whitespace() {
        // The exact case the plan calls out: not "hello \n world".
        assert_eq!(apply_formatting_commands("hello new line world"), "hello\nworld");
        // Leading/trailing edges too — no stray space at either end.
        assert_eq!(apply_formatting_commands("new paragraph hello"), "\n\nhello");
        assert_eq!(apply_formatting_commands("hello new line"), "hello\n");
    }

    #[test]
    fn formatting_commands_take_the_mark_the_asr_put_after_them() {
        assert_eq!(apply_formatting_commands("Shopping list. New line. Apples. New line. Rice."), "Shopping list.\nApples.\nRice.");
        // The mark before the command is the speaker's sentence: it stays.
        assert_eq!(apply_formatting_commands("Dear team, new paragraph, the office is closed."), "Dear team,\n\nthe office is closed.");
        assert_eq!(apply_formatting_commands("Thanks. New line."), "Thanks.\n");
        // Not a stray mark: it starts the word.
        assert_eq!(apply_formatting_commands("it goes in new line .env"), "it goes in\n.env");
    }

    #[test]
    fn formatting_commands_are_case_insensitive() {
        assert_eq!(apply_formatting_commands("hello New Line world"), "hello\nworld");
        assert_eq!(apply_formatting_commands("hello NEW PARAGRAPH world"), "hello\n\nworld");
    }

    #[test]
    fn formatting_commands_inert_when_toggle_off() {
        let cfg = crate::config::Config::default();
        let controls = crate::Controls::from_config(&cfg);
        controls.formatting_commands.store(false, Ordering::Relaxed);
        controls.polish_mode.store(PolishMode::Off.as_u8(), Ordering::Relaxed);
        let p = Polisher::new(controls, cfg.llm.clone());
        assert_eq!(p.polish("hello new line world").text, "hello new line world");
    }

    #[test]
    fn formatting_commands_run_before_the_dictionary_so_a_snippet_saying_new_line_survives() {
        // This is the ordering bug the plan calls out: formatting commands
        // must run on the raw transcript, before dictionary replacement, so a
        // replacement's own literal text is never mistaken for the command.
        let cfg = crate::config::Config::default();
        let controls = crate::Controls::from_config(&cfg);
        controls.polish_mode.store(PolishMode::Off.as_u8(), Ordering::Relaxed);
        let entry = crate::config::DictEntry {
            spoken: "my snippet".into(),
            replacement: "here is a new line for you".into(),
            aliases: vec![],
            enabled: true,
            kind: Some(crate::config::EntryKind::Snippet),
        };
        *controls.dictionary.lock().unwrap() = vec![(
            entry.phrases().into_iter().map(str::to_string).collect::<Vec<_>>(),
            entry.replacement.clone(),
            entry.kind.unwrap(),
        )];
        let p = Polisher::new(controls, cfg.llm.clone());
        let out = p.polish("please insert my snippet now");
        assert_eq!(out.text, "please insert here is a new line for you now");
    }

    // ── F2: voice quote commands ─────────────────────────────────────

    #[test]
    fn open_close_quote_tokens_hug_their_content() {
        assert_eq!(
            apply_quote_commands("she said open quote hello there close quote to me", QuoteStyle::Straight),
            "she said \"hello there\" to me"
        );
    }

    #[test]
    fn open_close_quote_tokens_are_case_insensitive() {
        assert_eq!(
            apply_quote_commands("Open Quote hi Close Quote", QuoteStyle::Straight),
            "\"hi\""
        );
    }

    #[test]
    fn paired_quote_unquote_wraps_the_span_between() {
        assert_eq!(
            apply_quote_commands("he said quote hello there unquote to me", QuoteStyle::Straight),
            "he said \"hello there\" to me"
        );
    }

    #[test]
    fn quote_on_quote_is_an_alias_for_the_paired_opener() {
        // ASR's common rendering of a doubled "quote, unquote" filler.
        assert_eq!(
            apply_quote_commands("he said quote on quote testing unquote", QuoteStyle::Straight),
            "he said \"testing\""
        );
    }

    #[test]
    fn a_lone_quote_with_no_closer_is_left_alone() {
        // The false-positive guard the plan calls out: "quote" without a
        // later "unquote" is ordinary speech, not a dictated pair. No comma
        // right after "quote" here either -- it flows straight into "from",
        // the second signal that rules out an inferred close too.
        let s = "I found a quote from the paper";
        assert_eq!(apply_quote_commands(s, QuoteStyle::Straight), s);
    }

    #[test]
    fn a_comma_pause_after_quote_infers_the_close_at_the_next_period() {
        // Kai's real, unprompted usage -- confirmed live, twice in a row:
        // he never says "unquote". Parakeet punctuates "quote" as a
        // discourse marker with a comma, which is the signal that lets this
        // fire without also matching "a quote from the paper" (see the
        // no-comma test above).
        // "and I" stays outside the quote -- only the word "quote" itself
        // becomes the glyph, same as "open quote"/"close quote" only ever
        // replace the marker phrase, never any introductory words before it.
        assert_eq!(
            apply_quote_commands(
                "he said, and I quote, I am the leader of this group. and everyone believed him",
                QuoteStyle::Straight
            ),
            "he said, and I \"I am the leader of this group.\" and everyone believed him"
        );
    }

    #[test]
    fn a_comma_pause_after_quote_with_no_terminal_punctuation_closes_at_end_of_text() {
        assert_eq!(
            apply_quote_commands("she told me, quote, we start Monday", QuoteStyle::Straight),
            "she told me, \"we start Monday\""
        );
    }

    #[test]
    fn explicit_unquote_still_trims_a_comma_on_either_side() {
        // The same discourse-marker comma Parakeet adds around "quote" gets
        // added around "unquote" too -- confirmed the hand-typed unit tests
        // above never had commas to catch this, since they were typed
        // directly rather than run through real ASR punctuation.
        assert_eq!(
            apply_quote_commands("he said, quote, hello there, unquote, to me", QuoteStyle::Straight),
            "he said, \"hello there\" to me"
        );
    }

    #[test]
    fn two_separate_pairs_in_one_take_both_wrap() {
        assert_eq!(
            apply_quote_commands("quote a unquote and quote b unquote", QuoteStyle::Straight),
            "\"a\" and \"b\""
        );
    }

    #[test]
    fn curly_style_uses_curly_glyphs() {
        assert_eq!(apply_quote_commands("quote hi unquote", QuoteStyle::Curly), "\u{201C}hi\u{201D}");
    }

    #[test]
    fn quote_commands_are_inert_when_formatting_toggle_is_off() {
        let cfg = crate::config::Config::default();
        let controls = crate::Controls::from_config(&cfg);
        controls.formatting_commands.store(false, Ordering::Relaxed);
        controls.polish_mode.store(PolishMode::Off.as_u8(), Ordering::Relaxed);
        let p = Polisher::new(controls, cfg.llm.clone());
        assert_eq!(p.polish("quote hello unquote").text, "quote hello unquote");
    }

    #[test]
    fn curly_quote_style_setting_reaches_the_full_pipeline() {
        let cfg = crate::config::Config::default();
        let controls = crate::Controls::from_config(&cfg);
        controls.polish_mode.store(PolishMode::Off.as_u8(), Ordering::Relaxed);
        *controls.quote_style.lock().unwrap() = "curly".to_string();
        let p = Polisher::new(controls, cfg.llm.clone());
        assert_eq!(p.polish("quote hi unquote").text, "\u{201C}hi\u{201D}");
    }

    #[test]
    fn word_kind_entries_run_before_tier0_so_a_filler_word_inside_the_phrase_survives() {
        // The real bug: a Word entry whose spoken phrase happens to contain
        // what tier0 treats as a filler ("uh") used to lose the match
        // entirely, because dictionary replacement ran AFTER tier0 had
        // already stripped it — "uh huh" -> tier0 drops "uh" -> "huh" ->
        // dictionary looks for "uh huh", finds nothing. This is the
        // deterministic half of the fix that lets "E and" -> "e&" survive
        // AI polish, which drops a stray trailing "and" the same way.
        let cfg = crate::config::Config::default();
        let controls = crate::Controls::from_config(&cfg);
        controls.polish_mode.store(PolishMode::Rules.as_u8(), Ordering::Relaxed);
        let entry = crate::config::DictEntry {
            spoken: "uh huh".into(),
            replacement: "yes".into(),
            aliases: vec![],
            enabled: true,
            kind: Some(crate::config::EntryKind::Word),
        };
        *controls.dictionary.lock().unwrap() = vec![(
            entry.phrases().into_iter().map(str::to_string).collect::<Vec<_>>(),
            entry.replacement.clone(),
            entry.kind.unwrap(),
        )];
        let p = Polisher::new(controls, cfg.llm.clone());
        let out = p.polish("uh huh that works");
        assert_eq!(out.dict_hits, 1, "the Word entry must have matched before tier0 ate \"uh\"");
        let lower = out.text.to_ascii_lowercase();
        assert!(lower.starts_with("yes"), "expected the entry's replacement, got {:?}", out.text);
        assert!(!lower.contains("huh"), "the un-replaced fragment must not survive: {:?}", out.text);
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

    // ── #113: the user's spellings, and names nobody trained ─────────
    /// Rules mode with `words` trained.
    fn trained(words: &[&str]) -> (Polisher, Controls) {
        let cfg = crate::config::Config::default();
        let controls = crate::Controls::from_config(&cfg);
        controls.polish_mode.store(PolishMode::Rules.as_u8(), Ordering::Relaxed);
        *controls.vocabulary.lock().unwrap() =
            words.iter().map(|w| VocabEntry { word: w.to_string(), ..Default::default() }).collect();
        (Polisher::new(controls.clone(), cfg.llm.clone()), controls)
    }

    #[test]
    fn harper_never_respells_a_trained_word() {
        let (p, _) = trained(&["Heliboard", "Qwen"]);
        let same = "I installed Heliboard and tried Qwen today";
        assert_eq!(p.polish(same).text, same);
        // The phonetic corrector's fix stands; "kwen" is too short for it.
        assert_eq!(p.polish("download heleboard and try kwen").text, "Download Heliboard and try kwen");
        // Heard in lowercase, written as trained, in either script's company.
        assert_eq!(p.polish("نزّل heliboard وجرّب qwen").text, "نزّل Heliboard وجرّب Qwen");
        assert_eq!(p.polish("heliboard and QWEN both work").text, "Heliboard and Qwen both work");
    }

    #[test]
    fn harper_never_respells_a_name_it_does_not_know() {
        let (p, _) = trained(&[]);
        for same in ["نزّل heliboard وجرّب qwen على onnx", "افتح new ده vanto flow دلوقتي"] {
            assert_eq!(p.polish(same).text, same);
        }
        assert_eq!(p.polish("yalla ya basha inshallah bokra").text, "Yalla ya basha inshallah bokra");
        // Casing is not respelling, and a real word's lints are untouched.
        assert_eq!(p.polish("push it to github").text, "Push it to GitHub");
        assert_eq!(p.polish("I went to the the store.").text, "I went to the store.");
    }

    #[test]
    fn a_trained_ordinary_word_keeps_the_casing_it_was_heard_in() {
        // "Cursor" the editor is trained; "cursor" the caret is still a word.
        let (p, _) = trained(&["Cursor"]);
        assert_eq!(p.polish("move the cursor to the top in Cursor").text, "Move the cursor to the top in Cursor");
    }

    #[test]
    fn a_word_entry_replacement_is_protected_and_a_snippet_still_keys_on_a_trained_word() {
        let (p, controls) = trained(&["Heliboard"]);
        *controls.dictionary.lock().unwrap() = vec![
            (vec!["zor X".into()], "Zorvecks".into(), EntryKind::Word),
            (vec!["heliboard link".into()], "example.com/heliboard".into(), EntryKind::Snippet),
        ];
        // Made by the entry, and written by the ASR on its own in lowercase.
        assert_eq!(p.polish("ask zor X about it then ask zorvecks again").text, "Ask Zorvecks about it then ask Zorvecks again");
        assert_eq!(p.polish("send me the heliboard link").text, "Send me the example.com/heliboard");
        // The master switch off: entries don't fire, so they recase nothing.
        controls.replacements_enabled.store(false, Ordering::Relaxed);
        assert_eq!(p.polish("ask zorvecks again").text, "Ask zorvecks again");
    }

    #[test]
    fn mask_terms_takes_the_longest_term_whole_and_each_occurrence() {
        let terms = ["Flow".to_string(), "Nuvanto Flow".to_string(), " ".to_string()];
        let mut made = vec!["userName".to_string()]; // an identifier, masked earlier
        let masked = mask_terms("open nuvanto flow and the Flow tab, not the overflow", &terms, &mut made);
        assert_eq!(made, ["userName", "Nuvanto Flow", "Flow"]);
        assert_eq!(unmask_from(&masked, &made, 1), "open Nuvanto Flow and the Flow tab, not the overflow");
        // "flow" is an ordinary word: lowercase, it is left as said.
        let mut made = Vec::new();
        assert_eq!(mask_terms("the flow is fine", &terms, &mut made), "the flow is fine");
        assert!(made.is_empty());
        // Placeholders before `first` stay masked.
        let made = vec!["userName".to_string(), "Qwen".to_string()];
        let text = format!("{} runs {}", placeholder(0), placeholder(1));
        assert_eq!(unmask_from(&text, &made, 1), format!("{} runs Qwen", placeholder(0)));
        assert_eq!(unmask(&text, &made), "userName runs Qwen");
    }

    #[test]
    fn numbers_the_three_reported_examples() {
        // Kai's exact live-tested cases.
        assert_eq!(normalize_numbers("I am twenty three years old"), "I am 23 years old");
        assert_eq!(normalize_numbers("seven fifteen a.m."), "7:15 a.m.");
        assert_eq!(
            normalize_numbers("the twenty third of September of twenty twenty six"),
            "the 23rd of September of 2026"
        );
    }

    #[test]
    fn numbers_cardinals_and_scales() {
        assert_eq!(normalize_numbers("twenty three"), "23");
        assert_eq!(normalize_numbers("one hundred and five"), "105");
        assert_eq!(normalize_numbers("two thousand twenty six"), "2026");
        assert_eq!(normalize_numbers("three million"), "3000000");
        assert_eq!(normalize_numbers("nine"), "9");
    }

    #[test]
    fn numbers_ordinals_take_the_right_suffix() {
        assert_eq!(normalize_numbers("March first"), "March 1st");
        assert_eq!(normalize_numbers("the second of June"), "the 2nd of June");
        assert_eq!(normalize_numbers("third floor"), "3rd floor");
        assert_eq!(normalize_numbers("July fourth"), "July 4th");
        assert_eq!(normalize_numbers("the eleventh of May"), "the 11th of May"); // 11-13 are always -th
        assert_eq!(normalize_numbers("twenty first"), "21st");
        assert_eq!(normalize_numbers("the twentieth century"), "the 20th century");
    }

    #[test]
    fn numbers_leave_a_lone_ordinal_a_word() {
        // #114's list, and the QA set's (#29).
        for s in [
            "wait a second",
            "give me one second",
            "the second one",
            "this is the first draft",
            "at first I thought so",
            "First, the budget. Second, the hiring plan. Third, the office move.",
            "okay so first we load the data",
            "pack a first aid kit",
            "one third of the class",
            "you may first want to check",
            "it is due on the fifteenth",
            "استنى second واحدة بس",
            "ده الـ first draft بتاعي",
            "خد one وانا هاخد الـ second",
        ] {
            assert_eq!(normalize_numbers(s), s);
        }
        assert_eq!(normalize_numbers("wait a second I need one more minute"), "wait a second I need 1 more minute");
        // Where a figure is written: a compound, a date, a numbered noun.
        assert_eq!(normalize_numbers("the twenty second of May"), "the 22nd of May");
        assert_eq!(normalize_numbers("due on march fifteenth"), "due on march 15th");
        assert_eq!(normalize_numbers("on May first, the second quarter starts"), "on May 1st, the 2nd quarter starts");
        // A count between Arabic words is still a count, unless it is "one".
        assert_eq!(normalize_numbers("عايز three نسخ من الملف"), "عايز 3 نسخ من الملف");
    }

    #[test]
    fn numbers_a_ten_and_a_unit_join_after_a_hundred() {
        // Was "120 8": the ten-then-unit check forgot the hundred (#29).
        assert_eq!(normalize_numbers("one hundred twenty eight"), "128");
        assert_eq!(normalize_numbers("three hundred and forty five units"), "345 units");
        assert_eq!(normalize_numbers("two thousand nine hundred ninety nine"), "2999");
        assert_eq!(normalize_numbers("the one hundred twenty third"), "the 123rd");
        // Still two numbers: a unit after a unit, a teen after a hundred's unit.
        assert_eq!(normalize_numbers("one hundred five six"), "105 6");
        assert_eq!(normalize_numbers("twenty three fourth street"), "23 4th street");
    }

    #[test]
    fn numbers_seven_fifteen_is_not_twenty_two() {
        // The core hazard: a left-fold would make this 22. Without a meridiem
        // it stays two separate figures, never a bogus sum.
        assert_eq!(normalize_numbers("seven fifteen"), "7 15");
        assert_eq!(normalize_numbers("seven fifteen p.m."), "7:15 p.m.");
        // Zero-padded minutes.
        assert_eq!(normalize_numbers("nine five a.m."), "9:05 a.m.");
    }

    #[test]
    fn numbers_year_pairs_only_for_19xx_20xx() {
        assert_eq!(normalize_numbers("nineteen ninety nine"), "1999");
        assert_eq!(normalize_numbers("twenty twenty"), "2020");
        // A non-year adjacent pair must NOT merge into a 4-digit run.
        assert_eq!(normalize_numbers("thirty forty"), "30 40");
    }

    #[test]
    fn numbers_preserve_surrounding_words_and_punctuation() {
        assert_eq!(normalize_numbers("I need three, maybe four."), "I need 3, maybe 4.");
        assert_eq!(normalize_numbers("no numbers here"), "no numbers here");
        assert_eq!(normalize_numbers(""), "");
    }

    #[test]
    fn soundex_codes_known_pairs() {
        assert_eq!(soundex("Claude"), "C43");
        assert_eq!(soundex("clod"), "C43"); // same code -> exact match
        assert_eq!(soundex("clawed"), "C43");
        assert_eq!(soundex("claw"), "C4"); // a different skeleton: no match
        assert_eq!(soundex("Claude's"), "C432"); // nor is the possessive
        assert_eq!(soundex("Sotto"), "S3");
        assert_eq!(soundex("Antigravity"), "A532613"); // no 4-char cut
        assert_eq!(soundex(""), "");
        assert_eq!(soundex("!!"), "");
    }

    #[test]
    fn phonetic_pulls_mishearings_to_trained_words() {
        let targets = vec!["Claude".to_string(), "Sotto".to_string()];
        let (out, fired) = apply_phonetic_corrections("ask clode to help", &targets);
        assert_eq!(out, "ask Claude to help");
        assert_eq!(fired, vec![("clode".to_string(), "Claude".to_string())]);

        // Punctuation preserved.
        assert_eq!(apply_phonetic_corrections("thanks clode,", &targets).0, "thanks Claude,");
        // "soto" -> Sotto, even at the end of a sentence.
        assert_eq!(apply_phonetic_corrections("open soto now", &targets).0, "open Sotto now");
        assert_eq!(apply_phonetic_corrections("I use soto.", &targets).0, "I use Sotto.");
    }

    #[test]
    fn phonetic_never_replaces_common_words_or_code() {
        // #63's repro: each of these keyed within one edit of a trained word.
        let targets = vec!["Claude".to_string(), "Sotto".to_string()];
        for s in [
            "I'm not sure but I said we should study it soon, sorry",
            "called class clear claw clawed cloud could",
            "a clod of earth was culled", // real words, just not common ones
            "Claude's and Sotto's", // the target's own possessive stays
            "open claude.md and src/sotto then sotto_v2 or claude3",
        ] {
            let (out, fired) = apply_phonetic_corrections(s, &targets);
            assert_eq!(out, s);
            assert!(fired.is_empty(), "{s}: {fired:?}");
        }
    }

    #[test]
    fn trainer_corrections_write_dictionary_entries_for_non_words_only() {
        // #94: non-words and names get an exact entry (plus the hint). Arabic
        // isn't English, so Harper can't vouch for it: it keeps today's entry.
        for heard in ["clode", "Clode", "soto.", "clode soto", "كوشري"] {
            assert!(dictionary_safe(heard), "{heard}");
        }
        // ...a real word, or a phrase holding one, stays a hint.
        for heard in ["cloud", "Cloud,", "clawed", "culled", "cloud code", "adding gravity", "clode code", "the German ICLI"] {
            assert!(!dictionary_safe(heard), "{heard}");
        }
    }

    #[test]
    fn phonetic_leaves_correct_and_unrelated_words_alone() {
        let targets = vec!["Claude".to_string(), "Sotto".to_string()];
        // Already correct — untouched, and not double-reported.
        let (out, fired) = apply_phonetic_corrections("Claude is here", &targets);
        assert_eq!(out, "Claude is here");
        assert!(fired.is_empty());
        // Unrelated words with different leading sounds stay put.
        assert_eq!(apply_phonetic_corrections("the meeting ran long", &targets).0, "the meeting ran long");
        // Too-short tokens are never phonetic-matched.
        assert_eq!(apply_phonetic_corrections("go to lab", &targets).0, "go to lab");
        // No targets -> no-op.
        assert_eq!(apply_phonetic_corrections("anything at all", &[]).0, "anything at all");
    }

    #[test]
    fn split_affixes_keeps_a_multibyte_last_char_whole() {
        // Used to slice inside the last char and panic — on any Arabic word,
        // in both the number and the phonetic pass.
        assert_eq!(split_affixes("café,"), ("", "café", ","));
        assert_eq!(split_affixes("«يعني»"), ("«", "يعني", "»"));
        let targets = vec!["Sotto".to_string()];
        assert_eq!(apply_phonetic_corrections("قال soto يعني", &targets).0, "قال Sotto يعني");
    }

    fn span_targets() -> Vec<String> {
        ["Antigravity", "Kai's Flow", "Hugging Face", "Jupyter Notebook"].map(String::from).to_vec()
    }

    #[test]
    fn phonetic_pulls_multiword_mishearings_to_trained_phrases() {
        let t = span_targets();
        // 2 heard words -> 1 target word, bridged by one swapped sound (D-N/N-T).
        let (out, fired) = apply_phonetic_corrections("open adding gravity and run it", &t);
        assert_eq!(out, "open Antigravity and run it");
        assert_eq!(fired, vec![("adding gravity".to_string(), "Antigravity".to_string())]);
        // A split compound, punctuation kept.
        assert_eq!(apply_phonetic_corrections("anti gravity, then ship", &t).0, "Antigravity, then ship");
        // 2 -> 2.
        assert_eq!(apply_phonetic_corrections("load the Jupiter notebook", &t).0, "load the Jupyter Notebook");
        assert_eq!(apply_phonetic_corrections("push it to hugging phase.", &t).0, "push it to Hugging Face.");
        // 1 -> 2.
        assert_eq!(apply_phonetic_corrections("the huggingface hub", &t).0, "the Hugging Face hub");
        // Line breaks around a span survive.
        let face = vec!["Hugging Face".to_string()];
        assert_eq!(apply_phonetic_corrections("hugging phase\n\nnext", &face).0, "Hugging Face\n\nnext");
    }

    #[test]
    fn phonetic_multiword_leaves_ordinary_english_alone() {
        let t = span_targets();
        for s in [
            "the keys fell off the desk",           // K214 = Kai's Flow's whole code: too short to match
            "our cash flow is fine",                // C214, same
            "a house fly landed on the rim",        // H214, same
            "his flaw is pride",                    // function word
            "we are adding a gravity term",         // function word inside the window
            "we kept adding, gravity did the rest", // a pause inside the window
            "adding great value",                   // one sound short of Antigravity
            "we were adding gravel to the path",    // two slips, and 4 beats to 5
            "Andy grabbed the rope",                // A532613 = Antigravity exactly, but 4 beats to 5
            "the kids were hugging fish",           // H25212 = Hugging Face exactly, but 3 beats to 4
            "hugging يعني phase",                   // an Arabic word is never swallowed
            "open hugging.face or hugging/phase",   // code, not speech
            "Hugging Face and hugging face and Jupyter Notebook", // already right, any casing
            // The issue's own example, deliberately NOT corrected: H214 is a
            // changed onset on a 3-digit skeleton, the same shape as "house
            // fly" and "cash flow". An exact dictionary entry covers it.
            "high's flaw",
        ] {
            let (out, fired) = apply_phonetic_corrections(s, &t);
            assert_eq!(out, s);
            assert!(fired.is_empty(), "{s}: {fired:?}");
        }
    }

    #[test]
    fn numbers_leave_the_pronoun_one_alone() {
        for s in [
            "I prefer the next one",
            "that one, not this one",
            "which one do you want",
            "no one replied",
            "every one of them agreed",
            "we need a new one",
            "the big one is broken",
        ] {
            assert_eq!(normalize_numbers(s), s);
        }
        assert_eq!(normalize_numbers("pick the third one"), "pick the third one");
    }

    #[test]
    fn numbers_still_count_with_one() {
        assert_eq!(normalize_numbers("one hundred"), "100");
        assert_eq!(normalize_numbers("one of three"), "1 of 3");
        assert_eq!(normalize_numbers("I have one"), "I have 1");
        assert_eq!(normalize_numbers("the one hundred days"), "the 100 days");
        assert_eq!(normalize_numbers("one more time"), "1 more time");
    }

    #[test]
    fn numbers_connector_and_is_not_swallowed() {
        // "and" inside a number is consumed ("one hundred and five" -> 105),
        // but a listing "and" between two numbers survives as the word.
        assert_eq!(normalize_numbers("three and four"), "3 and 4");
        assert_eq!(normalize_numbers("cats and dogs"), "cats and dogs");
        assert_eq!(normalize_numbers("one hundred and five"), "105");
    }

    #[test]
    fn dictionary_replaces_whole_phrases_case_insensitively() {
        let dict = vec![
            (vec!["gee pee tee".to_string()], "GPT".to_string(), EntryKind::Word),
            (vec!["arrow".to_string()], "→".to_string(), EntryKind::Word),
        ];
        assert_eq!(apply_dictionary("use Gee Pee Tee now", &dict, EntryKind::Word), ("use GPT now".into(), 1));
        assert_eq!(apply_dictionary("arrow key", &dict, EntryKind::Word), ("→ key".into(), 1));
        // Whole-word only: "arrows" must not become "→s".
        assert_eq!(apply_dictionary("two arrows here", &dict, EntryKind::Word), ("two arrows here".into(), 0));
        // No entries → untouched.
        assert_eq!(apply_dictionary("nothing", &[], EntryKind::Word), ("nothing".into(), 0));
    }

    #[test]
    fn dictionary_whole_words_hold_in_any_script() {
        let dict = vec![
            (vec!["نور".to_string()], "Nour".to_string(), EntryKind::Word),
            (vec!["sotto".to_string()], "Sotto".to_string(), EntryKind::Word),
            (vec!["ren".to_string()], "Wren".to_string(), EntryKind::Word),
        ];
        let fix = |t: &str| apply_dictionary(t, &dict, EntryKind::Word);
        // The standalone Arabic word, not the same letters inside "النور"/"منور" (#98).
        assert_eq!(fix("صباح النور يا نور، البيت منور"), ("صباح النور يا Nour، البيت منور".into(), 1));
        // Code-switched, either way round.
        assert_eq!(fix("كلمت نور about sotto النهارده"), ("كلمت Nour about Sotto النهارده".into(), 2));
        // Accented Latin letters are word letters too.
        assert_eq!(fix("René met ren."), ("René met Wren.".into(), 1));
    }

    #[test]
    fn aliases_all_map_to_one_replacement() {
        // The exact bug Khairy hit: "my main email" fired, "my primary email"
        // didn't, because only one spoken phrase could point at an address.
        let entry = crate::config::DictEntry {
            spoken: "my main email".into(),
            replacement: "someone@example.com".into(),
            aliases: vec!["my primary email".into(), "my personal email".into()],
            enabled: true,
            kind: Some(EntryKind::Snippet),
        };
        let dict = vec![(
            entry.phrases().into_iter().map(str::to_string).collect::<Vec<_>>(),
            entry.replacement.clone(),
            entry.kind.unwrap(),
        )];
        for said in ["my main email", "My Primary Email", "my personal email"] {
            let (out, hits) = apply_dictionary(&format!("send it to {said} please"), &dict, EntryKind::Snippet);
            assert_eq!(out, "send it to someone@example.com please", "failed for {said:?}");
            assert_eq!(hits, 1);
        }
    }

    #[test]
    fn longer_phrase_wins_over_shorter_overlapping_one() {
        // "my email" and "my work email" both live. Matching the short one
        // first would leave "work" stranded next to the address.
        let entry = crate::config::DictEntry {
            spoken: "my email".into(),
            replacement: "personal@example.com".into(),
            aliases: vec!["my work email".into()],
            enabled: true,
            kind: Some(EntryKind::Snippet),
        };
        // phrases() sorts longest-first, which is what makes this safe.
        assert_eq!(entry.phrases(), vec!["my work email", "my email"]);
        let dict = vec![(
            entry.phrases().into_iter().map(str::to_string).collect::<Vec<_>>(),
            entry.replacement.clone(),
            entry.kind.unwrap(),
        )];
        let (out, _) = apply_dictionary("send my work email now", &dict, EntryKind::Snippet);
        assert_eq!(out, "send personal@example.com now");
    }

    #[test]
    fn disabled_entries_are_excluded_from_the_live_dictionary() {
        // Disabled entries must survive on disk but never fire. The filtering
        // happens in main::live_dictionary, so assert the shape it produces:
        // an empty phrase list is what "off" looks like to apply_dictionary.
        let dict: Vec<(Vec<String>, String, EntryKind)> = vec![];
        assert_eq!(apply_dictionary("my main email", &dict, EntryKind::Word), ("my main email".into(), 0));
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

    // ── tone resolution ──────────────────────────────────────────────
    fn test_polisher(default_tone: &str, app_tones: &[(&str, &str)]) -> Polisher {
        let cfg = crate::config::Config::default();
        let controls = crate::Controls::from_config(&cfg);
        *controls.tone.lock().unwrap() = default_tone.to_string();
        *controls.app_tones.lock().unwrap() =
            app_tones.iter().map(|(a, t)| (a.to_string(), t.to_string())).collect();
        Polisher::new(controls, cfg.llm.clone())
    }

    #[test]
    fn vocabulary_clause_names_heard_as_variants() {
        let entries = vec![
            VocabEntry { word: "Claude".into(), heard_as: vec!["clawed".into(), "code".into()], ..Default::default() },
            VocabEntry { word: "Sotto".into(), heard_as: vec![], ..Default::default() },
        ];
        assert_eq!(vocabulary_clause(&entries), "Claude (heard as clawed, code), Sotto");
    }

    #[test]
    fn vocabulary_clause_skips_blank_entries_and_empty_list_is_empty_string() {
        let entries = vec![VocabEntry { word: "  ".into(), heard_as: vec![], ..Default::default() }];
        assert_eq!(vocabulary_clause(&entries), "");
        assert_eq!(vocabulary_clause(&[]), "");
    }

    #[test]
    fn resolve_tone_prefers_exact_app_match_case_insensitively() {
        let p = test_polisher("Professional tone.", &[("Slack", "Casual and friendly.")]);
        assert_eq!(p.resolve_tone("slack", None), "Casual and friendly.");
    }

    #[test]
    fn resolve_tone_falls_back_to_default_when_no_app_match() {
        let p = test_polisher("Professional tone.", &[("Slack", "Casual and friendly.")]);
        assert_eq!(p.resolve_tone("chrome", None), "Professional tone.");
        assert_eq!(p.resolve_tone("", None), "Professional tone.");
    }

    #[test]
    fn resolve_tone_empty_default_yields_no_tone() {
        // Off by default, no app entries — matches today's behavior exactly.
        let p = test_polisher("", &[]);
        assert_eq!(p.resolve_tone("anything", None), "");
    }

    #[test]
    fn field_context_replaces_the_default_tone_but_never_an_app_tone() {
        let p = test_polisher("Professional tone.", &[("Slack", "Casual and friendly.")]);
        let tone = p.resolve_tone("Edge", Some("hey are you coming tonight"));
        assert_eq!(
            tone,
            "Match the register and formatting of the text the speaker is writing into, which so far \
             ends: \"hey are you coming tonight\". Use it only for style; never copy, continue or answer it."
        );
        // An explicit per-app tone still wins (#28 acceptance).
        assert_eq!(p.resolve_tone("slack", Some("Dear Ms. Rivera,")), "Casual and friendly.");
        // No context (flag off, a terminal, a password field, a slow read): today's lookup.
        assert_eq!(p.resolve_tone("Edge", None), "Professional tone.");
    }

    #[test]
    fn rewrite_guard_still_rejects_an_output_that_continues_the_field() {
        // The context steers style only (#28): its words turning up in the
        // output are new words like any others, so the take falls back to rules.
        let input = "i can bring the snacks at six";
        let continued = "Are you coming tonight? Bring your cousin too. I can bring the snacks at six.";
        assert_eq!(rewrite_guard(input, continued, &[]), Some("new-words"));
    }

    // ── #85: word passes keep line breaks ────────────────────────────
    #[test]
    fn rewrite_words_keeps_every_line_break() {
        let same = |s: &str| rewrite_words(s, |t| t.to_vec());
        assert_eq!(same("one  two\nthree"), "one two\nthree");
        assert_eq!(same("a\n\nb\n"), "a\n\nb\n");
        assert_eq!(same("\n\n  hi  "), "\n\nhi");
    }

    #[test]
    fn word_passes_keep_line_breaks() {
        assert_eq!(tier0("um first line\nsecond uh line"), "First line\nsecond line");
        assert_eq!(tier0("first\n\nsecond"), "First\n\nsecond");
        assert_eq!(tier0("\n\nuh hello there\n"), "\n\nHello there\n");
        assert_eq!(tier0("the\nthe end"), "The\nthe end"); // a break is not a stutter
        assert_eq!(normalize_numbers("I have twenty three\n\nboxes"), "I have 23\n\nboxes");
        assert_eq!(normalize_numbers("twenty\nthree"), "20\n3"); // a run never spans a break
        let targets = vec!["Claude".to_string(), "Sotto".to_string()];
        assert_eq!(apply_phonetic_corrections("ask clode\nthen soto", &targets).0, "ask Claude\nthen Sotto");
    }

    #[test]
    fn restore_breaks_puts_the_ai_output_back_on_its_lines() {
        let input = "the meeting went well and we agreed on the plan\nnext steps are simple";
        let output = "The meeting went well, and we agreed on the plan. Next steps are simple.";
        assert_eq!(
            restore_breaks(input, output).as_deref(),
            Some("The meeting went well, and we agreed on the plan.\nNext steps are simple.")
        );
        // A paragraph break the model kept, a line break it didn't.
        let input = "Dear team\n\nthe launch moves to friday\n23 people are coming";
        let output = "Dear team,\n\nThe launch moves to Friday. 23 people are coming.";
        assert_eq!(
            restore_breaks(input, output).as_deref(),
            Some("Dear team,\n\nThe launch moves to Friday.\n23 people are coming.")
        );
        // Breaks at either edge.
        assert_eq!(restore_breaks("\n\nhello there\n", "Hello there.").as_deref(), Some("\n\nHello there.\n"));
        // A boundary word the model dropped: no guess.
        assert_eq!(restore_breaks("call sam\nthen email jo", "Call Sam and email Jo."), None);
    }

    #[test]
    fn new_line_command_survives_the_rules_pipeline() {
        // Every word pass on: numbers, phonetic (with trained words), rules.
        let cfg = crate::config::Config::default();
        let controls = crate::Controls::from_config(&cfg);
        controls.polish_mode.store(PolishMode::Rules.as_u8(), Ordering::Relaxed);
        *controls.vocabulary.lock().unwrap() = vec![vocab("Sotto", &[])];
        let p = Polisher::new(controls, cfg.llm.clone());
        assert_eq!(p.polish("dear team new line see you soon").text, "Dear team\nsee you soon");
        assert_eq!(p.polish("um intro new paragraph ask soto for two things").text, "Intro\n\nask Sotto for 2 things");
    }

    // ── #65: AI rewrite guard ────────────────────────────────────────
    fn vocab(word: &str, heard_as: &[&str]) -> VocabEntry {
        VocabEntry { word: word.into(), heard_as: heard_as.iter().map(|h| h.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn rewrite_guard_accepts_a_cleanup() {
        let input = "so we should move the launch to friday and um tell the design team twenty three\nthanks";
        // Casing, punctuation, fillers, numbers and line breaks never count.
        let output = "So we should move the launch to Friday and tell the design team 23.\n\nThanks!";
        assert_eq!(rewrite_guard(input, output, &[]), None);
        // One word dropped is a resolved self-correction, not a rewrite.
        assert_eq!(rewrite_guard("meet me on tuesday, no, wednesday at noon", "Meet me on Wednesday at noon.", &[]), None);
        // A trained word may replace its mishearing.
        let v = [vocab("Sotto", &["soda"])];
        assert_eq!(rewrite_guard("open soda and start the recording", "Open Sotto and start the recording.", &v), None);
    }

    #[test]
    fn rewrite_guard_rejects_a_dropped_clause() {
        let input = "we need to finish the quarterly report by friday and then send the budget numbers to finance \
                     before the planning meeting";
        assert_eq!(rewrite_guard(input, "We need to finish the quarterly report by Friday.", &[]), Some("dropped-words"));
    }

    #[test]
    fn rewrite_guard_rejects_invented_words() {
        let input = "the garden needs water and the fence needs paint";
        let output = "The garden desperately needs fresh water, and the fence needs paint.";
        assert_eq!(rewrite_guard(input, output, &[]), None); // two new words: a respelling's worth
        let output = "The garden desperately needs fresh cold water, and the fence needs paint.";
        assert_eq!(rewrite_guard(input, output, &[]), Some("new-words"));
    }

    #[test]
    fn rewrite_guard_rejects_one_lost_word_of_another_script() {
        // Dropped, and translated: each a single lost word, which English may lose.
        let dropped = rewrite_guard("the build failed بكرة so we need a rollback plan", "The build failed. We need a rollback plan.", &[]);
        assert_eq!(dropped, Some("dropped-words"));
        let translated = rewrite_guard("the client قال the release is fine", "The client said the release is fine.", &[]);
        assert_eq!(translated, Some("dropped-words"));
        assert_eq!(rewrite_guard("the client قال the release is fine", "The client قال the release is fine.", &[]), None);
        // The filler the prompt names may still go, and so may one English word.
        assert_eq!(rewrite_guard("يعني the build failed again", "The build failed again.", &[]), None);
        assert_eq!(rewrite_guard("the build totally failed again today", "The build failed again today.", &[]), None);
    }

    #[test]
    fn a_take_with_arabic_script_never_goes_to_the_model() {
        let cfg = crate::config::Config::default();
        let controls = crate::Controls::from_config(&cfg);
        controls.polish_mode.store(PolishMode::Ai.as_u8(), Ordering::Relaxed);
        controls.ai_min_words.store(0, Ordering::Relaxed);
        let p = Polisher::new(controls, cfg.llm.clone());
        // One Arabic word in an English take, a code-switched take, an Arabic one.
        for (take, rules) in [
            ("um the build failed بكرة so we need a rollback plan", "The build failed بكرة so we need a rollback plan"),
            ("um the client قال the release is fine", "The client قال the release is fine"),
            ("um افتح الـ terminal وشغّل الـ build", "افتح الـ terminal وشغّل الـ build"),
        ] {
            let out = p.polish(take);
            assert_eq!((out.text.as_str(), out.tier, out.fallback), (rules, "rules", "arabic"), "{take}");
            assert!(!p.uses_ai_tier(take), "no Polishing state for {take}");
        }
        assert!(is_arabic('ب') && is_arabic('؟') && is_arabic('ﻻ') && !is_arabic('b') && !is_arabic('é'));
    }

    #[test]
    fn rewrite_guard_catches_the_worked_example_leaking() {
        let input = "please book the train tickets for the conference";
        let leaked = "Please book the train tickets for the conference. Can you ask Claude to fix this for me?";
        // Only when the example was actually sent, i.e. with a vocabulary.
        assert_eq!(rewrite_guard(input, leaked, &[vocab("Sotto", &[])]), Some("example-leak"));
        assert_eq!(rewrite_guard(input, leaked, &[vocab("Claude", &[])]), Some("example-leak"));
        // A trained "Claude" on its own is a correction, not a leak.
        assert_eq!(rewrite_guard("ask clawed about the tickets", "Ask Claude about the tickets.", &[vocab("Claude", &["clawed"])]), None);
    }

    #[test]
    fn polish_result_reports_the_tier_that_ran() {
        let cfg = crate::config::Config::default();
        let controls = crate::Controls::from_config(&cfg);
        let p = Polisher::new(controls.clone(), cfg.llm.clone());
        controls.polish_mode.store(PolishMode::Off.as_u8(), Ordering::Relaxed);
        let out = p.polish("um hello there");
        assert_eq!((out.tier, out.fallback), ("off", ""));
        controls.polish_mode.store(PolishMode::Rules.as_u8(), Ordering::Relaxed);
        let out = p.polish("um hello there");
        assert_eq!((out.tier, out.fallback), ("rules", ""));
        // AI mode, under the word threshold: never reaches the sidecar.
        controls.polish_mode.store(PolishMode::Ai.as_u8(), Ordering::Relaxed);
        controls.ai_min_words.store(50, Ordering::Relaxed);
        let out = p.polish("um hello there");
        assert_eq!((out.tier, out.fallback), ("rules", "short"));
    }

    #[test]
    fn ai_mode_fallback_gets_the_same_harper_fixes_as_rules() {
        let cfg = crate::config::Config::default();
        let controls = crate::Controls::from_config(&cfg);
        let p = Polisher::new(controls.clone(), cfg.llm.clone());
        let clip = "we shipped it , i think";
        controls.polish_mode.store(PolishMode::Rules.as_u8(), Ordering::Relaxed);
        let rules = p.polish(clip).text;
        assert_eq!(rules, "We shipped it, I think"); // Harper's fixes, not just tier0's
        controls.polish_mode.store(PolishMode::Ai.as_u8(), Ordering::Relaxed);
        controls.ai_min_words.store(50, Ordering::Relaxed); // short: never reaches the sidecar
        let ai = p.polish(clip);
        assert_eq!((ai.text.as_str(), ai.fallback), (rules.as_str(), "short"));
    }

    #[test]
    fn tone_has_no_effect_outside_ai_mode() {
        // PolishMode defaults to Rules — tone must not change the output,
        // since only the AI tier can re-voice a sentence (see
        // `polish_with_tone`).
        let p = test_polisher("Casual and friendly.", &[]);
        let with_app = p.polish_for("um hello there", "slack", Some("hey are you around"));
        let default = p.polish("um hello there");
        assert_eq!(with_app.text, default.text);
    }

    // ── #27: variable recognition ────────────────────────────────────
    fn cased(text: &str) -> String {
        let (masked, made) = apply_casing_commands(text);
        unmask(&masked, &made)
    }

    #[test]
    fn each_casing_joins_its_words() {
        assert_eq!(cased("camel case user name"), "userName");
        assert_eq!(cased("pascal case user name"), "UserName");
        assert_eq!(cased("snake case user name"), "user_name");
        assert_eq!(cased("kebab case user name"), "user-name");
        assert_eq!(cased("constant case max retries"), "MAX_RETRIES");
        // However the ASR capitalized the command or the words.
        assert_eq!(cased("Camel Case User name"), "userName");
        assert_eq!(cased("SNAKE case Get URL"), "get_url");
    }

    #[test]
    fn casing_takes_one_to_four_words_until_a_natural_break() {
        assert_eq!(cased("camel case count"), "count");
        assert_eq!(cased("camel case get user by id now"), "getUserById now", "four words at most");
        assert_eq!(cased("set camel case user name, then save."), "set userName, then save.");
        assert_eq!(cased("snake case is valid. Next line"), "is_valid. Next line");
        assert_eq!(cased("camel case foo snake case bar baz"), "foo bar_baz", "the next command is a break");
        assert_eq!(cased("camel case user\nname"), "user\nname", "a line break is a break");
        assert_eq!(cased("camel case user (name)"), "user (name)", "punctuation before a word is a break");
        assert_eq!(cased("\"camel case user name\""), "\"userName\"");
    }

    #[test]
    fn a_casing_command_with_nothing_to_take_stays_prose() {
        for prose in ["I like camel case.", "we use camel case", "camel case, mostly", "camel, case user", "a case study"] {
            assert_eq!(cased(prose), prose);
        }
    }

    #[test]
    fn casing_mixes_with_prose_and_keeps_digits() {
        assert_eq!(cased("rename camel case user name, then run the tests"), "rename userName, then run the tests");
        assert_eq!(cased("snake case retry count 3"), "retry_count_3");
        assert_eq!(cased("camel case user 2 id"), "user2Id");
        assert_eq!(cased("constant case http 404"), "HTTP_404");
        assert_eq!(cased("camel case pi 3.14"), "pi 3.14", "a word that isn't plain letters or digits is a break");
    }

    #[test]
    fn casing_leaves_arabic_untouched() {
        let arabic = "افتح الملف وشغّل الـ build";
        assert_eq!(cased(arabic), arabic);
        assert_eq!(cased("camel case مرحبا يا صاحبي"), "camel case مرحبا يا صاحبي");
        assert_eq!(cased("سمّيه camel case user name."), "سمّيه userName.");
    }

    #[test]
    fn remask_needs_every_identifier_back_exactly() {
        let made = vec!["userName".to_string(), "userName".to_string(), "max_retries".to_string()];
        let back = remask("Set userName to userName and max_retries to 3.", &made).unwrap();
        assert_eq!(unmask(&back, &made), "Set userName to userName and max_retries to 3.");
        assert!(!back.contains("userName"), "masked again, so the snippet pass can't reach it");
        assert_eq!(remask("Set UserName to userName and max_retries to 3.", &made), Err("identifiers"));
        assert_eq!(remask("Set userName and max retries to 3.", &made), Err("identifiers"));
        assert_eq!(remask("Nothing to keep.", &[]), Ok("Nothing to keep.".to_string()));
        // Backticks the model put around an identifier aren't typed.
        let marked = remask("Rename it to `userName`, then `userName` and `max_retries`.", &made).unwrap();
        assert_eq!(unmask(&marked, &made), "Rename it to userName, then userName and max_retries.");
    }

    /// Rules mode, variable recognition on for VS Code, and every pass that
    /// could respell an identifier armed: a Word entry for "max", a snippet,
    /// and "Claude" trained for the phonetic corrector.
    fn code_polisher(editors: &[&str]) -> Polisher {
        let cfg = crate::config::Config::default();
        let controls = crate::Controls::from_config(&cfg);
        controls.polish_mode.store(PolishMode::Rules.as_u8(), Ordering::Relaxed);
        *controls.code_editors.lock().unwrap() = editors.iter().map(|e| e.to_string()).collect();
        *controls.vocabulary.lock().unwrap() = vec![vocab("Claude", &[])];
        *controls.dictionary.lock().unwrap() = vec![
            (vec!["max".into()], "Max".into(), EntryKind::Word),
            (vec!["retries".into()], "retries (see the runbook)".into(), EntryKind::Snippet),
        ];
        Polisher::new(controls, cfg.llm.clone())
    }

    #[test]
    fn identifiers_survive_harper_tier0_the_dictionary_and_the_phonetic_corrector() {
        let p = code_polisher(&["VS Code"]);
        // Not capitalized at the start, not respelled by Harper.
        assert_eq!(p.polish_for("camel case user name.", "VS Code", None).text, "userName.");
        assert_eq!(p.polish_for("set camel case get user by id, then save", "vs code", None).text, "Set getUserById, then save");
        // The dictionary would make it Max_retries and append the snippet.
        assert_eq!(p.polish_for("constant case max retries.", "VS Code", None).text, "MAX_RETRIES.");
        assert_eq!(p.polish_for("snake case max retries.", "VS Code", None).text, "max_retries.");
        // The phonetic corrector would make it Claude.
        assert_eq!(p.polish_for("camel case clode.", "VS Code", None).text, "clode.");
        // Harper doesn't start the sentence on the word after it either.
        assert_eq!(
            p.polish_for("camel case user name, is what the field is called in the form.", "VS Code", None).text,
            "userName, is what the field is called in the form."
        );
        // Spoken numbers run first, so they end up as digits in the identifier.
        assert_eq!(p.polish_for("camel case version two.", "VS Code", None).text, "version2.");
    }

    #[test]
    fn casing_only_fires_in_a_listed_app() {
        let p = code_polisher(&["VS Code"]);
        assert_eq!(p.polish_for("camel case user name", "Notepad", None).text, "Camel case user name");
        assert_eq!(p.polish_for("camel case user name", "", None).text, "Camel case user name");
        assert_eq!(p.polish("camel case user name").text, "Camel case user name", "no app: repolish_copy");
        // The other passes still run as normal outside the editor.
        assert_eq!(p.polish_for("camel case clode", "Notepad", None).text, "Camel case Claude");
        // Off (the default config): the list is empty until the flag is on.
        let cfg = crate::config::Config::default();
        let controls = crate::Controls::from_config(&cfg);
        assert!(controls.code_editors.lock().unwrap().is_empty());
        let p = Polisher::new(controls, cfg.llm.clone());
        assert_eq!(p.polish_for("camel case user name", "VS Code", None).text, "Camel case user name");
    }

    #[test]
    fn arabic_dictation_is_untouched_in_a_code_editor() {
        let p = code_polisher(&["VS Code"]);
        let arabic = "افتح الملف وشغّل الـ build";
        assert_eq!(p.polish_for(arabic, "VS Code", None).text, p.polish_for(arabic, "Notepad", None).text);
    }
}
