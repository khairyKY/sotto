// Sotto — local offline voice dictation for Windows. Tauri v2 shell around a
// UI-agnostic Rust core (asr, audio, inject, hotkey, llm, polish).
//
// DRAFT (Tauri migration): written before the MSVC toolchain was available, so
// it has not been compiled yet — expect a compile-fix pass. The core modules
// and the frontend (ui/) are done; this file wires them to Tauri windows,
// events, commands, and the tray. See docs/msvc-setup.md.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod asr;
mod assets;
mod audio;
mod config;
mod history;
mod hotkey;
mod inject;
mod job;
mod llm;
mod polish;
mod single_instance;
mod sounds;
mod startup;
mod stats;
mod tray;

use config::{ActivationMode, Config, DictEntry, InjectionMode, PolishMode};
use hotkey::DictationEvent;
use single_instance::SingleInstanceGuard;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};

/// Ignore captured clips shorter than this — almost always an accidental tap.
const MIN_CLIP_SAMPLES: usize = 16_000 / 2; // 0.5s at 16 kHz

/// App-window zoom bounds. Below 0.5 the sidebar labels stop being legible;
/// above 2.0 the 760px min-width layout starts clipping.
const ZOOM_MIN: f64 = 0.5;
const ZOOM_MAX: f64 = 2.0;

// ── shared runtime state ───────────────────────────────────────────────
#[derive(Clone)]
pub struct Controls {
    pub paused: Arc<AtomicBool>,
    pub polish_mode: Arc<AtomicU8>,
    pub activation: Arc<AtomicU8>,
    pub hotkey_idx: Arc<AtomicUsize>,
    pub ai_min_words: Arc<AtomicUsize>,
    pub dictionary: Arc<Mutex<Vec<(String, String)>>>,
    pub history: history::History,
    /// Live mic RMS (f32 bits), written by the audio callback.
    pub level: Arc<AtomicU32>,
    /// True while recording — gates the level-event emitter.
    pub listening: Arc<AtomicBool>,
    /// HWND (as isize) of the window that was focused when the user pressed
    /// the hotkey. If they Alt-Tab mid-dictation, we still inject here.
    /// 0 = nothing captured / capture failed.
    pub focus_target: Arc<AtomicIsize>,
    /// Set when Escape was pressed during an active dictation. The worker
    /// checks between stages and aborts if true, discarding the transcript.
    pub cancelled: Arc<AtomicBool>,
    /// Live "record usage stats" flag — flipped from settings without a
    /// restart, read by the worker at record time.
    pub stats_enabled: Arc<AtomicBool>,
    /// Selected input device name, or `None` for the OS default. Read by
    /// `Recorder::start` on every dictation, so a change applies immediately.
    pub microphone: Arc<Mutex<Option<String>>>,
    /// Current overlay pill state ("idle", "listening", …) — written by
    /// emit_state, read by the overlay hit-test poll to know the pill's
    /// width and whether it currently shows a clickable button.
    pub overlay_state: Arc<Mutex<String>>,
    /// Soft start/stop recording ticks — live-toggled from settings.
    pub sound_enabled: Arc<AtomicBool>,
    /// True while a retryable take is stashed — drives the tray menu's
    /// "Retry last dictation" enablement honestly.
    pub has_take: Arc<AtomicBool>,
}

