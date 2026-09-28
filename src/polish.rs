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
use harper_core::linting::{Lint, LintGroup, LintKind, Linter};
use harper_core::spell::{Dictionary, FstDictionary};
use harper_core::{Dialect, Document, remove_overlaps};
use std::cell::RefCell;
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
        self.polish_with_tone(raw, &tone)
    }

    /// Same as `polish`, but resolves a tone for `app` first: an exact
    /// case-insensitive match in the per-app overrides wins, else the default
    /// tone, else no tone at all. Used on the live delivery path, where the
    /// focused app is known.
    pub fn polish_for(&self, raw: &str, app: &str) -> PolishResult {
        let tone = self.resolve_tone(app);
        self.polish_with_tone(raw, &tone)
    }

    /// Tone-lookup order: per-app override → default tone → "" (no
    /// instruction, prompt stays byte-identical to before tones existed).
    fn resolve_tone(&self, app: &str) -> String {
        if !app.is_empty() {
            let app_tones = self.controls.app_tones.lock().unwrap();
            if let Some((_, tone)) = app_tones.iter().find(|(a, _)| a.eq_ignore_ascii_case(app)) {
                return tone.clone();
            }
        }
        self.controls.tone.lock().unwrap().clone()
    }

    fn polish_with_tone(&self, raw: &str, tone: &str) -> PolishResult {
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
        let dict = self.controls.dictionary.lock().unwrap();
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
        // the single-word trained-vocabulary entries — see
        // `apply_phonetic_corrections`.
        let phoneticized;
        let mut notable: Vec<(String, String)> = Vec::new();
        let raw = if self.controls.phonetic_correction.load(Ordering::Relaxed) {
            let targets: Vec<String> = {
                let vocab = self.controls.vocabulary.lock().unwrap();
                vocab
                    .iter()
                    .map(|e| e.word.clone())
                    .filter(|w| w.split_whitespace().count() == 1)
                    .collect()
            };
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

        let cleaned = match self.mode() {
            // Tone rewrites voice, which only the AI tier can do — Rules just
            // strips/fixes, it can't re-voice a sentence. `tone` is unused on
            // these two branches on purpose, so Off/Rules stay byte-identical
            // to before tones existed.
            PolishMode::Off => raw.trim().to_string(),
            PolishMode::Rules => self.apply_harper(&tier0(raw)),
            PolishMode::Ai => self.polish_ai(raw, tone),
        };
        // Diff against the word-dictionary-corrected raw, not the ASR-
        // original one, so a Word-entry substitution is never miscounted as
        // a tier0/AI correction — and BEFORE the snippet pass below, so
        // corrected-words and dict-hits don't double-count the same word.
        let corrected_words = changed_words(raw, &cleaned);
        let (text, snippet_hits) = if dict.is_empty() || !replacements_on {
            (cleaned, 0)
        } else {
            apply_dictionary(&cleaned, &dict, EntryKind::Snippet)
        };
        PolishResult { text, corrected_words, dict_hits: word_hits + snippet_hits, notable }
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
    fn polish_ai(&self, raw: &str, tone: &str) -> String {
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

        let vocabulary = vocabulary_clause(&self.controls.vocabulary.lock().unwrap());
        let t = std::time::Instant::now();
        match llm.polish(&rules, tone, &vocabulary) {
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
    /// Notable substitutions worth surfacing to the user (currently the
    /// phonetic corrector's `(heard, corrected)` pairs) — consumed by the
    /// "something smart just happened" flyout, ignored elsewhere.
    pub notable: Vec<(String, String)>,
}

fn word_count(s: &str) -> usize {
    s.split_whitespace().count()
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
    collapse_space_around_breaks(&text)
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
                if group_has_small && !(20..=99).contains(&group) {
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
                if group_has_small && !(20..=99).contains(&group) {
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
    let toks: Vec<&str> = text.split_whitespace().collect();
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

        let suffix = if run.ordinal { ordinal_suffix(run.value) } else { "" };
        out.push(format!("{lead}{}{suffix}{trail_last}", run.value));
        i += run.consumed;
    }
    out.join(" ")
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
fn find_whole_ci(hay: &str, needle: &str) -> Option<(usize, usize)> {
    let hay_lc = hay.to_ascii_lowercase();
    let needle_lc = needle.to_ascii_lowercase();
    let hb = hay_lc.as_bytes();
    let is_word = |b: u8| b.is_ascii_alphanumeric();
    let mut i = 0;
    while i <= hay_lc.len() {
        let start = i + hay_lc[i..].find(&needle_lc)?;
        let end = start + needle_lc.len();
        let left_ok = start == 0 || !is_word(hb[start - 1]);
        let right_ok = end == hb.len() || !is_word(hb[end]);
        if left_ok && right_ok {
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

/// Tier 0 rules cleanup. `split_whitespace` also collapses runs of spaces and
/// trims, so filtering + rejoining handles whitespace normalization for free.
fn tier0(raw: &str) -> String {
    let kept: Vec<&str> = raw.split_whitespace().filter(|t| !is_filler(t)).collect();
    // Stutter collapse runs AFTER filler stripping: by now "uh uh uh" is
    // already gone, so any run of identical tokens left is genuine word
    // repetition ("the the store", "I I I think"), not disfluency.
    let deduped = collapse_stutters(&kept);
    capitalize_first(&deduped.join(" "))
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
        let new_sentence = out.last().is_some_and(|p: &&str| p.ends_with(['.', '!', '?']));
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
        if !is_numeric && !new_sentence && !starts_quoted && !exempt {
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

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
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
/// `targets` are single words (proper nouns/jargon) the user trained. Returns
/// the corrected text plus the `(heard, corrected)` pairs that fired.
fn apply_phonetic_corrections(text: &str, targets: &[String]) -> (String, Vec<(String, String)>) {
    // Only targets of 4+ letters — short words are too collision-prone to
    // phonetic-match safely, and exact dictionary entries cover those anyway.
    let coded: Vec<(&str, String)> = targets
        .iter()
        .filter(|t| t.chars().filter(|c| c.is_ascii_alphabetic()).count() >= 4)
        .map(|t| (t.as_str(), soundex(t)))
        .filter(|(_, c)| !c.is_empty())
        .collect();
    if coded.is_empty() {
        return (text.to_string(), Vec::new());
    }
    // Harper's curated dictionary (already loaded for Rules mode) knows which
    // tokens are real English words.
    let english = FstDictionary::curated();

    let mut fired = Vec::new();
    let out = text
        .split_whitespace()
        .map(|tok| {
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
            // A real English word is what the speaker said, not a mishearing
            // — common ("said", "called") or not ("culled", "trie"). Only
            // non-words ("clode") and names ("Soto") are fair game.
            if english.get_word_metadata_str(core).is_some_and(|m| m.common || !m.is_proper_noun()) {
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
        .collect::<Vec<_>>()
        .join(" ");
    (out, fired)
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
        assert_eq!(normalize_numbers("first"), "1st");
        assert_eq!(normalize_numbers("second"), "2nd");
        assert_eq!(normalize_numbers("third"), "3rd");
        assert_eq!(normalize_numbers("fourth"), "4th");
        assert_eq!(normalize_numbers("eleventh"), "11th"); // 11-13 are always -th
        assert_eq!(normalize_numbers("twenty first"), "21st");
        assert_eq!(normalize_numbers("twentieth"), "20th");
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
        assert_eq!(p.resolve_tone("slack"), "Casual and friendly.");
    }

    #[test]
    fn resolve_tone_falls_back_to_default_when_no_app_match() {
        let p = test_polisher("Professional tone.", &[("Slack", "Casual and friendly.")]);
        assert_eq!(p.resolve_tone("chrome"), "Professional tone.");
        assert_eq!(p.resolve_tone(""), "Professional tone.");
    }

    #[test]
    fn resolve_tone_empty_default_yields_no_tone() {
        // Off by default, no app entries — matches today's behavior exactly.
        let p = test_polisher("", &[]);
        assert_eq!(p.resolve_tone("anything"), "");
    }

    #[test]
    fn tone_has_no_effect_outside_ai_mode() {
        // PolishMode defaults to Rules — tone must not change the output,
        // since only the AI tier can re-voice a sentence (see
        // `polish_with_tone`).
        let p = test_polisher("Casual and friendly.", &[]);
        let with_app = p.polish_for("um hello there", "slack");
        let default = p.polish("um hello there");
        assert_eq!(with_app.text, default.text);
    }
}
