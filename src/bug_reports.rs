//! Local "flag this transcription" log for the History page. One JSON line
//! per flag, appended to `data_dir()/bug-reports.jsonl` alongside
//! `stats.jsonl` — the transcript plus whatever config context was live at
//! flag time. Purely local, like everything else here: nothing is sent
//! anywhere. Kai reviews the file himself and files a real GitHub issue by
//! hand when one's worth reporting.

use crate::config;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone)]
pub struct BugReport {
    /// Unix seconds (UTC) — kept for ordering/debugging, same as `StatEntry`.
    pub t: u64,
    pub text: String,
    pub asr_engine: String,
    pub asr_language: String,
    pub polish_mode: String,
    pub dictionary_count: usize,
    pub vocabulary_count: usize,
    pub app_version: String,
}

fn path() -> PathBuf {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(text: &str) -> BugReport {
        BugReport {
            t: 0,
            text: text.to_string(),
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
}
