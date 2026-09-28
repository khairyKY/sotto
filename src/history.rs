//! Recent-dictation history for the settings window's "click to re-copy"
//! list. The worker appends on every successful injection; the settings
//! window reads a snapshot and copies an entry back to the clipboard on click.
//!
//! In memory by default, so it resets on restart. With `persist_history` on
//! (N4, opt-in, off by default) the list is also mirrored to
//! `data_dir()/history.jsonl` and reloaded at startup. The file is rewritten
//! whole on every push (tmp + rename, so a crash mid-write can't truncate it):
//! it's capped at 500 entries / 1 MB, which makes that a trivial write.

use crate::config;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use windows::Win32::System::SystemInformation::GetLocalTime;

/// Most recent dictations kept in memory only — plenty for "click to re-copy
/// the last thing I said", not meant as a searchable log.
const CAP: usize = 20;
/// Entries kept once history persists. ~150 bytes per 25-word dictation.
const PERSIST_CAP: usize = 500;
/// Byte cap on the file, whichever of the two bites first. Only guards
/// against someone dictating essays; the count cap is what bites in practice.
const PERSIST_MAX_BYTES: usize = 1_000_000;

/// Short keys keep the file compact: one line per entry.
#[derive(Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    #[serde(rename = "t")]
    pub time: String,
    #[serde(rename = "x")]
    pub text: String,
    /// Local date "YYYY-MM-DD", so an entry reloaded on a later day can show
    /// its date instead of a bare clock time.
    #[serde(rename = "d", default)]
    pub date: String,
}

/// Shared handle; cheap to clone. Newest entries first.
#[derive(Clone)]
pub struct History {
    entries: Arc<Mutex<VecDeque<HistoryEntry>>>,
    persist: Arc<AtomicBool>,
    path: PathBuf,
}

impl History {
    pub fn new(persist: bool) -> Self {
        Self::at(config::data_dir().join("history.jsonl"), persist)
    }

    /// `path` is only read when `persist` is on.
    fn at(path: PathBuf, persist: bool) -> Self {
        let entries = if persist { read(&path) } else { VecDeque::with_capacity(CAP) };
        Self { entries: Arc::new(Mutex::new(entries)), persist: Arc::new(AtomicBool::new(persist)), path }
    }

    pub fn push(&self, text: String) {
        let (time, date) = now_labels();
        let persist = self.persist.load(Ordering::Relaxed);
        let mut g = self.entries.lock().unwrap();
        g.push_front(HistoryEntry { time, text, date });
        g.truncate(if persist { PERSIST_CAP } else { CAP });
        if persist {
            save(&self.path, &g);
        }
    }

    /// Snapshot of entries, newest first. Entries from an earlier day are
    /// labelled with their date ("Sep 27") rather than a clock time.
    pub fn snapshot(&self) -> Vec<HistoryEntry> {
        let today = now_labels().1;
        let g = self.entries.lock().unwrap();
        g.iter()
            .map(|e| {
                let mut e = e.clone();
                if !e.date.is_empty() && e.date != today {
                    e.time = short_date(&e.date);
                }
                e
            })
            .collect()
    }

    /// Turning it on writes what's in memory now; off deletes the file —
    /// off means gone, not merely "stopped appending". The in-memory session
    /// list is left alone either way.
    pub fn set_persist(&self, on: bool) {
        self.persist.store(on, Ordering::Relaxed);
        let g = self.entries.lock().unwrap();
        if on {
            save(&self.path, &g);
        } else {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    /// Settings' "Clear history": the list and the file.
    pub fn clear(&self) {
        self.entries.lock().unwrap().clear();
        let _ = std::fs::remove_file(&self.path);
    }
}

/// File is oldest-first (append order); returns newest-first. Unparseable
/// lines are skipped rather than failing the whole load.
fn read(path: &Path) -> VecDeque<HistoryEntry> {
    let Ok(raw) = std::fs::read_to_string(path) else { return VecDeque::new() };
    raw.lines().rev().filter_map(|l| serde_json::from_str(l).ok()).take(PERSIST_CAP).collect()
}

/// Write `entries` (newest-first) to `path`, dropping the oldest past the
/// byte cap. Failures are logged, never fatal — same discipline as `stats`.
fn save(path: &Path, entries: &VecDeque<HistoryEntry>) {
    let mut bytes = 0;
    let mut lines: Vec<String> = entries
        .iter()
        .filter_map(|e| serde_json::to_string(e).ok())
        .take_while(|l| {
            bytes += l.len() + 1;
            bytes <= PERSIST_MAX_BYTES
        })
        .collect();
    lines.reverse();
    let body: String = lines.iter().map(|l| format!("{l}\n")).collect();
    let tmp = path.with_extension("jsonl.tmp");
    let write = || -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&tmp, body)?;
        std::fs::rename(&tmp, path)
    };
    if let Err(err) = write() {
        tracing::warn!(?err, "failed to save history");
    }
}

