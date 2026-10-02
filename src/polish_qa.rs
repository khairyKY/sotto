//! `sotto.exe --polish-qa [path]` (#29): a fixed set of invented dictations
//! through the real polish chain in AI mode (rules, the sidecar model, the
//! rewrite guard), each checked against properties of its output rather than
//! an exact string: a model's wording moves, what must hold of it doesn't.
//!
//! The set is `tests/polish-qa/cases.jsonl`, one JSON object per line:
//!
//! | field        | meaning                                                          |
//! | ------------ | ---------------------------------------------------------------- |
//! | `id`, `cat`  | name and category, for the table                                 |
//! | `input`      | what the speech engine hands to polish                           |
//! | `app`        | dictated "into" this app (casing commands need a code editor)    |
//! | `vocabulary` | trained words for this case, shaped like `polish.vocabulary`     |
//! | `lose`       | content words of the input it may drop (default 1)               |
//! | `max_new`    | content words the input never had (default 0)                    |
//! | `ends_punct` | ends like a sentence (default true)                              |
//! | `caps`       | sentences and the pronoun "I" are capitalized (default true)     |
//! | `punct`      | at least this many punctuation marks (a run-on got punctuated)   |
//! | `contains`   | exact terms that must be there                                   |
//! | `lacks`      | whole words or phrases that must be gone, in any case; a mark    |
//! |              | with no letter or digit in it (a backtick) is found anywhere     |
//! | `breaks`     | exactly this many line breaks                                    |
//! | `limit`      | why a failing case is a known limit, not a regression            |
//!
//! The share of Arabic to Latin words must also hold, always (`language`): a
//! translated or arabized take fails it.

use crate::config::{Config, PolishMode, VocabEntry};
use crate::polish::{content_words, find_whole_ci, is_arabic, PolishResult, Polisher};
use anyhow::Context;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Instant;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)] // a misspelled property must not pass as "unchecked"
pub struct Case {
    id: String,
    cat: String,
    input: String,
    #[serde(default)]
    app: String,
    #[serde(default)]
    vocabulary: Vec<VocabEntry>,
    #[serde(default = "one")]
    lose: usize,
    #[serde(default)]
    max_new: usize,
    #[serde(default = "yes")]
    ends_punct: bool,
    #[serde(default = "yes")]
    caps: bool,
    #[serde(default)]
    punct: usize,
    #[serde(default)]
    contains: Vec<String>,
    #[serde(default)]
    lacks: Vec<String>,
    breaks: Option<usize>,
    #[serde(default)]
    limit: String,
}

fn one() -> usize {
    1
}

fn yes() -> bool {
    true
}

/// `(kept, total, new)`: how many of `input`'s content words `output` still
/// has, out of how many, and how many content words it has that `input` never
/// did. A word said again is a repeat, not a new word. Same word list as
/// `polish::rewrite_guard`, counted here on its own so the check doesn't lean
/// on the guard it is checking.
fn word_delta(input: &str, output: &str) -> (usize, usize, usize) {
    let mut have: HashMap<String, usize> = HashMap::new();
    for w in content_words(input) {
        *have.entry(w).or_default() += 1;
    }
    let total = have.values().sum();
    let (mut kept, mut new) = (0, 0);
    for w in content_words(output) {
        match have.get_mut(&w) {
            Some(n) if *n > 0 => {
                *n -= 1;
                kept += 1;
            }
            Some(_) => {}
            None => new += 1,
        }
    }
    (kept, total, new)
}

/// Ends on sentence punctuation, Latin or Arabic, a closing quote or bracket
/// after it aside.
fn ends_sentence(s: &str) -> bool {
    s.trim_end().trim_end_matches(['"', '\u{201D}', '\'', ')', '`']).ends_with(['.', '!', '?', '؟', '…'])
}

/// The first letter, the first letter after each sentence end, and the
/// pronoun "I" are capitals. Letters with no case (Arabic) pass, and so does
/// the word after an abbreviation ("7 a.m. sharp").
fn capitalized(s: &str) -> bool {
    let mut opens = true;
    for tok in s.split_whitespace() {
        let word = tok.trim_matches(|c: char| !c.is_alphanumeric());
        let lower_start = word.chars().next().is_some_and(char::is_lowercase);
        if (opens && lower_start) || word == "i" || word.starts_with("i'") || word.starts_with("i’") {
            return false;
        }
        let end = tok.trim_end_matches(['"', '\u{201D}', '\'', ')']);
        opens = end.ends_with(['.', '!', '?']) && !end[..end.len() - 1].contains('.');
    }
    true
}

