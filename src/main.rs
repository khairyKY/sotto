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
mod startup;
mod tray;

use config::{ActivationMode, Config, DictEntry, PolishMode};
use hotkey::DictationEvent;
use single_instance::SingleInstanceGuard;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};

/// Ignore captured clips shorter than this — almost always an accidental tap.
const MIN_CLIP_SAMPLES: usize = 16_000 / 2; // 0.5s at 16 kHz

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
        }
    }
}

/// Tauri-managed state: live controls + the on-disk config (for persistence).
struct AppState {
    controls: Controls,
    cfg: Mutex<Config>,
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
    }
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

    let state = AppState { controls: controls.clone(), cfg: Mutex::new(cfg.clone()) };
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            get_settings, set_hotkey, set_activation, set_polish, set_threshold,
            set_dictionary, set_launch_login, set_start_hidden, copy_text,
            open_url, check_update, install_update,
            assets::assets_status, assets::download_assets
        ])
        .setup(move |app| {
            build_tray(app)?;
            if let Some(w) = app.get_webview_window("overlay") {
                let _ = w.set_ignore_cursor_events(true);
                position_overlay(&w);
            }
            if !cfg.start_hidden {
                if let Some(w) = app.get_webview_window("settings") {
                    let _ = w.show();
                }
            }
            spawn_pipeline(app.handle().clone(), controls.clone(), cfg.clone());
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
    use tauri::menu::{CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder, SubmenuBuilder};
    use tauri::tray::TrayIconBuilder;

    let cur = PolishMode::from_u8(app.state::<AppState>().controls.polish_mode.load(Ordering::Relaxed));
    let pause = CheckMenuItemBuilder::with_id("pause", "Pause dictation").checked(false).build(app)?;
    let p_off = CheckMenuItemBuilder::with_id("polish_off", "Off").checked(cur == PolishMode::Off).build(app)?;
    let p_rules = CheckMenuItemBuilder::with_id("polish_rules", "Rules only").checked(cur == PolishMode::Rules).build(app)?;
    let p_ai = CheckMenuItemBuilder::with_id("polish_ai", "AI").checked(cur == PolishMode::Ai).build(app)?;
    let polish = SubmenuBuilder::new(app, "Polish").item(&p_off).item(&p_rules).item(&p_ai).build()?;
    let settings = MenuItemBuilder::with_id("settings", "Settings…").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Quit Sotto").build(app)?;
    let menu = MenuBuilder::new(app)
        .item(&pause)
        .item(&polish)
        .separator()
        .item(&settings)
        .separator()
        .item(&quit)
        .build()?;

    TrayIconBuilder::with_id("main")
        .icon(tray::idle_icon())
        .tooltip("Sotto")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "settings" => {
                if let Some(w) = app.get_webview_window("settings") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            "quit" => app.exit(0),
            "pause" => {
                let c = &app.state::<AppState>().controls;
                let paused = !c.paused.load(Ordering::Relaxed);
                c.paused.store(paused, Ordering::Relaxed);
                tracing::info!(paused, "pause toggled from tray");
            }
            "polish_off" => set_polish_runtime(app, PolishMode::Off),
            "polish_rules" => set_polish_runtime(app, PolishMode::Rules),
            "polish_ai" => set_polish_runtime(app, PolishMode::Ai),
            _ => {}
        })
        .build(app)?;
    Ok(())
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

fn set_polish_runtime(app: &tauri::AppHandle, mode: PolishMode) {
    let st = app.state::<AppState>();
    st.controls.polish_mode.store(mode.as_u8(), Ordering::Relaxed);
    let mut cfg = st.cfg.lock().unwrap();
    cfg.polish.mode = mode;
    let _ = cfg.save();
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
fn spawn_pipeline(app: tauri::AppHandle, controls: Controls, cfg: Config) {
    let (tx, rx) = crossbeam_channel::unbounded::<DictationEvent>();
    let suppressed = Arc::new(AtomicBool::new(false));

    // Global hotkey listener.
    {
        let hotkey_idx = controls.hotkey_idx.clone();
        let activation = controls.activation.clone();
        let paused = controls.paused.clone();
        let supp = suppressed.clone();
        std::thread::spawn(move || hotkey::run_listener(hotkey_idx, activation, tx, supp, paused));
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
    std::thread::spawn(move || {
        let mut recorder = audio::Recorder::new(level);
        let mut asr = asr::Asr::new();

        // Local closure: emit "cancelled", clear the flag, and reset session
        // state. Called between stages when the user has hit Escape.
        let abort = |app: &tauri::AppHandle| {
            emit_state(app, "cancelled");
            tracing::info!("dictation cancelled by user");
        };

        for event in rx {
            match event {
                DictationEvent::Start => {
                    // A fresh cycle — clear any stale cancel flag from before.
                    cancelled.store(false, Ordering::SeqCst);
                    match recorder.start() {
                        Ok(()) => {
                            listening.store(true, Ordering::Relaxed);
                            // Capture the focus target NOW so an Alt-Tab during
                            // dictation doesn't reroute the injection. Restored
                            // just before we send the keystrokes below.
                            focus_target.store(inject::capture_focus(), Ordering::Relaxed);
                            // Warm the LLM sidecar while the user speaks (AI mode
                            // only) so its cold model-load doesn't block after Stop.
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
                },
                DictationEvent::Cancel => {
                    // Only meaningful if a dictation is actually in flight —
                    // Escape while idle is a no-op. Snapshot listening BEFORE
                    // we set cancelled so downstream stages see the flag.
                    let was_listening = listening.load(Ordering::Relaxed);
                    cancelled.store(true, Ordering::SeqCst);
                    if was_listening {
                        // User pressed Escape while still speaking — stop the
                        // recorder and discard the samples. The `for event in
                        // rx` loop is currently at rest, so we can do this
                        // inline; if a Stop then arrives it'll see empty
                        // samples and skip.
                        listening.store(false, Ordering::Relaxed);
                        let _ = recorder.stop();
                        focus_target.store(0, Ordering::Relaxed);
                        abort(&app);
                    }
                    // If Cancel arrives during transcribe/polish (worker is
                    // blocked mid-op), the flag will be checked when the
                    // current stage finishes — see the Stop handler below.
                }
                DictationEvent::Stop => {
                    listening.store(false, Ordering::Relaxed);
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
                    // Cancel arrived while we were reading `samples` — abort
                    // now, before wasting cycles on transcription.
                    if cancelled.swap(false, Ordering::SeqCst) {
                        abort(&app);
                        continue;
                    }
                    emit_state(&app, "transcribing");
                    let text = match asr.transcribe(&samples) {
                        Ok(t) => t,
                        Err(err) => {
                            emit_state(&app, "error");
                            tracing::error!(?err, "transcription failed");
                            continue;
                        }
                    };
                    // ASR is synchronous and can't be interrupted mid-call —
                    // but a cancel that arrived DURING transcription is honored
                    // here, so at least the polish + injection never happen.
                    if cancelled.swap(false, Ordering::SeqCst) {
                        abort(&app);
                        continue;
                    }
                    if text.is_empty() {
                        emit_state(&app, "error");
                        continue;
                    }
                    if polisher.uses_ai_tier(&text) {
                        emit_state(&app, "polishing");
                    }
                    let polished = polisher.polish(&text);
                    // Final chance to abort before injection lands text in the
                    // user's window.
                    if cancelled.swap(false, Ordering::SeqCst) {
                        abort(&app);
                        continue;
                    }
                    if polished.is_empty() {
                        emit_state(&app, "error");
                        continue;
                    }
                    // Restore focus to the original target before injecting.
                    // No-op if the user hasn't moved focus, cheap otherwise.
                    inject::restore_focus(focus_target.swap(0, Ordering::Relaxed));
                    suppressed.store(true, Ordering::SeqCst);
                    let result = inject::inject_text(&polished, injection_mode);
                    suppressed.store(false, Ordering::SeqCst);
                    match result {
                        Ok(()) => {
                            // Leave the polished text on the clipboard so a
                            // mis-focused injection can still be Ctrl+V'd into
                            // the intended app. Paste mode already touches the
                            // clipboard mid-inject then restores the old value;
                            // this re-sets ours as the final state.
                            if let Ok(mut cb) = arboard::Clipboard::new() {
                                let _ = cb.set_text(polished.clone());
                            }
                            history.push(polished.clone());
                            emit_state(&app, "done");
                            emit_history(&app, &history);
                            tracing::info!("injected: {polished:?}");
                        }
                        Err(err) => {
                            emit_state(&app, "error");
                            tracing::error!(?err, "injection failed");
                        }
                    }
                }
            }
        }
    });
}

fn emit_state(app: &tauri::AppHandle, s: &str) {
    let _ = app.emit("overlay-state", s);
    if let Some(tray) = app.tray_by_id("main") {
        let icon = if s == "listening" { tray::active_icon() } else { tray::idle_icon() };
        let _ = tray.set_icon(Some(icon));
    }
    if s != "idle" {
        show_overlay(app);
    }
}

fn show_overlay(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("overlay") {
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
    println!("polished => {out:?}  ({} ms)", t.elapsed().as_millis());
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