impl Controls {
    fn from_config(cfg: &Config) -> Self {
        Self {
            paused: Arc::new(AtomicBool::new(false)),
            polish_mode: Arc::new(AtomicU8::new(cfg.polish.mode.as_u8())),
            activation: Arc::new(AtomicU8::new(cfg.activation_mode.as_u8())),
            hotkey_idx: Arc::new(AtomicUsize::new(hotkey::index_of(&cfg.hotkey))),
            ai_min_words: Arc::new(AtomicUsize::new(cfg.polish.ai_min_words)),
            dictionary: Arc::new(Mutex::new(
                cfg.dictionary.iter().map(|e| (e.spoken.clone(), e.replacement.clone())).collect(),
            )),
            history: history::History::new(),
            level: Arc::new(AtomicU32::new(0)),
            listening: Arc::new(AtomicBool::new(false)),
            focus_target: Arc::new(AtomicIsize::new(0)),
            cancelled: Arc::new(AtomicBool::new(false)),
            stats_enabled: Arc::new(AtomicBool::new(cfg.stats_enabled)),
            microphone: Arc::new(Mutex::new(cfg.microphone.clone())),
            overlay_state: Arc::new(Mutex::new("idle".to_string())),
            sound_enabled: Arc::new(AtomicBool::new(cfg.sound_enabled)),
            has_take: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// Tauri-managed state: live controls + the on-disk config (for persistence)
/// + the worker's event channel (so commands and the tray can drive it).
struct AppState {
    controls: Controls,
    cfg: Mutex<Config>,
    tx: crossbeam_channel::Sender<DictationEvent>,
}

// ── IPC payloads ───────────────────────────────────────────────────────
#[derive(serde::Serialize, serde::Deserialize)]
struct DictEntryDto {
    spoken: String,
    replacement: String,
}
#[derive(serde::Serialize, Clone)]
struct HistoryDto {
    time: String,
    text: String,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelDto {
    name: String,
    variant: String,
    meta: String,
    state: String,
    size: String,
    selected: bool,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct HotkeyOption {
    /// Human label shown in the picker (e.g. "Right Ctrl", "Mouse: Middle click").
    label: String,
    /// Stable config-file name — what `set_hotkey` accepts.
    name: String,
    /// UI hint: prompt the user "are you sure?" before saving this pick.
    risky: bool,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SettingsPayload {
    hotkey: String,
    activation: String,
    polish: String,
    threshold: usize,
    launch_login: bool,
    start_hidden: bool,
    dictionary: Vec<DictEntryDto>,
    history: Vec<HistoryDto>,
    models: Vec<ModelDto>,
    hotkey_options: Vec<HotkeyOption>,
    theme: String,
    /// Current input device name, or "" for the OS default.
    microphone: String,
    microphone_options: Vec<String>,
    paused: bool,
    sound_enabled: bool,
    has_take: bool,
    /// Where models/config live, for the settings "Open folder" link.
    data_dir: String,
    zoom: f64,
}

// ── commands ───────────────────────────────────────────────────────────
#[tauri::command]
fn get_settings(state: tauri::State<'_, AppState>) -> SettingsPayload {
    let cfg = state.cfg.lock().unwrap();
    let c = &state.controls;
    let idx = c.hotkey_idx.load(Ordering::Relaxed).min(hotkey::SUPPORTED_HOTKEYS.len() - 1);
    let hotkey_options: Vec<HotkeyOption> = hotkey::SUPPORTED_HOTKEYS
        .iter()
        .map(|(label, name, _, risky)| HotkeyOption {
            label: (*label).to_string(),
            name: (*name).to_string(),
            risky: *risky,
        })
        .collect();
    let activation = match ActivationMode::from_u8(c.activation.load(Ordering::Relaxed)) {
        ActivationMode::Toggle => "toggle",
        ActivationMode::Hold => "hold",
    };
    let polish = match PolishMode::from_u8(c.polish_mode.load(Ordering::Relaxed)) {
        PolishMode::Off => "off",
        PolishMode::Rules => "rules",
        PolishMode::Ai => "ai",
    };
    let installed = config::model_dir().exists();
    let size = dir_size_mb(config::model_dir()).map(|mb| format!("{mb} MB")).unwrap_or_default();
    SettingsPayload {
        hotkey_options,
        hotkey: hotkey::SUPPORTED_HOTKEYS[idx].1.to_string(),
        activation: activation.to_string(),
        polish: polish.to_string(),
        threshold: c.ai_min_words.load(Ordering::Relaxed),
        launch_login: startup::is_enabled(),
        start_hidden: cfg.start_hidden,
        dictionary: c.dictionary.lock().unwrap().iter().map(|(s, r)| DictEntryDto { spoken: s.clone(), replacement: r.clone() }).collect(),
        history: c.history.snapshot().into_iter().map(|e| HistoryDto { time: e.time, text: e.text }).collect(),
        models: vec![ModelDto {
            name: "Parakeet v3".into(),
            variant: "· English".into(),
            meta: "NVIDIA · int8 quantized".into(),
            state: if installed { "installed" } else { "download" }.into(),
            size,
            selected: installed,
        }],
        theme: cfg.theme.clone(),
        microphone: c.microphone.lock().unwrap().clone().unwrap_or_default(),
        microphone_options: audio::list_input_devices(),
        paused: c.paused.load(Ordering::Relaxed),
        sound_enabled: c.sound_enabled.load(Ordering::Relaxed),
        has_take: c.has_take.load(Ordering::Relaxed),
        data_dir: config::data_dir().display().to_string(),
        zoom: cfg.zoom,
    }
}

#[tauri::command]
fn set_sound_enabled(enabled: bool, state: tauri::State<'_, AppState>) {
    state.controls.sound_enabled.store(enabled, Ordering::Relaxed);
    let mut cfg = state.cfg.lock().unwrap();
    cfg.sound_enabled = enabled;
    let _ = cfg.save();
}

/// App-window zoom. Uses the webview's own zoom rather than CSS `zoom`/
/// transforms — the native factor scales the layout viewport too, so `100vh`
/// and the flex shell keep resolving correctly at any level.
#[tauri::command]
fn set_zoom(factor: f64, app: tauri::AppHandle, state: tauri::State<'_, AppState>) {
    let factor = factor.clamp(ZOOM_MIN, ZOOM_MAX);
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.set_zoom(factor);
    }
    let mut cfg = state.cfg.lock().unwrap();
    cfg.zoom = factor;
    let _ = cfg.save();
}

#[tauri::command]
fn set_microphone(name: String, state: tauri::State<'_, AppState>) {
    let picked = if name.is_empty() { None } else { Some(name) };
    *state.controls.microphone.lock().unwrap() = picked.clone();
    let mut cfg = state.cfg.lock().unwrap();
    cfg.microphone = picked;
    let _ = cfg.save();
}

#[tauri::command]
fn set_hotkey(key: String, state: tauri::State<'_, AppState>) {
    state.controls.hotkey_idx.store(hotkey::index_of(&key), Ordering::Relaxed);
    let mut cfg = state.cfg.lock().unwrap();
    cfg.hotkey = key;
    let _ = cfg.save();
}
#[tauri::command]
fn set_activation(mode: String, state: tauri::State<'_, AppState>) {
    let m = if mode == "toggle" { ActivationMode::Toggle } else { ActivationMode::Hold };
    state.controls.activation.store(m.as_u8(), Ordering::Relaxed);
    let mut cfg = state.cfg.lock().unwrap();
    cfg.activation_mode = m;
    let _ = cfg.save();
}
#[tauri::command]
fn set_polish(mode: String, state: tauri::State<'_, AppState>) {
    let m = match mode.as_str() {
        "off" => PolishMode::Off,
        "rules" => PolishMode::Rules,
        _ => PolishMode::Ai,
    };
    state.controls.polish_mode.store(m.as_u8(), Ordering::Relaxed);
    let mut cfg = state.cfg.lock().unwrap();
    cfg.polish.mode = m;
    let _ = cfg.save();
}
#[tauri::command]
fn set_threshold(words: usize, state: tauri::State<'_, AppState>) {
    state.controls.ai_min_words.store(words, Ordering::Relaxed);
    let mut cfg = state.cfg.lock().unwrap();
    cfg.polish.ai_min_words = words;
    let _ = cfg.save();
}
#[tauri::command]
fn set_dictionary(entries: Vec<DictEntryDto>, state: tauri::State<'_, AppState>) {
    let pairs: Vec<(String, String)> = entries
        .into_iter()
        .filter(|e| !e.spoken.trim().is_empty())
        .map(|e| (e.spoken, e.replacement))
        .collect();
    *state.controls.dictionary.lock().unwrap() = pairs.clone();
    let mut cfg = state.cfg.lock().unwrap();
    cfg.dictionary = pairs.into_iter().map(|(spoken, replacement)| DictEntry { spoken, replacement }).collect();
    let _ = cfg.save();
}
#[tauri::command]
fn set_launch_login(enabled: bool) {
    if let Err(err) = startup::set_enabled(enabled) {
        tracing::error!(?err, "failed to set launch-at-login");
    }
}
#[tauri::command]
fn set_start_hidden(enabled: bool, state: tauri::State<'_, AppState>) {
    let mut cfg = state.cfg.lock().unwrap();
    cfg.start_hidden = enabled;
    let _ = cfg.save();
}
#[tauri::command]
fn set_theme(theme: String, state: tauri::State<'_, AppState>) {
    let valid = theme == "light" || theme == "dark" || theme == "system";
    if !valid { return; }
    let mut cfg = state.cfg.lock().unwrap();
    cfg.theme = theme;
    let _ = cfg.save();
}

#[tauri::command]
fn copy_text(text: String) {
    if let Ok(mut cb) = arboard::Clipboard::new() {
        let _ = cb.set_text(text);
    }
}

/// Open a URL in the user's default browser. Used by the settings window's
/// "Download from GitHub" fallback link — always available so an updater
/// error is never a dead end. `cmd /c start "" <url>` handles URL escaping
/// and the empty "" arg is a start.exe quirk that prevents the URL being
/// treated as a window title.
#[tauri::command]
fn open_url(url: String) {
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new("cmd")
        .args(["/c", "start", "", &url])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .spawn();
}

/// Re-run the last dictation take (tray item + the overlay's ↺ button in the
/// 2.0 UI). No-op when nothing is stashed.
#[tauri::command]
fn retry_last(state: tauri::State<'_, AppState>) {
    let _ = state.tx.send(DictationEvent::Retry);
}

/// The overlay's ✕ button — same effect as pressing Escape, but reachable by
/// mouse for anyone who'd rather click than remember the shortcut.
#[tauri::command]
fn cancel_dictation(state: tauri::State<'_, AppState>) {
    let _ = state.tx.send(DictationEvent::Cancel);
}

/// A history row's ↻ — re-run the current polish tier + dictionary over that
/// row's text and leave the result on the clipboard.
#[tauri::command]
fn repolish_copy(text: String, state: tauri::State<'_, AppState>) {
    let _ = state.tx.send(DictationEvent::Repolish(text));
}

/// Aggregated usage stats for the Insights dashboard. Cheap enough to compute
/// on every call — no caching until it measurably matters.
#[tauri::command]
fn get_stats() -> stats::StatsPayload {
    let (today, _) = stats::local_today();
    stats::aggregate(&stats::load(), today)
}

#[tauri::command]
fn clear_stats() {
    stats::clear();
}

#[tauri::command]
fn set_stats_enabled(enabled: bool, state: tauri::State<'_, AppState>) {
    state.controls.stats_enabled.store(enabled, Ordering::Relaxed);
    let mut cfg = state.cfg.lock().unwrap();
    cfg.stats_enabled = enabled;
    let _ = cfg.save();
}

// ── main ───────────────────────────────────────────────────────────────
fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("SOTTO_LOG").unwrap_or_else(|_| "info".into()))
        .init();
    init_ort();

    if let Some(path) = arg_value("--transcribe") {
        return run_transcribe_once(&path);
    }
    if let Some(text) = arg_value("--polish") {
        return run_polish_once(&text);
    }

    let Some(_guard) = SingleInstanceGuard::acquire()? else {
        tracing::warn!("another Sotto instance is already running — exiting");
        return Ok(());
    };

    let cfg = Config::load_or_init()?;
    tracing::info!(?cfg, path = %Config::path().display(), "loaded config");
    let controls = Controls::from_config(&cfg);

    // The worker's event channel is created here so both the pipeline and the
    // Tauri commands / tray (via AppState.tx) can drive it — e.g. Retry.
    let (tx, rx) = crossbeam_channel::unbounded::<DictationEvent>();
    let state = AppState { controls: controls.clone(), cfg: Mutex::new(cfg.clone()), tx: tx.clone() };
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            get_settings, set_hotkey, set_activation, set_polish, set_threshold,
            set_dictionary, set_launch_login, set_start_hidden, set_theme, copy_text,
            open_url, check_update, install_update, retry_last, cancel_dictation, repolish_copy,
            get_stats, clear_stats, set_stats_enabled, set_microphone, set_sound_enabled, set_zoom,
            menu_action,
            assets::assets_status, assets::download_assets
        ])
        .setup(move |app| {
            build_tray(app)?;
            if let Some(w) = app.get_webview_window("overlay") {
                let _ = w.set_ignore_cursor_events(true);
                position_overlay(&w);
            }
            if let Some(w) = app.get_webview_window("settings") {
                if (cfg.zoom - 1.0).abs() > f64::EPSILON {
                    let _ = w.set_zoom(cfg.zoom.clamp(ZOOM_MIN, ZOOM_MAX));
                }
                if !cfg.start_hidden {
                    let _ = w.show();
                }
            }
            spawn_pipeline(app.handle().clone(), controls.clone(), cfg.clone(), tx.clone(), rx.clone());
            spawn_overlay_hittest(app.handle().clone(), controls.overlay_state.clone());
            spawn_update_check(app.handle().clone());
            assets::spawn_provision_if_missing(app.handle().clone());
            tracing::info!("Sotto ready — hold the hotkey and speak");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Sotto");
    Ok(())
}

/// Builds the tray icon + menu and wires its actions.
fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::tray::TrayIconBuilder;

    TrayIconBuilder::with_id("main")
        .icon(tray::idle_icon())
        .tooltip("Sotto")
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            match event {
                tauri::tray::TrayIconEvent::Click {
                    button: tauri::tray::MouseButton::Left,
                    button_state: tauri::tray::MouseButtonState::Up,
                    ..
                } => {
                    if let Some(w) = tray.app_handle().get_webview_window("settings") {
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                }
                tauri::tray::TrayIconEvent::Click {
                    button: tauri::tray::MouseButton::Right,
                    button_state: tauri::tray::MouseButtonState::Up,
                    position,
                    ..
                } => {
                    if let Some(w) = tray.app_handle().get_webview_window("menu") {
                        // Use the window's real size (already physical px) so
                        // this never drifts from tauri.conf.json / menu.html —
                        // hardcoded 230x260 here is what chipped the menu.
                        let (mw, mh) = w
                            .outer_size()
                            .map(|s| (s.width as f64, s.height as f64))
                            .unwrap_or((230.0, 380.0));
                        let x = (position.x - mw + 10.0) as i32;
                        let y = (position.y - mh - 5.0) as i32;
                        let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                }
                _ => {}
            }
        })
        .build(app)?;
    Ok(())
}

#[tauri::command]
fn menu_action(app: tauri::AppHandle, action: String) {
    if action == "quit" {
        app.exit(0);
    } else if action == "retry" {
        let _ = app.state::<AppState>().tx.send(DictationEvent::Retry);
    } else if action == "pause" {
        let c = &app.state::<AppState>().controls;
        let paused = !c.paused.load(Ordering::Relaxed);
        c.paused.store(paused, Ordering::Relaxed);
        tracing::info!(paused, "pause toggled from tray");
    } else {
        if let Some(w) = app.get_webview_window("settings") {
            let _ = w.show();
            let _ = w.set_focus();
            if action != "settings" {
                let _ = w.emit("navigate", action);
            }
        }
    }
}

#[derive(serde::Serialize)]
struct UpdateInfo {
    version: String,
    notes: Option<String>,
}

/// Ask GitHub whether a newer Sotto is published. Returns `None` when up to
/// date, on any network error, or if the check takes longer than 8 seconds
/// (a hung endpoint would otherwise leave the settings UI spinning forever).
#[tauri::command]
async fn check_update(app: tauri::AppHandle) -> Option<UpdateInfo> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater().ok()?;
    let checked = tokio::time::timeout(std::time::Duration::from_secs(8), updater.check()).await;
    match checked {
        Ok(Ok(Some(u))) => Some(UpdateInfo { version: u.version, notes: u.body }),
        Ok(Ok(None)) => None,
        Ok(Err(err)) => {
            tracing::warn!(?err, "check_update: updater plugin error");
            None
        }
        Err(_) => {
            tracing::warn!("check_update: 8s timeout — endpoint unreachable or slow");
            None
        }
    }
}

/// Download the pending update (the small ~15 MB installer — models aren't
/// bundled), verify its signature, install, and relaunch. Progress is emitted
/// as `update-progress` (downloaded, total).
#[tauri::command]
async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater().map_err(|e| e.to_string())?;
    let update = updater
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no update available".to_string())?;