/// `(arabic, all)`: how many of `s`'s words are in Arabic script, out of how
/// many words it has.
fn arabic_words(s: &str) -> (usize, usize) {
    let words = s.split_whitespace().filter(|w| w.chars().any(char::is_alphabetic));
    words.fold((0, 0), |(a, n), w| (a + usize::from(w.chars().any(is_arabic)), n + 1))
}

/// `output` is in the language(s) `input` was: it has as many Arabic words as
/// `input`'s Arabic share predicts for a text its length, give or take 2 (or
/// a tenth of its words). A dropped filler moves that a little; a translated
/// or arabized clause moves it a lot.
fn language_kept(input: &str, output: &str) -> bool {
    let ((a0, n0), (a1, n1)) = (arabic_words(input), arabic_words(output));
    (a1 * n0).abs_diff(a0 * n1) <= (n1 / 10).max(2) * n0
}

/// Every property of `case` that `output` breaks, named. Empty: it passes.
fn failures(case: &Case, output: &str) -> Vec<String> {
    let mut bad = Vec::new();
    let (kept, total, new) = word_delta(&case.input, output);
    if total - kept > case.lose {
        bad.push(format!("lost {} > {}", total - kept, case.lose));
    }
    if new > case.max_new {
        bad.push(format!("new {new} > {}", case.max_new));
    }
    if case.ends_punct && !ends_sentence(output) {
        bad.push("ends_punct".into());
    }
    if case.caps && !capitalized(output) {
        bad.push("caps".into());
    }
    let marks = output.chars().filter(|c| ".,;:!?؟،".contains(*c)).count();
    if marks < case.punct {
        bad.push(format!("punct {marks} < {}", case.punct));
    }
    if !language_kept(&case.input, output) {
        bad.push("language".into());
    }
    bad.extend(case.contains.iter().filter(|t| !output.contains(t.as_str())).map(|t| format!("missing {t:?}")));
    let has = |t: &str| if t.contains(char::is_alphanumeric) { find_whole_ci(output, t).is_some() } else { output.contains(t) };
    bad.extend(case.lacks.iter().filter(|t| has(t.as_str())).map(|t| format!("still has {t:?}")));
    let breaks = output.matches('\n').count();
    if let Some(n) = case.breaks.filter(|&n| n != breaks) {
        bad.push(format!("breaks {breaks} != {n}"));
    }
    bad
}

/// Run every case in `path` through `polish` and print one row per case
/// (tier that produced the text, the fallback that fired, content words
/// kept/said + new, the verdict), its input and output, then the totals per
/// category. Returns how many cases failed; one that fails with a `limit` is
/// a known limit, not a failure. `println!`, like `bug_reports::replay`: the
/// table is the result, and the dictations are invented.
pub fn run(path: &Path, mut polish: impl FnMut(&Case) -> PolishResult) -> anyhow::Result<usize> {
    let body = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let cases: Vec<Case> = body
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, l)| serde_json::from_str(l).with_context(|| format!("{}:{}", path.display(), i + 1)))
        .collect::<anyhow::Result<_>>()?;

    // Per category, in file order: pass, fail, limit, fixed, reached the AI.
    let mut cats: Vec<(&str, [usize; 5])> = Vec::new();
    let mut fallbacks: Vec<(&str, usize)> = Vec::new();
    println!("{:<12} {:<6} {:<14} {:>6} {:>9}  result", "id", "tier", "fallback", "ms", "words");
    for case in &cases {
        let t = Instant::now();
        let out = polish(case);
        let ms = t.elapsed().as_millis();
        let bad = failures(case, &out.text);
        let (slot, verdict) = match (bad.is_empty(), case.limit.is_empty()) {
            (true, true) => (0, "pass".to_string()),
            (false, true) => (1, format!("FAIL: {}", bad.join("; "))),
            (false, false) => (2, format!("limit: {} ({})", bad.join("; "), case.limit)),
            (true, false) => (3, format!("PASSES NOW, drop its limit ({})", case.limit)),
        };
        let fallback = if out.fallback.is_empty() { "-" } else { out.fallback };
        let (kept, total, new) = word_delta(&case.input, &out.text);
        let words = format!("{kept}/{total}+{new}");
        println!("{:<12} {:<6} {fallback:<14} {ms:>6} {words:>9}  {verdict}", case.id, out.tier);
        println!("    in   {:?}", case.input);
        println!("    out  {:?}", out.text);

        if !cats.iter().any(|(c, _)| *c == case.cat) {
            cats.push((case.cat.as_str(), [0; 5]));
        }
        let counts = &mut cats.iter_mut().find(|(c, _)| *c == case.cat).unwrap().1;
        counts[slot] += 1;
        counts[4] += usize::from(out.tier == "ai");
        if !out.fallback.is_empty() {
            match fallbacks.iter_mut().find(|(f, _)| *f == out.fallback) {
                Some((_, n)) => *n += 1,
                None => fallbacks.push((out.fallback, 1)),
            }
        }
    }

    let mut sum = [0; 5];
    println!("\n{:<14} {:>5} {:>5} {:>5} {:>5} {:>5} {:>6}", "category", "cases", "pass", "FAIL", "limit", "fixed", "by AI");
    let row = |name: &str, n: &[usize; 5]| {
        println!("{name:<14} {:>5} {:>5} {:>5} {:>5} {:>5} {:>6}", n[..4].iter().sum::<usize>(), n[0], n[1], n[2], n[3], n[4])
    };
    for (cat, n) in &cats {
        row(cat, n);
        sum.iter_mut().zip(n).for_each(|(s, n)| *s += n);
    }
    row("total", &sum);
    let fired: Vec<String> = fallbacks.iter().map(|(f, n)| format!("{f} {n}")).collect();
    println!("kept the rules result: {}", if fired.is_empty() { "never".into() } else { fired.join(", ") });
    Ok(sum[1])
}

