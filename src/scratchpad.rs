//! Scratchpad (#19): a private pad, a page in the main window. A take spoken
//! into it lands there as a row instead of being typed into an app. Its chord
//! (default Ctrl+Alt+Space) brings the pad up over whatever you're in, and
//! puts you back there.
//!
//! Rows live in `data_dir()/scratchpad.jsonl`, capped at `CAP`. They are the
//! user's words: never logged (#48), never sent anywhere. Each can be copied,
//! typed into the app that was focused before the chord opened the pad,
//! parked to a markdown file (`scratchpad_park_file`), or deleted.

use crate::config::{self, InjectionMode};
use crate::hotkey::{parse_chord, Chord, DictationEvent};
use crate::inject;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// Rows kept; the oldest go first. ~150 bytes for a 25-word thought.
const CAP: usize = 200;

/// The pad's chord, `None` while the pad is off or its chord won't parse.
static CHORD: Mutex<Option<Chord>> = Mutex::new(None);
/// The pad is the page in front in the main window, as the page reports it.
/// Hiding the window leaves it set: the page is still the pad when it shows.
static OPEN: AtomicBool = AtomicBool::new(false);
/// The window that was focused when the chord opened the pad: where a row's
/// inject types.
static TARGET: AtomicIsize = AtomicIsize::new(0);
/// Takes land on the worker thread and deletes come from the page, so the
/// file's read-modify-write is serialized.
static FILE: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// The chord: bring the pad up, or put it away if it's in front.
    Toggle,
    /// A row's inject: type this into `TARGET`.
    Inject(String),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Row {
    /// Unix ms when it landed, which also makes it the row's id.
    pub id: u64,
    pub text: String,
}

pub fn init(cfg: &config::Config) {
    *CHORD.lock().unwrap() = cfg.scratchpad.then(|| parse_chord(&cfg.scratchpad_chord)).flatten();
}

/// Read by the hotkey listener on every key event.
pub fn chord() -> Option<Chord> {
    *CHORD.lock().unwrap()
}

fn main_window(app: &AppHandle) -> Option<(tauri::WebviewWindow, isize)> {
    let w = app.get_webview_window("settings")?;
    let hwnd = w.hwnd().map_or(0, |h| h.0 as isize);
    Some((w, hwnd))
}

/// `process_take`'s routing: does a take spoken into window `target` go to
/// the pad instead of being typed?
pub fn catches(app: &AppHandle, target: isize) -> bool {
    lands_in_pad(OPEN.load(Ordering::Relaxed), target, main_window(app).map_or(0, |(_, h)| h))
}

/// Only a take spoken into the pad itself: the pad page open, and the main
/// window the one it was spoken into. A take spoken into any other app is
/// typed there, pad open or not. One still in flight when the chord hid the
/// pad still lands in it: its window is hidden, and typing there would lose it.
fn lands_in_pad(open: bool, target: isize, pad: isize) -> bool {
    open && target != 0 && target == pad
}

/// A take spoken into the pad becomes its newest row. False if it couldn't
/// be saved, and the caller keeps the take for a retry.
pub fn land(app: &AppHandle, text: &str) -> bool {
    match push(&path(), text, now_ms()) {
        Ok(rows) => {
            let _ = app.emit("scratchpad-updated", rows);
            true
        }
        Err(err) => {
            tracing::warn!(?err, "couldn't save a scratchpad row");
            false
        }
    }
}