    let app2 = app.clone();
    let mut downloaded: u64 = 0;
    update
        .download_and_install(
            move |chunk, total| {
                downloaded += chunk as u64;
                let _ = app2.emit("update-progress", (downloaded, total.unwrap_or(0)));
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;

    app.restart();
}

/// On launch, ask GitHub whether a newer Sotto is published. If so, emit
/// `update-available` so the settings window can show its banner. **No native
/// OS toast** — Windows toast dismiss timing is uncontrollable and users
/// found it lingering; the in-app banner + tray presence are enough.
/// No-op in dev builds (updater not configured).
fn spawn_update_check(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        use tauri_plugin_updater::UpdaterExt;
        let updater = match app.updater() {
            Ok(u) => u,
            Err(err) => {
                tracing::warn!(?err, "updater unavailable");
                return;
            }
        };
        match updater.check().await {
            Ok(Some(update)) => {
                tracing::info!(version = %update.version, "update available");
                let _ = app.emit("update-available", update.version.clone());
            }
            Ok(None) => tracing::info!("Sotto is up to date"),
            Err(err) => tracing::warn!(?err, "update check failed"),
        }
    });
}

/// Position the (always-on-top, transparent) overlay window bottom-center.
fn position_overlay(w: &tauri::WebviewWindow) {
    if let Ok(Some(mon)) = w.current_monitor() {
        let sz = mon.size();
        let scale = mon.scale_factor();
        let ww = 260.0 * scale;
        let wh = 120.0 * scale;
        let x = ((sz.width as f64 - ww) / 2.0) as i32;
        let y = (sz.height as f64 - wh - 8.0 * scale) as i32;
        let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
    }
}

