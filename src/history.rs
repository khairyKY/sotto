//! In-memory recent-dictation history for the settings window's "click to
//! re-copy" list. The worker appends on every successful injection; the
//! settings window reads a snapshot and copies an entry back to the clipboard
//! on click.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use windows::Win32::System::SystemInformation::GetLocalTime;

/// Most recent dictations kept — plenty for "click to re-copy the last thing
/// I said", not meant as a searchable log.
const CAP: usize = 20;

#[derive(Clone)]
pub struct HistoryEntry {
    pub time: String,
    pub text: String,
}

/// Shared handle; cheap to clone. Newest entries first.
// ponytail: in-memory only, resets on restart — add a small JSON log under
// data_dir() if surviving restarts turns out to matter.
#[derive(Clone)]
pub struct History(Arc<Mutex<VecDeque<HistoryEntry>>>);

impl History {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(VecDeque::with_capacity(CAP))))
    }

    pub fn push(&self, text: String) {
        let mut g = self.0.lock().unwrap();
        g.push_front(HistoryEntry { time: clock_label(), text });
        g.truncate(CAP);
    }

    /// Snapshot of entries, newest first.
    pub fn snapshot(&self) -> Vec<HistoryEntry> {
        self.0.lock().unwrap().iter().cloned().collect()
    }
}

/// Current local time as "2:14 PM" (12-hour, no leading zero) — matches the
/// design mock's history rows.
fn clock_label() -> String {
    // SAFETY: GetLocalTime just fills a caller-owned SYSTEMTIME; no preconditions.
    let t = unsafe { GetLocalTime() };
    let (h12, suffix) = match t.wHour {
        0 => (12, "AM"),
        1..=11 => (t.wHour, "AM"),
        12 => (12, "PM"),
        h => (h - 12, "PM"),
    };
    format!("{h12}:{:02} {suffix}", t.wMinute)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_and_capped() {
        let h = History::new();
        for i in 0..CAP + 5 {
            h.push(format!("entry {i}"));
        }
        let snap = h.snapshot();
        assert_eq!(snap.len(), CAP, "should truncate to the cap");
        assert_eq!(snap[0].text, format!("entry {}", CAP + 4), "newest goes first");
    }

    #[test]
    fn clock_label_is_12_hour_with_am_pm() {
        let label = clock_label();
        assert!(label.ends_with("AM") || label.ends_with("PM"));
        let hour: u32 = label.split(':').next().unwrap().parse().unwrap();
        assert!((1..=12).contains(&hour));
    }
}