/// The chord and a row's inject. They run on the worker thread, queued behind
/// any take being delivered, with the hotkey listener muted while we press
/// keys (`suppressed`, see `hotkey::run_listener`).
pub fn run(app: &AppHandle, action: Action, suppressed: &Arc<AtomicBool>, mode: InjectionMode) {
    let Some((w, pad)) = main_window(app) else { return };
    match action {
        Action::Toggle if OPEN.load(Ordering::Relaxed) && inject::capture_focus() == pad => {
            let _ = w.hide();
            inject::restore_focus(TARGET.load(Ordering::Relaxed));
        }
        Action::Toggle => {
            if !inject::foreground_is_ours() {
                TARGET.store(inject::capture_focus(), Ordering::Relaxed);
            }
            OPEN.store(true, Ordering::Relaxed);
            let _ = w.unminimize();
            let _ = w.show();
            // From the background, set_focus taps a fake Alt to be allowed
            // the foreground, which a hotkey bound to Alt would take as a
            // press. `is_focused` is a round trip to the UI thread, so
            // set_focus has run once it returns.
            suppressed.store(true, Ordering::SeqCst);
            let _ = w.set_focus();
            let _ = w.is_focused();
            std::thread::sleep(Duration::from_millis(30));
            suppressed.store(false, Ordering::SeqCst);
            let _ = w.emit("navigate", "scratchpad");
        }
        Action::Inject(text) => {
            let target = TARGET.load(Ordering::Relaxed);
            // The window is gone (or the pad was never opened by its chord):
            // never type into whatever happens to be focused instead.
            if crate::stats::app_name(target).is_empty() {
                crate::emit_state(app, "kept");
                return;
            }
            inject::restore_focus(target);
            let _ = w.hide();
            suppressed.store(true, Ordering::SeqCst);
            let typed = inject::inject_text(&text, mode).is_ok();
            suppressed.store(false, Ordering::SeqCst);
            crate::emit_state(app, if typed { "done" } else { "kept" });
        }
    }
}

// ── the store ──────────────────────────────────────────────────────────