/// Spawns the hotkey listener, the dictation worker, and the mic-level emitter.
/// These own `!Send` resources (recorder, ASR) so each lives on its own thread.
fn spawn_pipeline(
    app: tauri::AppHandle,
    controls: Controls,
    cfg: Config,
    tx: crossbeam_channel::Sender<DictationEvent>,
    rx: crossbeam_channel::Receiver<DictationEvent>,
) {
    let suppressed = Arc::new(AtomicBool::new(false));

    // Global hotkey listener.
    {
        let hotkey_idx = controls.hotkey_idx.clone();
        let activation = controls.activation.clone();
        let paused = controls.paused.clone();
        let supp = suppressed.clone();
        let cancelled = controls.cancelled.clone();
        std::thread::spawn(move || {
            hotkey::run_listener(hotkey_idx, activation, tx, supp, paused, cancelled)
        });
    }

    // Mic-level emitter (only while recording).
    {
        let app = app.clone();
        let level = controls.level.clone();
        let listening = controls.listening.clone();
        std::thread::spawn(move || loop {
            if listening.load(Ordering::Relaxed) {
                let lv = f32::from_bits(level.load(Ordering::Relaxed));
                let _ = app.emit("overlay-level", lv);
            }
            std::thread::sleep(Duration::from_millis(40));
        });
    }

    // Dictation worker.
    let injection_mode = cfg.injection_mode;
    let polisher = polish::Polisher::new(controls.clone(), cfg.llm.clone());
    let history = controls.history.clone();
    let listening = controls.listening.clone();
    let level = controls.level.clone();
    let focus_target = controls.focus_target.clone();
    let cancelled = controls.cancelled.clone();
    let stats_enabled = controls.stats_enabled.clone();
    let microphone = controls.microphone.clone();
    let sound_enabled = controls.sound_enabled.clone();
    let has_take = controls.has_take.clone();
    std::thread::spawn(move || {
        let mut recorder = audio::Recorder::new(level, microphone);
        let mut asr = asr::Asr::new();
        // Warm the ASR model on this thread before serving events, so the
        // first dictation doesn't eat the ~5s load. Any Start that arrives
        // meanwhile just queues on the channel.
        asr.preload();
        // The last dictation, kept in memory so Escape/error is recoverable
        // via Retry. Overwritten by the next Start; cleared on successful
        // delivery.
        let mut stash: Option<Take> = None;

        for event in rx {
            match event {
                DictationEvent::Start => {
                    cancelled.store(false, Ordering::SeqCst);
                    match recorder.start() {
                        Ok(()) => {
                            listening.store(true, Ordering::Relaxed);
                            if sound_enabled.load(Ordering::Relaxed) {
                                sounds::tick();
                            }
                            // Capture the focus target NOW so an Alt-Tab during
                            // dictation doesn't reroute the injection.
                            focus_target.store(inject::capture_focus(), Ordering::Relaxed);
                            // Warm the LLM sidecar while the user speaks.
                            polisher.prewarm();
                            show_overlay(&app);
                            emit_state(&app, "listening");
                            tracing::info!("listening");
                        }
                        Err(err) => {
                            emit_state(&app, "error");
                            tracing::error!(?err, "failed to start capture");
                        }
                    }
                }
                DictationEvent::Cancel => {
                    // Only meaningful while recording — Escape mid-transcribe/
                    // polish is handled by the stage-boundary flag checks in
                    // process_take (the flag was already set by the listener).
                    if listening.swap(false, Ordering::Relaxed) {
                        if sound_enabled.load(Ordering::Relaxed) {
                            sounds::tock();
                        }
                        let samples = recorder.stop().unwrap_or_default();
                        cancelled.store(false, Ordering::SeqCst); // consumed here
                        let take = Take::new(samples, focus_target.swap(0, Ordering::Relaxed), &polisher);
                        emit_state(&app, "cancelled");
                        tracing::info!("dictation cancelled while recording");
                        // Only worth stashing if there's enough audio to retry.
                        if take.samples.len() >= MIN_CLIP_SAMPLES {
                            record_outcome(&take, &stats_enabled, "cancelled");
                            stash = Some(take);
                        }
                    }
                }
                DictationEvent::Stop => {
                    // Only tock if we were actually recording (a stray Stop —
                    // e.g. toggle-mode release edge — shouldn't chirp).
                    if listening.swap(false, Ordering::Relaxed)
                        && sound_enabled.load(Ordering::Relaxed)
                    {
                        sounds::tock();
                    }
                    let samples = match recorder.stop() {
                        Ok(s) => s,
                        Err(err) => {
                            emit_state(&app, "error");
                            tracing::error!(?err, "failed to stop capture");
                            continue;
                        }
                    };
                    if samples.len() < MIN_CLIP_SAMPLES {
                        emit_state(&app, "idle");
                        continue;
                    }
                    let take = Take::new(samples, focus_target.swap(0, Ordering::Relaxed), &polisher);
                    // Cancel arrived during/right after recording.
                    if cancelled.swap(false, Ordering::SeqCst) {
                        emit_state(&app, "cancelled");
                        record_outcome(&take, &stats_enabled, "cancelled");
                        stash = Some(take);
                        continue;
                    }
                    process_take(
                        &app, &mut asr, &polisher, &history, &suppressed, &cancelled,
                        injection_mode, &stats_enabled, take, &mut stash,
                    );
                }
                DictationEvent::Retry => {
                    if let Some(take) = stash.take() {
                        cancelled.store(false, Ordering::SeqCst);
                        show_overlay(&app);
                        tracing::info!(has_text = take.raw_text.is_some(), "retrying last dictation");
                        process_take(
                            &app, &mut asr, &polisher, &history, &suppressed, &cancelled,
                            injection_mode, &stats_enabled, take, &mut stash,
                        );
                    } else {
                        tracing::info!("retry requested but nothing stashed");
                    }
                }
                DictationEvent::Repolish(text) => {
                    let out = polisher.polish(&text);
                    if !out.text.is_empty() {
                        if let Ok(mut cb) = arboard::Clipboard::new() {
                            let _ = cb.set_text(out.text.clone());
                        }
                        // Brief "done" pill as the only feedback — the result
                        // is on the clipboard, nothing is injected.
                        show_overlay(&app);
                        emit_state(&app, "done");
                        tracing::info!("re-polished and copied {} chars", out.text.len());
                    }
                }
            }
            // Keep the tray menu's "Retry last dictation" honest.
            has_take.store(stash.is_some(), Ordering::Relaxed);
        }
    });
}