/// The CLI mode. The set is fixed, so the user's dictionary, vocabulary and
/// tones must not move its results: every setting is the default, and only
/// the sidecar's own (its port, mostly) come from `config.toml`. Casing
/// commands are switched on, for the cases dictated into a code editor.
/// Exits non-zero when a case fails, so it works as a regression check.
pub fn run_cli(path: Option<String>) -> anyhow::Result<()> {
    anyhow::ensure!(crate::llm::Llm::is_available(), "AI polish isn't installed (model or llama-server missing)");
    let llm = Config::load_or_init().unwrap_or_default().llm;
    let controls = crate::Controls::from_config(&Config { variable_recognition: true, ..Config::default() });
    controls.polish_mode.store(PolishMode::Ai.as_u8(), Ordering::Relaxed);
    let polisher = Polisher::new(controls.clone(), llm);
    let path = path.map(PathBuf::from).unwrap_or_else(|| PathBuf::from("tests/polish-qa/cases.jsonl"));
    let failed = run(&path, |case| {
        *controls.vocabulary.lock().unwrap() = case.vocabulary.clone();
        polisher.polish_for(&case.input, &case.app, None)
    })?;
    anyhow::ensure!(failed == 0, "{failed} case(s) failed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case(json: &str) -> Case {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn word_delta_counts_kept_and_new_content_words_only() {
        // Fillers, function words, casing and punctuation never count.
        assert_eq!(word_delta("um the launch moved to friday", "The launch moved to Friday."), (3, 3, 0));
        // A dropped clause, and words from nowhere.
        assert_eq!(word_delta("the launch moved to friday", "The launch is delayed, sorry."), (1, 3, 2));
        // Said once, written twice: a repeat, not a new word.
        assert_eq!(word_delta("ship the build", "Ship the build, the build."), (2, 2, 0));
        // Said twice, written once: one of the two is gone.
        assert_eq!(word_delta("the report, the report is late", "The report is late."), (2, 3, 0));
        // Numbers may be reformatted freely.
        assert_eq!(word_delta("twenty three people came", "23 people came."), (2, 2, 0));
        // An Arabic word swapped for its English meaning is one lost, one new.
        assert_eq!(word_delta("الاجتماع اتأجل", "الاجتماع postponed"), (1, 2, 1));
    }

    #[test]
    fn ends_sentence_takes_either_script_and_a_closing_quote() {
        for ok in ["Done.", "Really?", "Go!", "هتيجي بكرة؟", "She said \"go.\"", "Wait…", "Done.  \n"] {
            assert!(ends_sentence(ok), "{ok:?}");
        }
        for bad in ["Done", "Done,", "and then", "هتيجي بكرة", "", "userName"] {
            assert!(!ends_sentence(bad), "{bad:?}");
        }
    }

    #[test]
    fn capitalized_checks_sentence_starts_and_the_pronoun() {
        for ok in [
            "We left. Then it rained.",
            "I think I'm late.",
            "Meet at 7 a.m. sharp, e.g. by the door.",
            "انا رايح الشغل. هكلمك بعدين.",
            "She asked \"why?\" And left.",
            "Set userName, then save.",
        ] {
            assert!(capitalized(ok), "{ok:?}");
        }
        for bad in ["we left.", "We left. then it rained.", "So i think so.", "Well, i'm late.", "\"quoted start\" is lower."] {
            assert!(!capitalized(bad), "{bad:?}");
        }
    }

    #[test]
    fn language_kept_fails_a_translated_or_arabized_take() {
        let mixed = "انا عملت push للـ branch الجديد بس الـ build فشل";
        assert!(language_kept(mixed, "انا عملت push للـ branch الجديد، بس الـ build فشل."));
        // A dropped English filler moves the share a little: still kept.
        assert!(language_kept("um انا مش عارف هنعمل ايه في الموضوع ده", "انا مش عارف هنعمل ايه في الموضوع ده."));
        assert!(!language_kept(mixed, "I pushed the new branch but the build failed."), "translated");
        assert!(!language_kept(mixed, "انا عملت بوش للبرانش الجديد بس البيلد فشل"), "arabized");
        assert!(!language_kept("the build failed again today", "البيلد فشل تاني النهارده"));
        assert!(language_kept("the build failed", "The build failed."));
        // Two fillers gone from a short take is not a language change.
        assert!(language_kept("um انا مش عارف uh هنعمل ايه", "انا مش عارف هنعمل ايه."));
        assert_eq!(arabic_words("12, انا said 34!"), (1, 2));
    }

    #[test]
    fn failures_names_each_broken_property() {
        let c = case(
            r#"{"id":"x","cat":"t","input":"um send the summary to Dana new line thanks","lose":2,"punct":2,
                "contains":["Dana"],"lacks":["um","new line","`"],"breaks":1}"#,
        );
        // "summary" holds "um", but not as a word.
        assert!(failures(&c, "Send the summary to Dana.\nThanks.").is_empty());
        assert_eq!(
            failures(&c, "um, i sent a note to dana"),
            [
                "lost 5 > 2",
                "new 2 > 0",
                "ends_punct",
                "caps",
                "punct 1 < 2",
                "missing \"Dana\"",
                "still has \"um\"",
                "breaks 0 != 1"
            ]
        );
        assert_eq!(failures(&c, "أرسل المسودة إلى دانا.\nشكرا."), ["lost 6 > 2", "new 5 > 0", "language", "missing \"Dana\""]);
        // A mark is found anywhere, a word only as a whole word.
        assert_eq!(failures(&c, "Send the `summary` to Dana.
Thanks."), ["still has \"`\""]);
    }

    #[test]
    fn a_case_defaults_to_the_strict_properties_and_refuses_an_unknown_one() {
        let c = case(r#"{"id":"x","cat":"t","input":"hello there"}"#);
        assert!((c.lose, c.max_new, c.ends_punct, c.caps, c.punct, c.breaks) == (1, 0, true, true, 0, None));
        assert!(serde_json::from_str::<Case>(r#"{"id":"x","cat":"t","input":"hi","contain":["hi"]}"#).is_err());
    }

    fn result(text: &str, tier: &'static str, fallback: &'static str) -> PolishResult {
        PolishResult { text: text.into(), corrected_words: 0, dict_hits: 0, notable: vec![], tier, fallback }
    }

    #[test]
    fn run_counts_failures_and_lets_a_known_limit_through() {
        let path = std::env::temp_dir().join(format!("sotto-polish-qa-test-{}.jsonl", std::process::id()));
        std::fs::write(
            &path,
            concat!(
                r#"{"id":"a","cat":"one","input":"um hello there"}"#,
                "\n\n",
                r#"{"id":"b","cat":"one","input":"send it to tuesday no wednesday","lacks":["tuesday"],"limit":"the guard keeps both"}"#,
                "\n",
                r#"{"id":"c","cat":"two","input":"good morning"}"#,
                "\n",
            ),
        )
        .unwrap();
        let lost = |_: &Case| result("Okay.", "rules", "short");
        assert_eq!(run(&path, lost).unwrap(), 2, "a and c lose every word; b fails too, but is a known limit");
        let echo = |c: &Case| {
            let said = c.input.trim_start_matches("um ");
            result(&format!("{}{}.", said[..1].to_uppercase(), &said[1..]), "ai", "")
        };
        assert_eq!(run(&path, echo).unwrap(), 0, "all three keep their words; b is still a limit");
        std::fs::write(&path, "{\"id\":\"a\"}\n").unwrap();
        assert!(run(&path, echo).is_err(), "a broken line stops the run, it isn't skipped");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_shipped_set_parses_with_unique_ids() {
        let cases: Vec<Case> = include_str!("../tests/polish-qa/cases.jsonl")
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l}")))
            .collect();
        assert!(cases.len() >= 60, "{} cases", cases.len());
        let mut ids: Vec<&str> = cases.iter().map(|c| c.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), cases.len(), "duplicate ids");
    }
}
