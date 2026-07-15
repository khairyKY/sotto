//! Local usage stats for the Insights dashboard. One JSON line is appended to
//! `data_dir()/stats.jsonl` per dictation — **counts and timings only, never
//! transcript text** — and aggregation happens on demand when the dashboard
//! asks. A few thousand JSONL lines parse in milliseconds, so there is no
//! database and no background indexing. Nothing ever leaves the device.

use crate::config;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;

// ── the on-disk record ─────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
pub struct StatEntry {
    /// Unix seconds (UTC) — kept for ordering/debugging.
    pub t: u64,
    /// Local calendar day as days-since-1970 (civil), for streak math.
    pub day: i64,
    /// Local date "YYYY-MM-DD", for calendar labels without date math in JS.
    pub date: String,
    /// Words in the final injected text (0 for cancelled/empty takes).
    pub words: usize,
    /// Captured audio length.
    pub audio_ms: u64,
    /// Executable stem of the app dictated into ("VS Code", "chrome", …).
    pub app: String,
    /// Polish tier at the time ("off" | "rules" | "ai").
    pub tier: String,
    /// Words the cleanup changed (see polish::changed_words).
    pub corrected: usize,
    /// Dictionary replacements that fired.
    pub dict: usize,
    /// "injected" | "cancelled" | "error".
    pub outcome: String,
}

fn stats_path() -> PathBuf {
    config::data_dir().join("stats.jsonl")
}

/// Append one entry. Failures are logged, never fatal — losing a stats line
/// must not affect dictation.
pub fn record(e: &StatEntry) {
    let write = || -> anyhow::Result<()> {
        if let Some(parent) = stats_path().parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(stats_path())?;
        writeln!(f, "{}", serde_json::to_string(e)?)?;
        Ok(())
    };
    if let Err(err) = write() {
        tracing::warn!(?err, "failed to record stats entry");
    }
}

pub fn load() -> Vec<StatEntry> {
    let Ok(raw) = std::fs::read_to_string(stats_path()) else { return Vec::new() };
    raw.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

pub fn clear() {
    let _ = std::fs::remove_file(stats_path());
}

/// Convenience constructor stamping "now" (UTC secs + local day/date).
pub fn entry_now(
    words: usize,
    audio_ms: u64,
    app: String,
    tier: &str,
    corrected: usize,
    dict: usize,
    outcome: &str,
) -> StatEntry {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (day, date) = local_today();
    StatEntry {
        t,
        day,
        date,
        words,
        audio_ms,
        app,
        tier: tier.to_string(),
        corrected,
        dict,
        outcome: outcome.to_string(),
    }
}

// ── local calendar day ─────────────────────────────────────────────────

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as i64; // [0, 399]
    let mp = ((m + 9) % 12) as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// (local day number, "YYYY-MM-DD") for right now, via the OS local clock.
pub fn local_today() -> (i64, String) {
    let st = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    let (y, m, d) = (st.wYear as i64, st.wMonth as u32, st.wDay as u32);
    (days_from_civil(y, m, d), format!("{y:04}-{m:02}-{d:02}"))
}

// ── focused-app resolution ─────────────────────────────────────────────

/// Friendly names for common exe stems; everything else shows the stem as-is.
/// ponytail: a tiny static list, extend as users report gaps.
const PRETTY: &[(&str, &str)] = &[
    ("code", "VS Code"),
    ("cursor", "Cursor"),
    ("chrome", "Chrome"),
    ("msedge", "Edge"),
    ("firefox", "Firefox"),
    ("winword", "Word"),
    ("excel", "Excel"),
    ("powerpnt", "PowerPoint"),
    ("outlook", "Outlook"),
    ("olk", "Outlook"),
    ("notepad", "Notepad"),
    ("discord", "Discord"),
    ("slack", "Slack"),
    ("telegram", "Telegram"),
    ("whatsapp", "WhatsApp"),
    ("explorer", "File Explorer"),
    ("windowsterminal", "Terminal"),
    ("notion", "Notion"),
    ("obsidian", "Obsidian"),
];

/// Resolve the process behind `hwnd` to a display name. Empty string when the
/// window is gone or access is denied — aggregation groups those under "".
pub fn app_name(hwnd: isize) -> String {
    use windows::Win32::Foundation::{CloseHandle, HWND};
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

    if hwnd == 0 {
        return String::new();
    }
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(HWND(hwnd as *mut _), Some(&mut pid));
        if pid == 0 {
            return String::new();
        }
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return String::new();
        };
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(handle);
        if ok.is_err() {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        let stem = std::path::Path::new(&path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let key = stem.to_ascii_lowercase();
        PRETTY
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.to_string())
            .unwrap_or(stem)
    }
}