/// One dictation attempt kept in memory for a possible Retry. Never written to
/// disk. `raw_text` is set once ASR has run, so a retry after a successful
/// transcription skips straight to polish + injection.
struct Take {
    samples: Vec<f32>,
    raw_text: Option<String>,
    focus_target: isize,
    audio_ms: u64,
    tier: String,
}

impl Take {
    fn new(samples: Vec<f32>, focus_target: isize, polisher: &polish::Polisher) -> Self {
        // Recorder returns 16 kHz mono, so ms = samples / 16.
        let audio_ms = samples.len() as u64 * 1000 / 16_000;
        Take { samples, raw_text: None, focus_target, audio_ms, tier: mode_str(polisher.mode()).into() }
    }
}

fn mode_str(m: PolishMode) -> &'static str {
    match m {
        PolishMode::Off => "off",
        PolishMode::Rules => "rules",
        PolishMode::Ai => "ai",
    }
}

/// Record a non-delivered outcome (cancelled/error) — words=0, no fixes.
fn record_outcome(take: &Take, stats_enabled: &Arc<AtomicBool>, outcome: &str) {
    if !stats_enabled.load(Ordering::Relaxed) {
        return;
    }
    let app_name = stats::app_name(take.focus_target);
    stats::record(&stats::entry_now(0, take.audio_ms, app_name, &take.tier, 0, 0, outcome));
}

