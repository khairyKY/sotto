//! Local "flag this transcription" log for the History page. One JSON line
//! per flag, appended to `data_dir()/bug-reports.jsonl` alongside
//! `stats.jsonl` — the transcript plus whatever config context was live at
//! flag time. Purely local, like everything else here: nothing is sent
//! anywhere. Kai reviews the file himself and files a real GitHub issue by
//! hand when one's worth reporting.
//!
//! Each flag also keeps the raw ASR transcript, so `sotto.exe --replay-flags`
//! can re-run the whole corpus through the current polish chain after a
//! polish-layer change and diff against the last run (#8).

use crate::config;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone)]
pub struct BugReport {
    /// Unix seconds (UTC) — kept for ordering/debugging, same as `StatEntry`.
    pub t: u64,
    pub text: String,
    /// The ASR transcript `text` was polished from. Empty on flags from
    /// before #8, and on History rows reloaded after a restart (History
    /// keeps raw in memory only). `--replay-flags` skips those.
    #[serde(default)]
    pub raw: String,
    pub asr_engine: String,
    pub asr_language: String,
    pub polish_mode: String,
    pub dictionary_count: usize,
    pub vocabulary_count: usize,
    pub app_version: String,
}

pub fn path() -> PathBuf {
    config::data_dir().join("bug-reports.jsonl")
}

/// Append one flagged entry. Failures are logged, never fatal — this is a
/// diagnostic aid, not something that should ever interrupt dictation.
pub fn record(r: &BugReport) {
    if let Err(err) = record_in(&path(), r) {
        tracing::warn!(?err, "failed to record bug report");
    }
}

fn record_in(path: &Path, r: &BugReport) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(f, "{}", serde_json::to_string(r)?)?;
    Ok(())
}

/// One flag's output from a replay. The snapshot next to the flags file is a
/// JSON array of these: the baseline the next run diffs against.
#[derive(Serialize, Deserialize)]
struct Replayed {
    t: u64,
    raw: String,
    out: String,
}

/// `sotto.exe --replay-flags [path]`: run every flag's `raw` through
/// `polish` (main supplies the current chain), print raw / delivered / now,
/// and diff against the last run's snapshot. The new run always becomes the
/// snapshot, so a change fails once and a rerun accepts it. Returns how many
/// flags changed since the last run; a flag not in the snapshot is new, not
/// changed. Prints with `println!`, never `tracing`: this is dictated text,
/// and the log must not hold it (#48).
pub fn replay(path: &Path, mut polish: impl FnMut(&BugReport) -> String) -> anyhow::Result<usize> {
    let body = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let snapshot = path.with_extension("snapshot.json");
    let prev: Vec<Replayed> =
        std::fs::read_to_string(&snapshot).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let (mut changed, mut new, mut skipped) = (0, 0, 0);
    let mut cur = Vec::new();
    // Unparseable lines are skipped, same as history's loader.
    for f in body.lines().filter_map(|l| serde_json::from_str::<BugReport>(l).ok()) {
        if f.raw.is_empty() {
            skipped += 1;
            continue;
        }
        let out = polish(&f);
        println!("[{} {}]", f.t, f.polish_mode);
        println!("  raw        {:?}", f.raw);
        println!("  delivered  {:?}", f.text);
        println!("  now        {out:?}");
        match prev.iter().find(|p| p.t == f.t && p.raw == f.raw) {
            None => new += 1,
            Some(p) if p.out != out => {
                changed += 1;
                println!("  CHANGED    last run {:?}", p.out);
            }
            Some(_) => {}
        }
        cur.push(Replayed { t: f.t, raw: f.raw, out });
    }
    std::fs::write(&snapshot, serde_json::to_string_pretty(&cur)?)?;
    println!(
        "{} replayed: {changed} changed since the last run, {new} new, {skipped} skipped (no raw). Snapshot: {}",
        cur.len(),
        snapshot.display()
    );
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(text: &str) -> BugReport {
        BugReport {
            t: 0,
            text: text.to_string(),
            raw: String::new(),
            asr_engine: "parakeet-v3".into(),
            asr_language: "auto".into(),
            polish_mode: "ai".into(),
            dictionary_count: 3,
            vocabulary_count: 2,
            app_version: "0.5.4".into(),
        }
    }

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sotto-bugreport-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn record_writes_a_readable_jsonl_line() {
        let path = temp_path("one.jsonl");
        record_in(&path, &sample("clawed is broken again")).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = raw.lines().collect();
        assert_eq!(lines.len(), 1);
        let parsed: BugReport = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(parsed.text, "clawed is broken again");
        assert_eq!(parsed.dictionary_count, 3);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn multiple_flags_append_rather_than_overwrite() {
        let path = temp_path("two.jsonl");
        record_in(&path, &sample("first")).unwrap();
        record_in(&path, &sample("second")).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert_eq!(raw.lines().count(), 2);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_line_from_before_raw_existed_still_parses() {
        let old = r#"{"t":1,"text":"hi","asr_engine":"parakeet-v3","asr_language":"auto","polish_mode":"rules","dictionary_count":0,"vocabulary_count":0,"app_version":"0.5.4"}"#;
        let parsed: BugReport = serde_json::from_str(old).unwrap();
        assert_eq!(parsed.raw, "");
    }

    #[test]
    fn replay_diffs_against_the_last_run() {
        let path = temp_path("replay.jsonl");
        let snapshot = path.with_extension("snapshot.json");
        let _ = std::fs::remove_file(&snapshot);
        let flag = |t, raw: &str| BugReport { t, raw: raw.into(), ..sample("delivered") };
        let write = |flags: &[BugReport]| {
            let _ = std::fs::remove_file(&path);
            for f in flags {
                record_in(&path, f).unwrap();
            }
        };
        let upper = |f: &BugReport| f.raw.to_uppercase();
        let fixed = |f: &BugReport| if f.t == 1 { "The plan.".to_string() } else { f.raw.to_uppercase() };
        let corpus = vec![flag(1, "um the the plan"), flag(2, "send it to sam"), sample("flagged before raw was stored")];
        write(&corpus);

        // First run: everything is new, nothing to diff against.
        assert_eq!(replay(&path, upper).unwrap(), 0);
        let snap: Vec<Replayed> = serde_json::from_str(&std::fs::read_to_string(&snapshot).unwrap()).unwrap();
        assert_eq!(snap.len(), 2, "the flag without raw is skipped");
        assert_eq!(snap[0].out, "UM THE THE PLAN");

        // Same chain: no changes.
        assert_eq!(replay(&path, upper).unwrap(), 0);

        // The chain now handles one flag differently, and a new flag lands.
        let mut grown = corpus.clone();
        grown.push(flag(3, "brand new"));
        write(&grown);
        assert_eq!(replay(&path, fixed).unwrap(), 1, "only the flag whose output moved counts, not the new one");

        // That run became the snapshot, so a rerun accepts the change.
        assert_eq!(replay(&path, fixed).unwrap(), 0);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&snapshot);
    }
}
