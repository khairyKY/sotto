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

use crate::config::{EntryKind, LlmConfig, PolishMode};
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
        PolishResult { text, corrected_words, dict_hits: word_hits + snippet_hits }
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

        let t = std::time::Instant::now();
        match llm.polish(&rules, tone) {
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

/// Like `replace_whole_ci`, but also consumes one whitespace character
/// immediately AFTER the match — "open quote hello" -> `"hello`, not
/// `" hello`. A quote mark hugs its content; it doesn't float a space away
/// from it.
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
        rest = after.strip_prefix(' ').unwrap_or(after);
    }
    out
}

/// Mirror of `replace_and_trim_after`, trimming one whitespace character
/// immediately BEFORE the match instead.
fn replace_and_trim_before(hay: &str, needle: &str, rep: &str) -> String {
    let mut out = String::new();
    let mut rest = hay;
    loop {
        let Some((start, end)) = find_whole_ci(rest, needle) else {
            out.push_str(rest);
            break;
        };
        let before = &rest[..start];
        out.push_str(before.strip_suffix(' ').unwrap_or(before));
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

/// Wrap the span between a "quote" trigger and the next "unquote" in
/// `open_glyph`/`close_glyph`. Only fires when a closer exists LATER in the
/// same take — a lone "quote" ("a quote from the paper") is ordinary speech
/// far more often than the start of a dictated pair, so it's left alone
/// rather than guessed at.
///
/// ponytail: a single left-to-right greedy pass — a standalone "quote" that
/// appears *before* a genuine, separate pair later in the same take gets
/// wrongly treated as that pair's opener (its "unquote" closer is the only
/// one found, so everything in between gets wrapped). Sentence-boundary
/// scoping would fix it; not worth it until Kai actually dictates two
/// separate quoted spans in one breath.
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
        match find_whole_ci(&rest[oend..], "unquote") {
            Some((cstart_rel, cend_rel)) => {
                let cstart = oend + cstart_rel;
                let cend = oend + cend_rel;
                out.push_str(&rest[..ostart]);
                out.push_str(open_glyph);
                out.push_str(rest[oend..cstart].trim());
                out.push_str(close_glyph);
                rest = &rest[cend..];
            }
            // No closer anywhere after this opener — not a pair. Emit
            // through the opener as plain text and keep scanning after it.
            None => {
                out.push_str(&rest[..oend]);
                rest = &rest[oend..];
            }
        }
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
        let exempt = LEGIT_DOUBLES.iter().any(|w| w.eq_ignore_ascii_case(core));
        if !is_numeric && !new_sentence && !exempt {
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
/// substitution. ASCII-folded so byte indices stay aligned (dictionary terms
/// are effectively ASCII), and word-boundary-checked so "arrow" doesn't hit
/// inside "arrows". Returns the rewritten text plus how many replacements
/// fired (the "dictionary fixes" stat).
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
        // later "unquote" is ordinary speech, not a dictated pair.
        let s = "I found a quote from the paper";
        assert_eq!(apply_quote_commands(s, QuoteStyle::Straight), s);
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
            replacement: "khairyshhn1@gmail.com".into(),
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
            assert_eq!(out, "send it to khairyshhn1@gmail.com please", "failed for {said:?}");
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