/// Transcribe (unless already done) → polish → inject, honoring a mid-flight
/// cancel at each stage boundary. On any non-delivery the take is put back in
/// `stash` so Retry can resume; on delivery `stash` is cleared.
#[allow(clippy::too_many_arguments)]
fn process_take(
    app: &tauri::AppHandle,
    asr: &mut asr::Asr,
    polisher: &polish::Polisher,
    history: &history::History,
    suppressed: &Arc<AtomicBool>,
    cancelled: &Arc<AtomicBool>,
    injection_mode: InjectionMode,
    stats_enabled: &Arc<AtomicBool>,
    mut take: Take,
    stash: &mut Option<Take>,
) {
    // Stage 1 — transcription (skipped on a retry that already has text).
    if take.raw_text.is_none() {
        emit_state(app, "transcribing");
        match asr.transcribe(&take.samples) {
            Ok(t) => take.raw_text = Some(t),
            Err(err) => {
                tracing::error!(?err, "transcription failed");
                // Distinguish "the model isn't on disk yet" (first-run
                // download still in flight) from a real failure — the take is
                // stashed either way, so ↻ works once the download lands.
                let model_missing =
                    !config::model_dir().join("encoder-model.int8.onnx").exists();
                emit_state(app, if model_missing { "nomodel" } else { "error" });
                record_outcome(&take, stats_enabled, "error");
                *stash = Some(take);
                return;
            }
        }
    }
    if cancelled.swap(false, Ordering::SeqCst) {
        emit_state(app, "cancelled");
        record_outcome(&take, stats_enabled, "cancelled");
        *stash = Some(take);
        return;
    }
    let raw = take.raw_text.clone().unwrap_or_default();
    if raw.trim().is_empty() {
        emit_state(app, "error");
        record_outcome(&take, stats_enabled, "error");
        *stash = Some(take);
        return;
    }

    // Stage 2 — polish.
    if polisher.uses_ai_tier(&raw) {
        emit_state(app, "polishing");
    }
    let result = polisher.polish(&raw);
    if cancelled.swap(false, Ordering::SeqCst) {
        emit_state(app, "cancelled");
        record_outcome(&take, stats_enabled, "cancelled");
        *stash = Some(take);
        return;
    }
    if result.text.is_empty() {
        emit_state(app, "error");
        record_outcome(&take, stats_enabled, "error");
        *stash = Some(take);
        return;
    }

    // Stage 3 — inject into the original window.
    inject::restore_focus(take.focus_target);
    suppressed.store(true, Ordering::SeqCst);
    let injected = inject::inject_text(&result.text, injection_mode);
    suppressed.store(false, Ordering::SeqCst);
    match injected {
        Ok(()) => {
            // Leave the text on the clipboard as a mis-focus safety net.
            if let Ok(mut cb) = arboard::Clipboard::new() {
                let _ = cb.set_text(result.text.clone());
            }
            history.push(result.text.clone());
            emit_state(app, "done");
            emit_history(app, history);
            if stats_enabled.load(Ordering::Relaxed) {
                let words = result.text.split_whitespace().count();
                stats::record(&stats::entry_now(
                    words,
                    take.audio_ms,
                    stats::app_name(take.focus_target),
                    &take.tier,
                    result.corrected_words,
                    result.dict_hits,
                    "injected",
                ));
            }
            *stash = None; // delivered — nothing to retry
            tracing::info!("injected: {:?}", result.text);
        }
        Err(err) => {
            tracing::error!(?err, "injection failed");
            emit_state(app, "error");
            record_outcome(&take, stats_enabled, "error");
            *stash = Some(take);
        }
    }
}