/// Current local time as ("2:14 PM", "2026-09-28"): 12-hour, no leading zero
/// — matches the design mock's history rows.
fn now_labels() -> (String, String) {
    // SAFETY: GetLocalTime just fills a caller-owned SYSTEMTIME; no preconditions.
    let t = unsafe { GetLocalTime() };
    let (h12, suffix) = match t.wHour {
        0 => (12, "AM"),
        1..=11 => (t.wHour, "AM"),
        12 => (12, "PM"),
        h => (h - 12, "PM"),
    };
    (format!("{h12}:{:02} {suffix}", t.wMinute), format!("{:04}-{:02}-{:02}", t.wYear, t.wMonth, t.wDay))
}

/// "2026-09-27" -> "Sep 27". Anything malformed comes back as-is.
fn short_date(date: &str) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let mut parts = date.split('-').skip(1).map(|p| p.parse::<usize>().ok());
    match (parts.next().flatten(), parts.next().flatten()) {
        (Some(m @ 1..=12), Some(d)) => format!("{} {d}", MONTHS[m - 1]),
        _ => date.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A private temp file per test — never `data_dir()`, which is Kai's real
    /// history once this ships.
    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sotto-history-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("history.jsonl")
    }

    #[test]
    fn newest_first_and_capped() {
        let h = History::at(temp_path("mem"), false);
        for i in 0..CAP + 5 {
            h.push(format!("entry {i}"));
        }
        let snap = h.snapshot();
        assert_eq!(snap.len(), CAP, "should truncate to the cap");
        assert_eq!(snap[0].text, format!("entry {}", CAP + 4), "newest goes first");
        assert!(!h.path.exists(), "persist off must never write the file");
    }

    #[test]
    fn persisted_history_survives_a_reload() {
        let path = temp_path("roundtrip");
        let h = History::at(path.clone(), true);
        h.push("first".into());
        h.push("ثاني second — code-switched".into());
        let reloaded = History::at(path.clone(), true).snapshot();
        let texts: Vec<_> = reloaded.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["ثاني second — code-switched", "first"], "newest first, text intact");
        // Persist off at startup reads nothing, even with a file present.
        assert!(History::at(path.clone(), false).snapshot().is_empty());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn persisted_history_evicts_oldest_past_the_count_cap() {
        let path = temp_path("cap");
        let h = History::at(path.clone(), true);
        for i in 0..PERSIST_CAP + 5 {
            h.push(format!("entry {i}"));
        }
        let reloaded = History::at(path.clone(), true).snapshot();
        assert_eq!(reloaded.len(), PERSIST_CAP);
        assert_eq!(reloaded[0].text, format!("entry {}", PERSIST_CAP + 4));
        assert_eq!(reloaded.last().unwrap().text, "entry 5", "the five oldest are gone");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn persisted_history_evicts_oldest_past_the_byte_cap() {
        let path = temp_path("bytes");
        let h = History::at(path.clone(), true);
        let essay = "word ".repeat(40_000); // ~200 KB a line
        for i in 0..8 {
            h.push(format!("{i} {essay}"));
        }
        assert!(std::fs::metadata(&path).unwrap().len() as usize <= PERSIST_MAX_BYTES);
        let reloaded = History::at(path.clone(), true).snapshot();
        assert!(reloaded.len() < 8 && !reloaded.is_empty());
        assert!(reloaded[0].text.starts_with("7 "), "the newest entry is always kept");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn turning_persist_off_deletes_the_file_and_on_saves_the_session() {
        let path = temp_path("toggle");
        let h = History::at(path.clone(), false);
        h.push("said before opting in".into());
        h.set_persist(true);
        assert_eq!(History::at(path.clone(), true).snapshot()[0].text, "said before opting in");
        h.set_persist(false);
        assert!(!path.exists(), "off must mean gone");
        assert_eq!(h.snapshot().len(), 1, "the session list stays");
        h.set_persist(true);
        h.clear();
        assert!(h.snapshot().is_empty() && !path.exists());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn earlier_days_show_their_date() {
        assert_eq!(short_date("2026-09-27"), "Sep 27");
        assert_eq!(short_date("2026-01-05"), "Jan 5");
        assert_eq!(short_date("garbage"), "garbage");
        let path = temp_path("dates");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{\"t\":\"2:14 PM\",\"x\":\"old\",\"d\":\"2020-03-04\"}\n{\"t\":\"9:00 AM\",\"x\":\"undated\"}\n").unwrap();
        let snap = History::at(path.clone(), true).snapshot();
        assert_eq!(snap[1].time, "Mar 4");
        assert_eq!(snap[0].time, "9:00 AM", "a line without a date keeps its clock time");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn clock_label_is_12_hour_with_am_pm() {
        let label = now_labels().0;
        assert!(label.ends_with("AM") || label.ends_with("PM"));
        let hour: u32 = label.split(':').next().unwrap().parse().unwrap();
        assert!((1..=12).contains(&hour));
    }
}