// ── aggregation for the dashboard ──────────────────────────────────────

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AppUsage {
    pub name: String,
    pub words: usize,
    pub pct: u32,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DayWords {
    pub date: String,
    pub day: i64,
    pub words: usize,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StatsPayload {
    pub total_words: usize,
    pub words_this_week: usize,
    pub words_this_month: usize,
    pub avg_wpm_30d: u32,
    pub best_wpm: u32,
    pub corrected_words_total: usize,
    pub dict_hits_total: usize,
    pub fixes_total: usize,
    pub time_saved_min: u32,
    pub top_apps: Vec<AppUsage>,
    /// Non-zero days within the last 371 (JS builds the calendar grid and
    /// fills the gaps).
    pub daily: Vec<DayWords>,
    pub current_streak: u32,
    pub longest_streak: u32,
}

/// Words-per-minute of one entry; None when too short to be meaningful.
/// Very short clips produce absurd rates, so require ≥10 words, and discard
/// anything past 400 wpm as a timing artifact.
fn wpm(e: &StatEntry) -> Option<u32> {
    if e.words < 10 || e.audio_ms == 0 {
        return None;
    }
    let v = (e.words as u64 * 60_000 / e.audio_ms) as u32;
    (v <= 400).then_some(v)
}

pub fn aggregate(entries: &[StatEntry], today: i64) -> StatsPayload {
    let injected: Vec<&StatEntry> = entries.iter().filter(|e| e.outcome == "injected").collect();

    let total_words: usize = injected.iter().map(|e| e.words).sum();
    let words_this_week: usize =
        injected.iter().filter(|e| e.day > today - 7).map(|e| e.words).sum();
    let words_this_month: usize =
        injected.iter().filter(|e| e.day > today - 30).map(|e| e.words).sum();

    let recent_wpms: Vec<u32> =
        injected.iter().filter(|e| e.day > today - 30).filter_map(|e| wpm(e)).collect();
    let avg_wpm_30d = if recent_wpms.is_empty() {
        0
    } else {
        recent_wpms.iter().sum::<u32>() / recent_wpms.len() as u32
    };
    let best_wpm = injected.iter().filter_map(|e| wpm(e)).max().unwrap_or(0);

    let corrected_words_total: usize = injected.iter().map(|e| e.corrected).sum();
    let dict_hits_total: usize = injected.iter().map(|e| e.dict).sum();

    // Minutes saved vs. typing the same words at 40 wpm, per dictation,
    // never negative.
    let time_saved_min: u32 = injected
        .iter()
        .map(|e| {
            let typing_min = e.words as f64 / 40.0;
            let spoken_min = e.audio_ms as f64 / 60_000.0;
            (typing_min - spoken_min).max(0.0)
        })
        .sum::<f64>()
        .round() as u32;

    // Top apps over the last 30 days.
    let mut by_app: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for e in injected.iter().filter(|e| e.day > today - 30 && !e.app.is_empty()) {
        *by_app.entry(e.app.as_str()).or_default() += e.words;
    }
    let app_total: usize = by_app.values().sum();
    let mut top_apps: Vec<AppUsage> = by_app
        .into_iter()
        .map(|(name, words)| AppUsage {
            name: name.to_string(),
            words,
            pct: if app_total > 0 { (words * 100 / app_total) as u32 } else { 0 },
        })
        .collect();
    top_apps.sort_by(|a, b| b.words.cmp(&a.words));
    top_apps.truncate(5);

    // Daily buckets (last 371 days → covers a 53-week calendar).
    let mut by_day: std::collections::BTreeMap<i64, (String, usize)> =
        std::collections::BTreeMap::new();
    for e in injected.iter().filter(|e| e.day > today - 371 && e.words > 0) {
        let slot = by_day.entry(e.day).or_insert_with(|| (e.date.clone(), 0));
        slot.1 += e.words;
    }
    let daily: Vec<DayWords> = by_day
        .iter()
        .map(|(&day, (date, words))| DayWords { date: date.clone(), day, words: *words })
        .collect();

    // Streaks over the set of active days. Current streak may start today or
    // yesterday (dictating daily shouldn't require checking after midnight).
    let active: std::collections::BTreeSet<i64> = by_day.keys().copied().collect();
    let mut current_streak = 0u32;
    let mut d = if active.contains(&today) { today } else { today - 1 };
    while active.contains(&d) {
        current_streak += 1;
        d -= 1;
    }
    let mut longest_streak = 0u32;
    let mut run = 0u32;
    let mut prev: Option<i64> = None;
    for &day in &active {
        run = match prev {
            Some(p) if day == p + 1 => run + 1,
            _ => 1,
        };
        longest_streak = longest_streak.max(run);
        prev = Some(day);
    }

    StatsPayload {
        total_words,
        words_this_week,
        words_this_month,
        avg_wpm_30d,
        best_wpm,
        corrected_words_total,
        dict_hits_total,
        fixes_total: corrected_words_total + dict_hits_total,
        time_saved_min,
        top_apps,
        daily,
        current_streak,
        longest_streak,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(day: i64, words: usize, audio_ms: u64, app: &str, outcome: &str) -> StatEntry {
        StatEntry {
            t: 0,
            day,
            date: format!("d{day}"),
            words,
            audio_ms,
            app: app.into(),
            tier: "ai".into(),
            corrected: 2,
            dict: 1,
            outcome: outcome.into(),
        }
    }

    #[test]
    fn days_from_civil_matches_known_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(1970, 1, 2), 1);
        assert_eq!(days_from_civil(2000, 3, 1), 11017);
        assert_eq!(days_from_civil(2026, 7, 15), 20649);
    }

    #[test]
    fn aggregate_totals_streaks_and_wpm() {
        let today = 20649;
        let entries = vec![
            e(today, 120, 60_000, "VS Code", "injected"),      // 120 wpm
            e(today - 1, 40, 60_000, "Chrome", "injected"),    // 40 wpm
            e(today - 2, 30, 30_000, "VS Code", "injected"),   // 60 wpm
            e(today - 10, 500, 300_000, "Word", "injected"),   // 100 wpm, breaks streak
            e(today, 0, 5_000, "VS Code", "cancelled"),        // ignored in totals
            e(today, 5, 1_000, "VS Code", "injected"),         // <10 words: no wpm
        ];
        let p = aggregate(&entries, today);
        assert_eq!(p.total_words, 120 + 40 + 30 + 500 + 5);
        assert_eq!(p.current_streak, 3); // today, -1, -2
        assert_eq!(p.longest_streak, 3);
        assert_eq!(p.best_wpm, 120);
        assert_eq!(p.avg_wpm_30d, (120 + 40 + 60 + 100) / 4);
        assert_eq!(p.fixes_total, p.corrected_words_total + p.dict_hits_total);
        assert_eq!(p.top_apps[0].name, "Word"); // 500 words
        // Time saved: e.g. 500 words typed = 12.5 min vs 5 min spoken = 7.5.
        assert!(p.time_saved_min >= 8); // 7.5 + 1 + 0 + 0.25 + … rounded
    }

    #[test]
    fn streak_tolerates_no_dictation_yet_today() {
        let today = 100;
        let entries = vec![e(99, 50, 30_000, "a", "injected"), e(98, 50, 30_000, "a", "injected")];
        let p = aggregate(&entries, today);
        assert_eq!(p.current_streak, 2); // counted from yesterday
    }
}