fn emit_state(app: &tauri::AppHandle, s: &str) {
    let _ = app.emit("overlay-state", s);
    *app.state::<AppState>().controls.overlay_state.lock().unwrap() = s.to_string();
    if let Some(tray) = app.tray_by_id("main") {
        let theme = app.state::<AppState>().cfg.lock().unwrap().theme.clone();
        let dark = theme == "dark";
        let icon = if s == "listening" {
            if dark { tray::active_icon_dark() } else { tray::active_icon() }
        } else {
            if dark { tray::idle_icon_dark() } else { tray::idle_icon() }
        };
        let _ = tray.set_icon(Some(icon));
    }
    if s != "idle" {
        show_overlay(app);
    }
}

/// Makes the overlay's ✕/↻ buttons clickable without the invisible window
/// margins eating clicks: the window stays click-through except while the
/// cursor is actually inside the pill's rectangle. A 30 ms cursor poll is the
/// only way to do this — mouse events can't reach the webview while
/// click-through is on, so JS can't hit-test for us.
fn spawn_overlay_hittest(app: tauri::AppHandle, ui_state: Arc<Mutex<String>>) {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    std::thread::spawn(move || {
        let mut ignoring = true; // window starts click-through
        loop {
            // Pill width per state — must mirror pillWidthFor() in overlay.js.
            let pill_w = match ui_state.lock().unwrap().as_str() {
                "error" => Some(236.0),
                "cancelled" => Some(220.0),
                "nomodel" => Some(248.0),
                "listening" | "transcribing" | "polishing" => Some(148.0),
                _ => None, // idle/done: no buttons
            };
            let Some(pill_w) = pill_w else {
                if !ignoring {
                    if let Some(w) = app.get_webview_window("overlay") {
                        let _ = w.set_ignore_cursor_events(true);
                    }
                    ignoring = true;
                }
                std::thread::sleep(Duration::from_millis(150));
                continue;
            };
            if let Some(w) = app.get_webview_window("overlay") {
                let inside = (|| {
                    let pos = w.outer_position().ok()?;
                    let size = w.outer_size().ok()?;
                    let scale = w.scale_factor().ok()?;
                    let mut pt = POINT::default();
                    unsafe { GetCursorPos(&mut pt).ok()? };
                    let (pw, ph) = (pill_w * scale, 40.0 * scale);
                    let cx = pos.x as f64 + size.width as f64 / 2.0;
                    let cy = pos.y as f64 + size.height as f64 / 2.0;
                    let pad = 4.0 * scale;
                    Some(
                        (pt.x as f64) >= cx - pw / 2.0 - pad
                            && (pt.x as f64) <= cx + pw / 2.0 + pad
                            && (pt.y as f64) >= cy - ph / 2.0 - pad
                            && (pt.y as f64) <= cy + ph / 2.0 + pad,
                    )
                })()
                .unwrap_or(false);
                if inside == ignoring {
                    let _ = w.set_ignore_cursor_events(!inside);
                    ignoring = !inside;
                }
            }
            std::thread::sleep(Duration::from_millis(30));
        }
    });
}