fn path() -> PathBuf {
    config::data_dir().join("scratchpad.jsonl")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// Oldest first, as on disk. An unparseable line is skipped, not fatal.
fn load(path: &Path) -> Vec<Row> {
    let Ok(raw) = std::fs::read_to_string(path) else { return Vec::new() };
    raw.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// tmp + rename, so a crash mid-write can't truncate it (as `history.rs`).
fn save(path: &Path, rows: &[Row]) -> std::io::Result<()> {
    let body: String = rows.iter().filter_map(|r| serde_json::to_string(r).ok()).map(|l| l + "\n").collect();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("jsonl.tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)
}

/// Add `text` as the newest row and drop the oldest past `CAP`. Ids only go
/// up, even if the clock steps back.
fn push(path: &Path, text: &str, now: u64) -> std::io::Result<Vec<Row>> {
    let _file = FILE.lock().unwrap();
    let mut rows = load(path);
    let id = rows.last().map_or(now, |r| now.max(r.id + 1));
    rows.push(Row { id, text: text.to_string() });
    rows.drain(..rows.len().saturating_sub(CAP));
    save(path, &rows)?;
    Ok(rows)
}

fn delete(path: &Path, id: u64) -> std::io::Result<Vec<Row>> {
    let _file = FILE.lock().unwrap();
    let mut rows = load(path);
    rows.retain(|r| r.id != id);
    save(path, &rows)?;
    Ok(rows)
}

/// Append `text` to the markdown file as one list item, "- 2026-09-29 14:05
/// text". A multi-line row's later lines are indented under it, and a file
/// that doesn't end in a newline gets one first.
fn park(file: &Path, stamp: &str, text: &str) -> std::io::Result<()> {
    let open_line = std::fs::read(file).is_ok_and(|b| b.last().is_some_and(|&c| c != b'\n'));
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(file)?;
    let body = text.trim().replace("\r\n", "\n").replace('\n', "\n  ");
    writeln!(f, "{}- {stamp} {body}", if open_line { "\n" } else { "" })
}

/// Local "2026-09-29 14:05".
fn stamp_now() -> String {
    // SAFETY: GetLocalTime just fills a caller-owned SYSTEMTIME.
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!("{:04}-{:02}-{:02} {:02}:{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute)
}

// ── commands ───────────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PadState {
    enabled: bool,
    /// As configured, "Ctrl+Alt+Space", for the page's hint.
    chord: String,
    /// A park file is set, so rows get a Park button.
    can_park: bool,
    /// The app a row's inject types into; "" hides the button.
    target_app: String,
    /// Oldest first.
    rows: Vec<Row>,
}

#[tauri::command]
pub fn scratchpad_state(state: tauri::State<'_, crate::AppState>) -> PadState {
    let (enabled, chord, can_park) = {
        let cfg = state.cfg.lock().unwrap();
        (cfg.scratchpad, cfg.scratchpad_chord.clone(), !cfg.scratchpad_park_file.trim().is_empty())
    };
    PadState {
        enabled,
        chord,
        can_park,
        target_app: crate::stats::app_name(TARGET.load(Ordering::Relaxed)),
        rows: if enabled { load(&path()) } else { Vec::new() },
    }
}

/// The page reports whether the pad is the page in front.
#[tauri::command]
pub fn scratchpad_page(open: bool, state: tauri::State<'_, crate::AppState>) {
    OPEN.store(open && state.cfg.lock().unwrap().scratchpad, Ordering::Relaxed);
}

#[tauri::command]
pub fn scratchpad_delete(id: u64) -> Result<Vec<Row>, String> {
    delete(&path(), id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn scratchpad_park(text: String, state: tauri::State<'_, crate::AppState>) -> Result<(), String> {
    let file = state.cfg.lock().unwrap().scratchpad_park_file.trim().to_string();
    if file.is_empty() {
        return Err("No park file is set".into());
    }
    park(Path::new(&file), &stamp_now(), &text).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn scratchpad_inject(text: String, state: tauri::State<'_, crate::AppState>) {
    let _ = state.tx.send(DictationEvent::Scratchpad(Action::Inject(text)));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A private temp dir per test, never `data_dir()`.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sotto-scratchpad-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn only_a_take_spoken_into_the_open_pad_lands_in_it() {
        let (pad, notepad) = (0x1001, 0x2002);
        assert!(lands_in_pad(true, pad, pad));
        // Pad open, but the take was spoken into another app: typed there.
        assert!(!lands_in_pad(true, notepad, pad));
        // Pad closed (another page in front): the main window gets it typed, as before.
        assert!(!lands_in_pad(false, pad, pad));
        // No capture, no window: never the pad.
        assert!(!lands_in_pad(true, 0, 0));
    }

    #[test]
    fn rows_round_trip_newest_last_and_capped() {
        let path = temp_dir("cap").join("scratchpad.jsonl");
        push(&path, "افتح الـ terminal وشغّل الـ build", 1_000).unwrap();
        push(&path, "call the dentist\nthen the bank", 1_000).unwrap(); // same ms: id still goes up
        let rows = load(&path);
        assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), [1_000, 1_001]);
        assert_eq!(rows[0].text, "افتح الـ terminal وشغّل الـ build");
        assert_eq!(rows[1].text, "call the dentist\nthen the bank");

        for i in 0..CAP {
            push(&path, &format!("thought {i}"), 2_000 + i as u64).unwrap();
        }
        let rows = load(&path);
        assert_eq!(rows.len(), CAP);
        assert_eq!(rows[0].text, "thought 0");
        assert_eq!(rows[CAP - 1].text, format!("thought {}", CAP - 1));

        let left = delete(&path, rows[0].id).unwrap();
        assert_eq!(left.len(), CAP - 1);
        assert_eq!(load(&path), left);
    }

    #[test]
    fn park_appends_a_stamped_list_item() {
        let file = temp_dir("park").join("inbox.md");
        std::fs::write(&file, "# Inbox").unwrap(); // no trailing newline
        park(&file, "2026-09-29 14:05", "ship the overlay states first").unwrap();
        park(&file, "2026-09-29 14:07", " two lines\nof thought \n").unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "# Inbox\n- 2026-09-29 14:05 ship the overlay states first\n- 2026-09-29 14:07 two lines\n  of thought\n"
        );
        // A missing file is created.
        let fresh = file.with_file_name("new.md");
        park(&fresh, "2026-09-29 14:09", "كشري").unwrap();
        assert_eq!(std::fs::read_to_string(&fresh).unwrap(), "- 2026-09-29 14:09 كشري\n");
    }

    #[test]
    fn the_default_chord_parses_and_collides_with_no_default_shortcut() {
        use crate::hotkey::{index_of, Input, SUPPORTED_HOTKEYS};
        let pad = parse_chord(&config::default_scratchpad_chord()).expect("parses");
        for t in config::default_transforms() {
            assert_ne!(parse_chord(&t.chord), Some(pad), "{}", t.chord);
        }
        let dictation = SUPPORTED_HOTKEYS[index_of(&config::Config::default().hotkey)].2;
        assert_ne!(dictation, Input::Key(pad.key));
    }
}