fn show_overlay(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("overlay") {
        position_overlay(&w);
        let _ = w.show();
    }
}

fn emit_history(app: &tauri::AppHandle, history: &history::History) {
    let dto: Vec<HistoryDto> = history.snapshot().into_iter().map(|e| HistoryDto { time: e.time, text: e.text }).collect();
    let _ = app.emit("history-updated", dto);
}

/// Sum of file sizes in `dir`, in MB (whole number). None if unreadable.
fn dir_size_mb(dir: PathBuf) -> Option<u64> {
    let mut total = 0u64;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        if let Ok(meta) = entry.metadata() {
            if meta.is_file() {
                total += meta.len();
            }
        }
    }
    Some(total / 1_000_000)
}

/// Returns the value following `flag` in the process args, if present.
fn arg_value(flag: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1).cloned())
}

fn run_polish_once(raw: &str) -> anyhow::Result<()> {
    let cfg = Config::load_or_init().unwrap_or_default();
    let controls = Controls::from_config(&cfg);
    controls.polish_mode.store(PolishMode::Ai.as_u8(), Ordering::Relaxed);
    controls.ai_min_words.store(0, Ordering::Relaxed);
    let polisher = polish::Polisher::new(controls, cfg.llm.clone());
    let t = Instant::now();
    let out = polisher.polish(raw);
    println!("raw      => {raw:?}");
    println!(
        "polished => {:?}  ({} ms, {} words corrected, {} dict fixes)",
        out.text, t.elapsed().as_millis(), out.corrected_words, out.dict_hits
    );
    Ok(())
}

fn run_transcribe_once(path: &str) -> anyhow::Result<()> {
    let samples = transcribe_rs::audio::read_wav_samples(std::path::Path::new(path))
        .map_err(|e| anyhow::anyhow!("failed to read {path}: {e}"))?;
    let mut asr = asr::Asr::new();
    let t = Instant::now();
    let text = asr.transcribe(&samples)?;
    println!("({} samples, {} ms) => {text:?}", samples.len(), t.elapsed().as_millis());
    Ok(())
}

/// Point `ort` (load-dynamic) at our bundled ONNX Runtime dll.
fn init_ort() {
    let dll = config::onnxruntime_dll();
    if dll.exists() {
        unsafe { std::env::set_var("ORT_DYLIB_PATH", &dll) };
        tracing::info!(path = %dll.display(), "ORT_DYLIB_PATH set");
    } else {
        tracing::warn!(path = %dll.display(), "onnxruntime.dll not found — dictation will fail until installed");
    }
}
