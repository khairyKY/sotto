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
mod bug_reports;
mod config;
mod correction;
mod history;
mod hotkey;
mod inject;
mod job;
mod journal;
mod lecture;
mod llm;
mod polish;
mod recordings;
mod scratchpad;
mod single_instance;
mod sounds;
mod startup;
mod stats;
mod transform;
mod tray;
mod uia;

use config::{ActivationMode, AppTone, Config, DictEntry, EntryKind, InjectionMode, PolishMode, VocabEntry};
use hotkey::DictationEvent;
use single_instance::SingleInstanceGuard;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};
use tracing_subscriber::fmt::writer::MakeWriterExt;

/// Ignore captured clips shorter than this — almost always an accidental tap.
const MIN_CLIP_SAMPLES: usize = 16_000 / 2; // 0.5s at 16 kHz
/// Home card reason for a take the crash journal brought back (N4).
const RECOVERED: &str = "Recovered after a restart";

// ── chunked transcription (all counts are 16 kHz samples) ──────────────
//
// Nothing below CHUNK_MIN_SAMPLES is ever cut, so a normal short dictation
// never touches any of this and behaves exactly as it did before.
//
// ponytail: the pause finder's threshold is fixed, no adaptation to the room's
// noise floor. If a noisy mic stops finding pauses, every take just falls back
// to cutting at CHUNK_MAX_SAMPLES — degraded, not broken. Raise SILENCE_RMS if
// that happens. (Speech detection is relative to the take: see `has_speech`.)
const CHUNK_MIN_SAMPLES: usize = 16_000 * 10; // don't cut before 10s of audio
const CHUNK_MAX_SAMPLES: usize = 16_000 * 24; // cut by 24s, pause or not
const SILENCE_WIN: usize = 16_000 * 300 / 1000; // pause detector window, 300ms
const SILENCE_STEP: usize = 16_000 * 50 / 1000; // scan stride, 50ms
/// RMS below this counts as a pause a chunk may be cut in (see `chunk_cut`).
const SILENCE_RMS: f32 = 0.015;
const SPEECH_FRAME: usize = 16_000 * 30 / 1000; // speech detector frame, 30ms
/// Voiced frames a stretch needs to hold speech: 150 ms, shorter than a word.
const SPEECH_MIN_FRAMES: usize = 5;
/// A voiced frame is this many times the take's noise floor (about 10 dB)...
const SPEECH_OVER_FLOOR: f32 = 3.0;
/// ...and never quieter than this (-60 dBFS), for a noise-gated mic whose
/// pauses are digital zero.
const SPEECH_MIN_RMS: f32 = 0.001;
/// Start-of-take audio left out of the noise-floor estimate (device warm-up).
const FLOOR_SKIP: usize = 16_000 / 5; // 200 ms
/// Each chunk after the first also re-hears this much audio before its cut,
/// so a word the cut clipped is heard whole on one side (see `seam_start`).
const CHUNK_OVERLAP: usize = 16_000; // 1s
/// How many trailing words of the text so far `join_text` searches for the
/// words a chunk's overlap repeats, and how many edge words either side of
/// that repeat it may drop as the halves of a clipped word.
const SEAM_WORDS: usize = 8;
const SEAM_SLACK: usize = 2;
/// How often the audio thread wakes to collect audio while recording.
const CHUNK_POLL: Duration = Duration::from_millis(250);

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
    /// Live replacements: `(all phrases longest-first, replacement, kind)`.
    /// Disabled entries are filtered out here rather than checked per
    /// dictation, so the hot path stays a plain iteration. `kind` decides
    /// which polish-pipeline pass applies the entry — see `EntryKind`.
    pub dictionary: Arc<Mutex<Vec<(Vec<String>, String, EntryKind)>>>,
    /// Master switch for all replacements — live-toggled from Settings.
    pub replacements_enabled: Arc<AtomicBool>,
    /// "New line"/"new paragraph" voice commands — live-toggled from Settings,
    /// read by the polisher on every dictation (see `polish.rs`).
    pub formatting_commands: Arc<AtomicBool>,
    /// Spoken numbers -> digits — live-toggled from Settings, read by the
    /// polisher on every dictation (see `polish.rs`'s `normalize_numbers`).
    pub number_formatting: Arc<AtomicBool>,
    /// Phonetic correction toward trained vocabulary — live-toggled from
    /// Settings (see `polish.rs`'s `apply_phonetic_corrections`).
    pub phonetic_correction: Arc<AtomicBool>,
    /// "straight" or "curly" — glyphs spoken quote commands produce. Shares
    /// `formatting_commands`'s toggle, not its own; see `polish.rs`.
    pub quote_style: Arc<Mutex<String>>,
    /// Default tone instruction for AI polish; empty = off. Same live-editable
    /// shape as `dictionary` — settings writes it, the polisher reads it live.
    pub tone: Arc<Mutex<String>>,
    /// Proper nouns/jargon hinted to the AI-tier system prompt — see
    /// `config::PolishConfig::vocabulary`.
    pub vocabulary: Arc<Mutex<Vec<VocabEntry>>>,
    /// The Pronunciation Trainer's armed word, or `None`. Deliberately NOT
    /// persisted to config — it's a live-session UI state, not a setting.
    /// One-shot: the next Start consumes it (see `arm_take`), and that one
    /// take short-circuits in `process_take` before polish/injection: a
    /// training utterance is never meant to land in a real document, it's
    /// just "what did the engine hear", compared against this word.
    pub training_word: Arc<Mutex<Option<Training>>>,
    /// Per-app tone overrides: (app name, tone instruction) pairs.
    pub app_tones: Arc<Mutex<Vec<(String, String)>>>,
    /// Apps where spoken casing commands run (#27). Empty unless
    /// `variable_recognition` is on (read at launch).
    pub code_editors: Arc<Mutex<Vec<String>>>,
    /// Transform chords (#18), read live by the hotkey listener — see
    /// `config::Transform`. `transforms_enabled` arms them.
    pub transforms: Arc<Mutex<Vec<config::Transform>>>,
    pub transforms_enabled: Arc<AtomicBool>,
    pub history: history::History,
    /// Live mic RMS (f32 bits), written by the audio callback.
    pub level: Arc<AtomicU32>,
    /// True while recording — gates the level-event emitter.
    pub listening: Arc<AtomicBool>,
    /// True while a lecture is being captured (#23). Its own flag, not
    /// `listening`: that one means a dictation take, to the hotkey's toggle,
    /// Escape and the pill's waveform. Written by the audio thread.
    pub lecture: Arc<AtomicBool>,
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
    /// Live "keep recordings" flag — same shape as `stats_enabled`, but for
    /// `recordings::record` (audio + raw + polished text). Off by default;
    /// unlike stats this persists dictated content, so it's opt-in per
    /// `RetentionConfig`.
    pub retention_enabled: Arc<AtomicBool>,
    /// Selected input device name, or `None` for the OS default. Read by
    /// `Recorder::start` on every dictation, so a change applies immediately.
    pub microphone: Arc<Mutex<Option<String>>>,
    /// Current overlay pill state ("idle", "listening", …) — written by
    /// emit_state, read by the overlay hit-test poll to know the pill's
    /// width and whether it currently shows a clickable button.
    pub overlay_state: Arc<Mutex<String>>,
    /// Configured pill anchor ("bottom-center", …) — read live by
    /// show_overlay/position_overlay so a Settings change repositions the
    /// pill without a restart.
    pub overlay_position: Arc<Mutex<String>>,
    /// Opt-in "keep the idle pill on screen, click it to start a dictation"
    /// mode (N1) — read by spawn_overlay_hittest to decide whether idle is
    /// currently clickable.
    pub overlay_always_visible: Arc<AtomicBool>,
    /// Soft start/stop recording ticks — live-toggled from settings.
    pub sound_enabled: Arc<AtomicBool>,
    /// The stashed-but-undelivered take, if any — drives the tray menu's
    /// "Retry last dictation" enablement and Home's alert card. `None` means
    /// the last dictation landed (or there hasn't been one).
    pub take_info: Arc<Mutex<Option<TakeInfo>>>,
}

/// What Home's alert card and the tray need to know about a stashed take.
/// `words` is 0 when the take never got as far as a transcript (cancelled
/// mid-recording), in which case the UI falls back to the audio length.
#[derive(Clone, serde::Serialize)]
pub struct TakeInfo {
    reason: String,
    words: usize,
    audio_ms: u64,
}

impl Controls {
    fn from_config(cfg: &Config) -> Self {
        Self {
            paused: Arc::new(AtomicBool::new(false)),
            polish_mode: Arc::new(AtomicU8::new(cfg.polish.mode.as_u8())),
            activation: Arc::new(AtomicU8::new(cfg.activation_mode.as_u8())),
            hotkey_idx: Arc::new(AtomicUsize::new(hotkey::index_of(&cfg.hotkey))),
            ai_min_words: Arc::new(AtomicUsize::new(cfg.polish.ai_min_words)),
            dictionary: Arc::new(Mutex::new(live_dictionary(&cfg.dictionary))),
            replacements_enabled: Arc::new(AtomicBool::new(cfg.replacements_enabled)),
            formatting_commands: Arc::new(AtomicBool::new(cfg.formatting_commands)),
            number_formatting: Arc::new(AtomicBool::new(cfg.number_formatting)),
            phonetic_correction: Arc::new(AtomicBool::new(cfg.phonetic_correction)),
            quote_style: Arc::new(Mutex::new(cfg.quote_style.clone())),
            tone: Arc::new(Mutex::new(cfg.tone.clone())),
            vocabulary: Arc::new(Mutex::new(cfg.polish.vocabulary.clone())),
            training_word: Arc::new(Mutex::new(None)),
            app_tones: Arc::new(Mutex::new(
                cfg.app_tones.iter().map(|e| (e.app.clone(), e.tone.clone())).collect(),
            )),
            code_editors: Arc::new(Mutex::new(if cfg.variable_recognition { cfg.code_editors.clone() } else { Vec::new() })),
            history: history::History::new(cfg.persist_history),
            level: Arc::new(AtomicU32::new(0)),
            listening: Arc::new(AtomicBool::new(false)),
            lecture: Arc::new(AtomicBool::new(false)),
            focus_target: Arc::new(AtomicIsize::new(0)),
            cancelled: Arc::new(AtomicBool::new(false)),
            stats_enabled: Arc::new(AtomicBool::new(cfg.stats_enabled)),
            retention_enabled: Arc::new(AtomicBool::new(cfg.retention.enabled)),
            microphone: Arc::new(Mutex::new(cfg.microphone.clone())),
            overlay_state: Arc::new(Mutex::new("idle".to_string())),
            overlay_position: Arc::new(Mutex::new(cfg.overlay.position.clone())),
            overlay_always_visible: Arc::new(AtomicBool::new(cfg.overlay.always_visible)),
            sound_enabled: Arc::new(AtomicBool::new(cfg.sound_enabled)),
            take_info: Arc::new(Mutex::new(None)),
            transforms: Arc::new(Mutex::new(cfg.transforms.clone())),
            transforms_enabled: Arc::new(AtomicBool::new(cfg.transforms_enabled)),
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
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default = "dto_enabled_default")]
    enabled: bool,
    // EntryKind's own #[default] (Word) covers a caller that omits this.
    // In practice the frontend always sends it explicitly now — see
    // ui/index.js's dict-add/snip-add and the example-chip handlers.
    #[serde(default)]
    kind: EntryKind,
}

fn dto_enabled_default() -> bool {
    true
}
#[derive(serde::Serialize, serde::Deserialize)]
struct AppToneDto {
    app: String,
    tone: String,
}
#[derive(serde::Serialize, Clone)]
struct HistoryDto {
    time: String,
    text: String,
    /// What the ASR heard, for History's review diff (#25). This session's
    /// rows only ("" after a reload); IPC to our own window, never logged.
    raw: String,
    tier: String,
    fallback: String,
}
impl From<history::HistoryEntry> for HistoryDto {
    fn from(e: history::HistoryEntry) -> Self {
        Self { time: e.time, text: e.text, raw: e.raw, tier: e.tier, fallback: e.fallback }
    }
}
/// One Pronunciation Trainer attempt, emitted as a "pronunciation-sample"
/// event — see `process_take`'s training short-circuit.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PronunciationSampleDto {
    word: String,
    heard: String,
    matched: bool,
}
/// "Something smart just happened" — a correction the overlay flyout shows in
/// place of the plain done checkmark. `more` is how many further corrections
/// fired in the same take beyond the one shown.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct FlyoutDto {
    heard: String,
    corrected: String,
    more: usize,
}
#[derive(serde::Serialize, serde::Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct VocabEntryDto {
    word: String,
    heard_as: Vec<String>,
    /// Pronunciation Trainer's rolling match history — see `VocabEntry::recent`.
    #[serde(default)]
    recent: Vec<bool>,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelDto {
    /// Stable id ("parakeet-v3" / "whisper-turbo") — what `set_asr_model`
    /// accepts. The frontend must select by this, never by `name`.
    id: String,
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
    replacements_enabled: bool,
    formatting_commands: bool,
    number_formatting: bool,
    phonetic_correction: bool,
    /// "straight" or "curly" — drives the quote-style segmented control.
    quote_style: String,
    /// AI-polish vocabulary hints — also the Pronunciation Trainer's list of
    /// already-trained words and what's been heard for each.
    vocabulary: Vec<VocabEntryDto>,
    /// Default tone instruction; "" = off.
    tone: String,
    app_tones: Vec<AppToneDto>,
    /// Apps that get an Enter after a dictation lands (#21).
    auto_send: Vec<String>,
    /// Variable recognition (#27): shows the Code editors list, and the list.
    variable_recognition: bool,
    code_editors: Vec<String>,
    history: Vec<HistoryDto>,
    models: Vec<ModelDto>,
    hotkey_options: Vec<HotkeyOption>,
    theme: String,
    /// Configured pill anchor — drives the Settings 3×3 picker's selected tile.
    overlay_position: String,
    /// "Always show the pill" opt-in — drives the Settings toggle switch.
    overlay_always_visible: bool,
    /// Current input device name, or "" for the OS default.
    microphone: String,
    microphone_options: Vec<String>,
    paused: bool,
    sound_enabled: bool,
    /// "Usage stats" toggle state — without this the Data & privacy switch
    /// always rendered off on load regardless of the real setting, since
    /// `s.statsEnabled` had no field here to read.
    stats_enabled: bool,
    has_take: bool,
    /// Details for Home's "last dictation wasn't delivered" card.
    take_info: Option<TakeInfo>,
    /// Where config/stats live, for the settings "Open folder" link.
    data_dir: String,
    /// Where the ~2.8 GB of models live — the same as `data_dir` unless the
    /// user pointed `assets_dir` somewhere with more room.
    assets_dir: String,
    zoom: f64,
    /// Currently configured ASR engine id ("parakeet-v3" / "whisper-turbo") —
    /// also derivable from `models`, but exposed directly so the language
    /// dropdown can disable itself without re-deriving it.
    asr_model: String,
    /// BCP-47 code, or "auto".
    asr_language: String,
    /// "Keep recordings" master switch — drives the Data & privacy toggle.
    retention_enabled: bool,
    /// Configured size budget in MB, for the row's descriptive text.
    retention_max_mb: u64,
    /// Where kept recordings live, for the "Open folder" link. Same value
    /// as `assets_dir` + "/recordings" — computed here so the frontend
    /// never has to know that join itself.
    recordings_dir: String,
    /// Current total size of kept recordings, in MB.
    recordings_size_mb: u64,
    /// "Keep history" toggle (N4) — History survives a restart.
    history_persist: bool,
    /// Transforms page (#18): the chord master switch, the list, and the
    /// seeds "Reset to defaults" restores. Items are `config::Transform` as
    /// is: `{ name, chord, prompt, keep_words }`.
    transforms_enabled: bool,
    transforms: Vec<config::Transform>,
    transform_defaults: Vec<config::Transform>,
    /// Shows the Pronunciation page's Calibrate card (#22).
    calibration: bool,
    /// Lecture mode (#23): shows Home's card and the tray item, whether a
    /// lecture is being captured now, and the transcripts' folder ("" until
    /// the first one is written).
    lecture_mode: bool,
    lecture: bool,
    lectures_dir: String,
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
    // Lock ONCE, before the struct literal. Every temporary inside a struct
    // expression lives until the end of the enclosing statement, so two
    // `take_info.lock()` calls in two fields both hold a guard at the same
    // time — and std's Mutex isn't reentrant, so the second one deadlocks the
    // thread. Tauri runs sync commands on the main thread, so that hung the
    // message loop: no tray, no clicks, no hotkey, nothing.
    let take_info = c.take_info.lock().unwrap().clone();
    let parakeet_installed = config::model_dir().join("encoder-model.int8.onnx").exists();
    // Falls back to the known approximate download size when not yet on disk
    // — `dir_size_mb` of a path that doesn't exist yet is `None`, and a blank
    // "MB" reads as a bug, not as "not downloaded yet".
    let parakeet_size = dir_size_mb(config::model_dir()).map(|mb| format!("{mb} MB")).unwrap_or_else(|| "639 MB".into());
    let whisper_installed = config::whisper_model_path("whisper-turbo").exists();
    let whisper_size = dir_size_mb(config::whisper_model_path("whisper-turbo")).map(|mb| format!("{mb} MB")).unwrap_or_else(|| "547 MB".into());
    let egyptian_installed = config::whisper_model_path("egyptian-small").exists();
    let egyptian_size = dir_size_mb(config::whisper_model_path("egyptian-small")).map(|mb| format!("{mb} MB")).unwrap_or_else(|| "465 MB".into());
    SettingsPayload {
        hotkey_options,
        hotkey: hotkey::SUPPORTED_HOTKEYS[idx].1.to_string(),
        activation: activation.to_string(),
        polish: polish.to_string(),
        threshold: c.ai_min_words.load(Ordering::Relaxed),
        launch_login: startup::is_enabled(),
        start_hidden: cfg.start_hidden,
        // Straight from config, not from `controls.dictionary` — the live copy
        // has disabled entries stripped, and the editor must still show them.
        dictionary: cfg
            .dictionary
            .iter()
            .map(|e| DictEntryDto {
                spoken: e.spoken.clone(),
                replacement: e.replacement.clone(),
                aliases: e.aliases.clone(),
                enabled: e.enabled,
                // Always Some by the time this runs: load_or_init backfills
                // every entry via migrate_entry_kinds before AppState exists.
                kind: e.kind.unwrap_or_default(),
            })
            .collect(),
        replacements_enabled: cfg.replacements_enabled,
        formatting_commands: cfg.formatting_commands,
        number_formatting: cfg.number_formatting,
        phonetic_correction: cfg.phonetic_correction,
        quote_style: c.quote_style.lock().unwrap().clone(),
        vocabulary: c
            .vocabulary
            .lock()
            .unwrap()
            .iter()
            .map(|e| VocabEntryDto { word: e.word.clone(), heard_as: e.heard_as.clone(), recent: e.recent.clone() })
            .collect(),
        tone: c.tone.lock().unwrap().clone(),
        app_tones: c.app_tones.lock().unwrap().iter().map(|(a, t)| AppToneDto { app: a.clone(), tone: t.clone() }).collect(),
        auto_send: cfg.auto_send.clone(),
        variable_recognition: cfg.variable_recognition,
        code_editors: cfg.code_editors.clone(),
        history: c.history.snapshot().into_iter().map(HistoryDto::from).collect(),
        models: vec![
            ModelDto {
                id: "parakeet-v3".into(),
                name: "Parakeet v3".into(),
                variant: "· English".into(),
                meta: "NVIDIA · int8 quantized".into(),
                state: if parakeet_installed { "installed" } else { "download" }.into(),
                size: parakeet_size,
                selected: cfg.asr.model == "parakeet-v3",
            },
            ModelDto {
                id: "whisper-turbo".into(),
                name: "Whisper turbo".into(),
                variant: "· 99 languages".into(),
                meta: "OpenAI · large-v3-turbo q5_0".into(),
                state: if whisper_installed { "installed" } else { "download" }.into(),
                size: whisper_size,
                selected: cfg.asr.model == "whisper-turbo",
            },
            ModelDto {
                id: "egyptian-small".into(),
                name: "Egyptian Arabic".into(),
                variant: "· عامية + English".into(),
                meta: "Whisper small · code-switch tuned".into(),
                state: if egyptian_installed { "installed" } else { "download" }.into(),
                size: egyptian_size,
                selected: cfg.asr.model == "egyptian-small",
            },
        ],
        theme: cfg.theme.clone(),
        overlay_position: c.overlay_position.lock().unwrap().clone(),
        overlay_always_visible: c.overlay_always_visible.load(Ordering::Relaxed),
        microphone: c.microphone.lock().unwrap().clone().unwrap_or_default(),
        microphone_options: audio::list_input_devices(),
        paused: c.paused.load(Ordering::Relaxed),
        sound_enabled: c.sound_enabled.load(Ordering::Relaxed),
        stats_enabled: c.stats_enabled.load(Ordering::Relaxed),
        has_take: take_info.is_some(),
        take_info,
        data_dir: config::data_dir().display().to_string(),
        assets_dir: config::assets_dir().display().to_string(),
        zoom: cfg.zoom,
        asr_model: cfg.asr.model.clone(),
        asr_language: cfg.asr.language.clone(),
        retention_enabled: c.retention_enabled.load(Ordering::Relaxed),
        retention_max_mb: cfg.retention.max_mb,
        recordings_dir: recordings::recordings_dir().display().to_string(),
        recordings_size_mb: recordings::total_size_mb(),
        history_persist: cfg.persist_history,
        transforms_enabled: c.transforms_enabled.load(Ordering::Relaxed),
        transforms: c.transforms.lock().unwrap().clone(),
        transform_defaults: config::default_transforms(),
        calibration: cfg.calibration,
        lecture_mode: cfg.lecture_mode,
        lecture: c.lecture.load(Ordering::Relaxed),
        lectures_dir: Some(lecture::dir()).filter(|d| d.is_dir()).map(|d| d.display().to_string()).unwrap_or_default(),
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
    let saved: Vec<DictEntry> = entries
        .into_iter()
        .filter(|e| !e.spoken.trim().is_empty())
        .map(|e| DictEntry {
            spoken: e.spoken,
            replacement: e.replacement,
            aliases: e.aliases.into_iter().filter(|a| !a.trim().is_empty()).collect(),
            enabled: e.enabled,
            kind: Some(e.kind),
        })
        .collect();
    *state.controls.dictionary.lock().unwrap() = live_dictionary(&saved);
    let mut cfg = state.cfg.lock().unwrap();
    cfg.dictionary = saved;
    let _ = cfg.save();
}

/// Master switch for all dictionary/snippet replacements.
#[tauri::command]
fn set_replacements_enabled(enabled: bool, state: tauri::State<'_, AppState>) {
    state.controls.replacements_enabled.store(enabled, Ordering::Relaxed);
    let mut cfg = state.cfg.lock().unwrap();
    cfg.replacements_enabled = enabled;
    let _ = cfg.save();
}

/// Switch for "new line"/"new paragraph" voice commands (F1).
#[tauri::command]
fn set_formatting_commands(enabled: bool, state: tauri::State<'_, AppState>) {
    state.controls.formatting_commands.store(enabled, Ordering::Relaxed);
    let mut cfg = state.cfg.lock().unwrap();
    cfg.formatting_commands = enabled;
    let _ = cfg.save();
}

/// Switch for spoken-number -> digits normalization.
#[tauri::command]
fn set_number_formatting(enabled: bool, state: tauri::State<'_, AppState>) {
    state.controls.number_formatting.store(enabled, Ordering::Relaxed);
    let mut cfg = state.cfg.lock().unwrap();
    cfg.number_formatting = enabled;
    let _ = cfg.save();
}

/// Switch for phonetic correction toward trained vocabulary.
#[tauri::command]
fn set_phonetic_correction(enabled: bool, state: tauri::State<'_, AppState>) {
    state.controls.phonetic_correction.store(enabled, Ordering::Relaxed);
    let mut cfg = state.cfg.lock().unwrap();
    cfg.phonetic_correction = enabled;
    let _ = cfg.save();
}

#[tauri::command]
fn set_quote_style(style: String, state: tauri::State<'_, AppState>) {
    *state.controls.quote_style.lock().unwrap() = style.clone();
    let mut cfg = state.cfg.lock().unwrap();
    cfg.quote_style = style;
    let _ = cfg.save();
}

/// Pronunciation Trainer: arm/disarm listening for `word`. The next
/// take's Start consumes the arm (`None` disarms), and that take short-circuits in
/// `process_take` — see its training-word check — reporting what was heard
/// instead of polishing/injecting it.
/// With `calibration`, `word` is a whole Calibrate sentence (#22).
#[tauri::command]
fn set_pronunciation_target(word: Option<String>, calibration: Option<bool>, state: tauri::State<'_, AppState>) {
    *state.controls.training_word.lock().unwrap() = word
        .filter(|w| !w.trim().is_empty())
        .map(|target| Training { target, calibration: calibration.unwrap_or(false) });
}

/// Pronunciation Trainer's (and Calibrate's) "add this correction" action.
/// `heard` always joins `word`'s vocabulary hint (Whisper prompt + AI
/// polish; works when the mishearing is close enough for the model to
/// bridge, e.g. "clawed" -> "Claude"). It also becomes a deterministic
/// Word-kind dictionary entry (works in every polish mode) only when it's
/// no real English word (`polish::dictionary_safe`, #94): an entry for
/// "cloud" would rewrite every "cloud". Skips the entry if an existing one
/// already covers this exact phrase, so retraining the same mishearing
/// twice doesn't pile up duplicates. Returns whether it's an entry, so the
/// page can say which happened.
#[tauri::command]
fn add_pronunciation_correction(word: String, heard: String, state: tauri::State<'_, AppState>) -> bool {
    let word = word.trim().to_string();
    let heard = heard.trim().to_string();
    if word.is_empty() || heard.is_empty() {
        return false;
    }
    let mut cfg = state.cfg.lock().unwrap();

    match cfg.polish.vocabulary.iter_mut().find(|e| e.word.eq_ignore_ascii_case(&word)) {
        Some(entry) => {
            if !entry.heard_as.iter().any(|h| h.eq_ignore_ascii_case(&heard)) {
                entry.heard_as.push(heard.clone());
            }
        }
        None => cfg.polish.vocabulary.push(VocabEntry { word: word.clone(), heard_as: vec![heard.clone()], recent: Vec::new() }),
    }
    *state.controls.vocabulary.lock().unwrap() = cfg.polish.vocabulary.clone();

    let exact = polish::dictionary_safe(&heard);
    let already_covered =
        cfg.dictionary.iter().any(|e| e.enabled && e.phrases().iter().any(|p| p.eq_ignore_ascii_case(&heard)));
    if exact && !already_covered {
        cfg.dictionary.push(DictEntry {
            spoken: heard,
            replacement: word,
            aliases: vec![],
            enabled: true,
            kind: Some(EntryKind::Word),
        });
        *state.controls.dictionary.lock().unwrap() = live_dictionary(&cfg.dictionary);
    }

    let _ = cfg.save();
    exact
}

/// Config entries -> the shape the polisher iterates: enabled ones only, each
/// flattened to all its phrases longest-first. One function so the startup path
/// and the live-edit path can't drift apart.
fn live_dictionary(entries: &[DictEntry]) -> Vec<(Vec<String>, String, EntryKind)> {
    entries
        .iter()
        .filter(|e| e.enabled)
        .map(|e| {
            let phrases = e.phrases().into_iter().map(str::to_string).collect();
            (phrases, e.replacement.clone(), e.kind.unwrap_or_default())
        })
        .collect()
}
#[tauri::command]
fn set_tone(tone: String, state: tauri::State<'_, AppState>) {
    *state.controls.tone.lock().unwrap() = tone.clone();
    let mut cfg = state.cfg.lock().unwrap();
    cfg.tone = tone;
    let _ = cfg.save();
}
#[tauri::command]
fn set_app_tones(tones: Vec<AppToneDto>, state: tauri::State<'_, AppState>) {
    let pairs: Vec<(String, String)> = tones
        .into_iter()
        .filter(|e| !e.app.trim().is_empty())
        .map(|e| (e.app, e.tone))
        .collect();
    *state.controls.app_tones.lock().unwrap() = pairs.clone();
    let mut cfg = state.cfg.lock().unwrap();
    cfg.app_tones = pairs.into_iter().map(|(app, tone)| AppTone { app, tone }).collect();
    let _ = cfg.save();
}
/// The apps that get an Enter after a dictation (#21). Read at each
/// delivery, so no live mirror.
#[tauri::command]
fn set_auto_send(apps: Vec<String>, state: tauri::State<'_, AppState>) {
    let mut cfg = state.cfg.lock().unwrap();
    cfg.auto_send = apps.into_iter().map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect();
    let _ = cfg.save();
}
/// The apps spoken casing commands run in (#27). Live only while
/// `variable_recognition` is on; saved either way.
#[tauri::command]
fn set_code_editors(apps: Vec<String>, state: tauri::State<'_, AppState>) {
    let mut cfg = state.cfg.lock().unwrap();
    cfg.code_editors = apps.into_iter().map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect();
    if cfg.variable_recognition {
        *state.controls.code_editors.lock().unwrap() = cfg.code_editors.clone();
    }
    let _ = cfg.save();
}
/// The Transforms page's list (#18). Nameless entries are dropped; a chord
/// `hotkey::parse_chord` can't read is kept but never fires.
#[tauri::command]
fn set_transforms(transforms: Vec<config::Transform>, state: tauri::State<'_, AppState>) {
    let list: Vec<config::Transform> = transforms.into_iter().filter(|t| !t.name.trim().is_empty()).collect();
    *state.controls.transforms.lock().unwrap() = list.clone();
    let mut cfg = state.cfg.lock().unwrap();
    cfg.transforms = list;
    let _ = cfg.save();
}
#[tauri::command]
fn set_transforms_enabled(enabled: bool, state: tauri::State<'_, AppState>) {
    state.controls.transforms_enabled.store(enabled, Ordering::Relaxed);
    let mut cfg = state.cfg.lock().unwrap();
    cfg.transforms_enabled = enabled;
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

/// Change the pill's screen anchor (N1). Persists + repositions immediately —
/// the window may currently be hidden, in which case this just pre-positions
/// it for the next show, same as `set_zoom`'s "apply now, not just on save".
#[tauri::command]
fn set_overlay_position(position: String, app: tauri::AppHandle, state: tauri::State<'_, AppState>) {
    if !OVERLAY_ANCHORS.contains(&position.as_str()) {
        return;
    }
    *state.controls.overlay_position.lock().unwrap() = position.clone();
    {
        let mut cfg = state.cfg.lock().unwrap();
        cfg.overlay.position = position.clone();
        let _ = cfg.save();
    }
    if let Some(w) = app.get_webview_window("overlay") {
        position_overlay(&w, &position);
        // The canvas anchors the pill against the same edge the window is
        // anchored to, so it has to hear about the change too — otherwise the
        // window moves to the new corner while the pill stays tucked to the
        // old one until the next restart.
        let _ = w.emit("overlay-position", &position);
    }
}

/// Opt-in "keep the idle pill on screen" mode (N1). Only needs to act
/// immediately while the pill is actually idle right now — every other state
/// already shows itself via `emit_state` regardless of this setting.
#[tauri::command]
fn set_overlay_always_visible(enabled: bool, app: tauri::AppHandle, state: tauri::State<'_, AppState>) {
    state.controls.overlay_always_visible.store(enabled, Ordering::Relaxed);
    {
        let mut cfg = state.cfg.lock().unwrap();
        cfg.overlay.always_visible = enabled;
        let _ = cfg.save();
    }
    let is_idle = *state.controls.overlay_state.lock().unwrap() == "idle";
    if is_idle {
        if enabled {
            show_overlay(&app);
        } else if let Some(w) = app.get_webview_window("overlay") {
            let _ = w.hide();
        }
    }
}

/// What overlay.js needs at boot to know whether idle should stay on screen
/// and be clickable. `get_settings` also carries this (for the Settings
/// window's picker/toggle), but that command does real work — dir sizes,
/// model install state — the overlay window has no use for.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct OverlaySettingsDto {
    always_visible: bool,
    /// The overlay canvas is 280x120 — big enough for the widest toast — but
    /// the pill inside it is much smaller, so drawing it centred leaves it
    /// floating ~60px off the screen edge it's supposed to be tucked against.
    /// The canvas anchors the pill itself; it needs to know which edge.
    position: String,
}
#[tauri::command]
fn get_overlay_settings(state: tauri::State<'_, AppState>) -> OverlaySettingsDto {
    OverlaySettingsDto {
        always_visible: state.controls.overlay_always_visible.load(Ordering::Relaxed),
        position: state.controls.overlay_position.lock().unwrap().clone(),
    }
}

/// The overlay's idle-pill body click when always-visible is on (N1) — same
/// event as the hotkey, just reachable by mouse. Mirrors cancel_dictation /
/// retry_last: push the event, let the worker do the rest. Pause blocks it
/// like it blocks the hotkey (#60); `false` tells the trainer why nothing began.
#[tauri::command]
fn start_dictation(state: tauri::State<'_, AppState>) -> bool {
    if state.controls.paused.load(Ordering::Relaxed) {
        return false;
    }
    let _ = state.tx.send(DictationEvent::Start);
    // Mid-lecture the Start is still sent, for the pill that says why it's
    // refused (#23); `false` keeps the trainer off "Listening".
    !state.controls.lecture.load(Ordering::Relaxed)
}

/// Start or stop lecture capture (#23): Home's card and the tray menu. The
/// audio thread does the rest and answers with "lecture-changed".
#[tauri::command]
fn set_lecture(on: bool, state: tauri::State<'_, AppState>) {
    if state.cfg.lock().unwrap().lecture_mode {
        let _ = state.tx.send(DictationEvent::Lecture(on));
    }
}

/// The Pronunciation Trainer's "Stop" click — same event a hotkey release/
/// second toggle-press would send, just reachable by mouse since this flow
/// bypasses the hotkey entirely (see `start_dictation` call in `index.js`'s
/// listen button).
#[tauri::command]
fn stop_dictation(state: tauri::State<'_, AppState>) {
    let _ = state.tx.send(DictationEvent::Stop);
}

/// The overlay reports back once its own countdown (done/error/cancelled/
/// nomodel toast) times out to idle — those transitions are timed inside
/// overlay.js, not driven by a DictationEvent, so nothing else updates
/// `overlay_state` for them. Only matters in always-visible mode: the
/// hit-test poll reads `overlay_state` to know whether idle is currently
/// clickable, and a stale non-idle value here would leave it keyed to a
/// button that overlay.js no longer draws.
#[tauri::command]
fn mark_overlay_idle(state: tauri::State<'_, AppState>) {
    *state.controls.overlay_state.lock().unwrap() = "idle".to_string();
}

/// Switch the configured ASR engine. Takes over on the next take, no restart
/// (#10): the transcribe thread `sync`s from this config between takes, and
/// the hotkey press that starts the next take loads the new model while the
/// user speaks. A picked engine that's still downloading waits until its
/// files land, and the old one keeps transcribing meanwhile.
#[tauri::command]
fn set_asr_model(model: String, state: tauri::State<'_, AppState>) {
    let valid = model == "parakeet-v3" || model == "whisper-turbo" || model == "egyptian-small";
    if !valid { return; }
    let mut cfg = state.cfg.lock().unwrap();
    cfg.asr.model = model;
    let _ = cfg.save();
}

/// Set the expected dictation language ("auto" or a BCP-47 code). Applies
/// from the next take, same as `set_asr_model` — and Parakeet ignores this
/// entirely, English-only regardless of what's stored here.
#[tauri::command]
fn set_asr_language(language: String, state: tauri::State<'_, AppState>) {
    let language = language.trim();
    let valid = language == "auto"
        || (!language.is_empty() && language.len() <= 8 && language.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    if !valid { return; }
    let mut cfg = state.cfg.lock().unwrap();
    cfg.asr.language = language.to_string();
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

/// Home's "Dismiss" — throw the stashed take away so the alert card stays gone
/// instead of coming back on the next refresh.
#[tauri::command]
fn dismiss_take(state: tauri::State<'_, AppState>) {
    let _ = state.tx.send(DictationEvent::Dismiss);
}

/// A history row's ↻ — re-run the current polish tier + dictionary over that
/// row's text and leave the result on the clipboard.
#[tauri::command]
fn repolish_copy(text: String, state: tauri::State<'_, AppState>) {
    let _ = state.tx.send(DictationEvent::Repolish(text));
}

/// A history row's "Use what I said" (#25): the raw transcript onto the
/// clipboard + a "Copied original" pill. ponytail: copy, not an in-place
/// swap — by now focus is on Sotto's own window, and re-selecting the last
/// injection in another app isn't something we can do cleanly.
#[tauri::command]
fn copy_original(raw: String, app: tauri::AppHandle) {
    copy_text(raw);
    emit_state(&app, "copied");
}

/// A history row's flag — "this came out wrong," logged locally to
/// `bug_reports::record` with whatever config context was live at the
/// moment, for Kai to review and file a real GitHub issue from by hand.
/// Nothing is sent anywhere; see `bug_reports.rs`.
#[tauri::command]
fn flag_transcription(text: String, state: tauri::State<'_, AppState>) {
    let text = text.trim().to_string();
    if text.is_empty() {
        return;
    }
    // The row's raw transcript, for `--replay-flags` (#8).
    let raw = state.controls.history.raw_for(&text);
    let cfg = state.cfg.lock().unwrap();
    bug_reports::record(&bug_reports::BugReport {
        t: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        text,
        raw,
        // The engine that ran last, not the configured one: right after a hot
        // switch (#10) the latest rows still came from the old engine. Config
        // only before this session's first transcript.
        asr_engine: asr::last_engine().unwrap_or_else(|| cfg.asr.model.clone()),
        asr_language: cfg.asr.language.clone(),
        polish_mode: format!("{:?}", cfg.polish.mode).to_lowercase(),
        dictionary_count: cfg.dictionary.iter().filter(|e| e.enabled).count(),
        vocabulary_count: cfg.polish.vocabulary.len(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
    });
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

#[tauri::command]
fn set_retention_enabled(enabled: bool, state: tauri::State<'_, AppState>) {
    state.controls.retention_enabled.store(enabled, Ordering::Relaxed);
    let mut cfg = state.cfg.lock().unwrap();
    cfg.retention.enabled = enabled;
    let _ = cfg.save();
}

#[tauri::command]
fn clear_recordings() {
    recordings::clear();
}

/// "Keep history". Off deletes `history.jsonl` — see `History::set_persist`.
#[tauri::command]
fn set_history_persist(enabled: bool, state: tauri::State<'_, AppState>) {
    state.controls.history.set_persist(enabled);
    let mut cfg = state.cfg.lock().unwrap();
    cfg.persist_history = enabled;
    let _ = cfg.save();
}

#[tauri::command]
fn clear_history(app: tauri::AppHandle, state: tauri::State<'_, AppState>) {
    state.controls.history.clear();
    emit_history(&app, &state.controls.history);
}

/// Send tracing to a file, not just stdout.
///
/// Release builds are `windows_subsystem = "windows"` — there is no console, so
/// every log line was going nowhere. When someone reported "it doesn't work"
/// there was literally nothing to read: no file to ask for, no way to diagnose
/// anything remotely. That's untenable once other people are running this.
///
/// Two files, no rotation crate: the current run and the previous one. The
/// interesting failure is often the run *before* the user thought to complain.
///
/// `cli` (--transcribe/--polish/--replay-flags) logs to stderr only: those
/// runs often happen while the app is up, and rotating here would split the
/// live app's log and drop its previous run (#82).
fn init_logging(cli: bool) {
    // ORT's info-level logging is ~345 lines per launch of internal graph
    // detail, which buries the handful of lines that actually explain a
    // failure. Keep its warnings and errors, drop the narration.
    // `SOTTO_LOG=debug` (or `ort::logging=info`) brings it all back.
    let filter = std::env::var("SOTTO_LOG").unwrap_or_else(|_| "info,ort::logging=warn".into());
    let dir = config::data_dir().join("logs");
    let file = (!cli).then(|| std::fs::create_dir_all(&dir).ok()).flatten().and_then(|_| {
        let path = dir.join("sotto.log");
        let _ = std::fs::rename(&path, dir.join("sotto.log.1"));
        std::fs::File::create(&path).ok()
    });
    match file {
        // Tee: the file is for users, stdout still works under `cargo run`.
        Some(f) => tracing_subscriber::fmt()
            .with_env_filter(filter)
            // No ANSI escapes — this file gets opened in Notepad and pasted
            // into a chat window.
            .with_ansi(false)
            .with_writer(std::io::stdout.and(std::sync::Arc::new(f)))
            .init(),
        // CLI mode, or couldn't open the log (read-only dir, disk full). Still
        // run. stderr keeps a CLI's stdout just its result.
        None => tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init(),
    }

    // A panic is exactly the case where the log has to survive, and the default
    // hook prints to a stderr that doesn't exist in a windowed build.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("PANIC: {info}");
        default_hook(info);
    }));

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        data_dir = %config::data_dir().display(),
        assets_dir = %config::assets_dir().display(),
        "Sotto starting"
    );
}

// ── main ───────────────────────────────────────────────────────────────
fn main() -> anyhow::Result<()> {
    let cli = std::env::args().any(|a| ["--transcribe", "--lecture", "--polish", "--replay-flags", "--transform"].contains(&a.as_str()));
    init_logging(cli);
    init_ort();

    if let Some(path) = arg_value("--transcribe") {
        return run_transcribe_once(&path);
    }
    if let Some(path) = arg_value("--lecture") {
        return lecture::run_once(&path);
    }
    if let Some(text) = arg_value("--polish") {
        return run_polish_once(&text);
    }
    if let Some(text) = arg_value("--transform") {
        return transform::run_once(&text);
    }
    if std::env::args().any(|a| a == "--replay-flags") {
        return run_replay_flags(arg_value("--replay-flags"));
    }

    let Some(_guard) = SingleInstanceGuard::acquire()? else {
        tracing::info!("another Sotto instance is already running — bringing it forward");
        single_instance::wake_running();
        return Ok(());
    };

    let cfg = Config::load_or_init()?;
    // Hands the uninstaller a relocated assets_dir it can't get from TOML
    // parsing — see config::write_assets_dir_marker and nsis/hooks.nsi (#53).
    config::write_assets_dir_marker(&cfg);
    tracing::info!(
        cfg = ?config_for_log(&cfg),
        dictionary = cfg.dictionary.len(),
        vocabulary = cfg.polish.vocabulary.len(),
        path = %Config::path().display(),
        "loaded config"
    );
    let controls = Controls::from_config(&cfg);
    scratchpad::init(&cfg);

    // The worker's event channel is created here so both the pipeline and the
    // Tauri commands / tray (via AppState.tx) can drive it — e.g. Retry.
    let (tx, rx) = crossbeam_channel::unbounded::<DictationEvent>();
    let state = AppState { controls: controls.clone(), cfg: Mutex::new(cfg.clone()), tx: tx.clone() };
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            get_settings, set_hotkey, set_activation, set_polish, set_threshold,
            set_dictionary, set_tone, set_app_tones, set_auto_send, set_code_editors, set_transforms, set_transforms_enabled, set_launch_login, set_start_hidden, set_theme, copy_text,
            set_overlay_position, set_overlay_always_visible, get_overlay_settings, start_dictation, stop_dictation, set_lecture, mark_overlay_idle,
            set_asr_model, set_asr_language,
            open_url, check_update, install_update, retry_last, cancel_dictation, dismiss_take,
            repolish_copy, flag_transcription,
            copy_original, get_stats, clear_stats, set_stats_enabled, set_retention_enabled, clear_recordings, set_history_persist, clear_history,
            set_microphone, set_sound_enabled, set_zoom,
            set_replacements_enabled,
            set_formatting_commands, set_number_formatting, set_phonetic_correction, set_quote_style, set_pronunciation_target, add_pronunciation_correction,
            menu_action,
            assets::assets_status, assets::download_assets,
            scratchpad::scratchpad_state, scratchpad::scratchpad_page, scratchpad::scratchpad_delete,
            scratchpad::scratchpad_park, scratchpad::scratchpad_inject
        ])
        .setup(move |app| {
            build_tray(app)?;
            // A second launch (Start menu, shortcut) lands here (#61).
            let handle = app.handle().clone();
            single_instance::on_wake(move || {
                if let Some(w) = handle.get_webview_window("settings") {
                    let _ = w.unminimize();
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            });
            if let Some(w) = app.get_webview_window("overlay") {
                let _ = w.set_ignore_cursor_events(true);
                position_overlay(&w, &cfg.overlay.position);
                harden_utility_window(&w, true);
                if cfg.overlay.always_visible {
                    let _ = w.show();
                    harden_utility_window(&w, true);
                }
            }
            if let Some(w) = app.get_webview_window("menu") {
                harden_utility_window(&w, false);
                // The lecture item (#23) is one more 33px row than
                // tauri.conf.json's 380 has room for.
                if cfg.lecture_mode {
                    let _ = w.set_size(tauri::LogicalSize::new(230.0, 413.0));
                }
            }
            if let Some(w) = app.get_webview_window("settings") {
                if (cfg.zoom - 1.0).abs() > f64::EPSILON {
                    let _ = w.set_zoom(cfg.zoom.clamp(ZOOM_MIN, ZOOM_MAX));
                }
                // First run (#54): a hidden window would hide the download
                // progress too, so show it until every asset is on disk.
                if !cfg.start_hidden || !assets::assets_status().ready {
                    let _ = w.show();
                }
            }
            spawn_pipeline(app.handle().clone(), controls.clone(), cfg.clone(), tx.clone(), rx.clone());
            spawn_overlay_hittest(
                app.handle().clone(),
                controls.overlay_state.clone(),
                controls.overlay_always_visible.clone(),
                controls.overlay_position.clone(),
            );
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

    let dark = tray::dark_for(&app.state::<AppState>().cfg.lock().unwrap().theme);
    TrayIconBuilder::with_id("main")
        .icon(tray::icon(false, dark))
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
                        harden_utility_window(&w, false);
                        let _ = w.show();
                        harden_utility_window(&w, false);
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
        if let Some(tray) = app.tray_by_id("main") {
            let _ = tray.set_tooltip(Some(if paused { "Sotto (paused)" } else { "Sotto" }));
        }
        let _ = app.emit("paused-changed", paused);
    } else if action == "lecture" {
        let state = app.state::<AppState>();
        let on = !state.controls.lecture.load(Ordering::Relaxed);
        set_lecture(on, state);
    } else {
        if let Some(w) = app.get_webview_window("settings") {
            let _ = w.show();
            let _ = w.set_focus();
            // "settings" too: it's a modal now, and navigate() is what opens it.
            let _ = w.emit("navigate", action);
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

/// The 9 supported pill anchors, kebab-case exactly as config.toml and the
/// Settings 3×3 picker use.
const OVERLAY_ANCHORS: [&str; 9] = [
    "top-left", "top-center", "top-right",
    "middle-left", "middle-center", "middle-right",
    "bottom-left", "bottom-center", "bottom-right",
];

/// Margin off the screen edge for every anchor — reuses the constant that
/// used to be bottom-only (8px) so every edge looks equally deliberate.
const OVERLAY_MARGIN: f64 = 8.0;

/// Anchor arithmetic: monitor size + pill size + anchor name -> top-left
/// physical position. Split out from `position_overlay` so it's unit-testable
/// without a real window/monitor. An unrecognized anchor (hand-edited config)
/// falls back to bottom-center rather than panicking or landing off-screen.
/// Where the pill sits INSIDE the overlay canvas, in CSS px. Must stay in
/// lockstep with `pillOrigin()` in ui/overlay.js — the canvas draws with it and
/// the cursor hit-test below measures with it, so a divergence means clicking
/// empty space. Anchored rather than centred because the canvas (280x120) is
/// sized for the widest toast, and centring a 46x16 tucked pill in it left the
/// pill floating ~60px off the edge it is supposed to be tucked against.
const OVERLAY_EDGE_INSET: f64 = 8.0; // room for the pill's own drop shadow
fn overlay_pill_origin(cw: f64, ch: f64, pw: f64, ph: f64, anchor: &str) -> (f64, f64) {
    let mut parts = anchor.split('-');
    let v = parts.next().unwrap_or("bottom");
    let h = parts.next().unwrap_or("center");
    let x = match h {
        "left" => OVERLAY_EDGE_INSET,
        "right" => cw - pw - OVERLAY_EDGE_INSET,
        _ => (cw - pw) / 2.0,
    };
    let y = match v {
        "top" => OVERLAY_EDGE_INSET,
        "bottom" => ch - ph - OVERLAY_EDGE_INSET,
        _ => (ch - ph) / 2.0,
    };
    (x.round(), y.round())
}

/// `area_x`/`area_y` are the top-left of the placement area (a monitor's work
/// area, which may not start at (0, 0) — either a secondary monitor, or a
/// primary monitor with a top- or left-docked taskbar).
fn anchor_xy(
    area_x: f64,
    area_y: f64,
    area_w: f64,
    area_h: f64,
    pill_w: f64,
    pill_h: f64,
    anchor: &str,
    margin: f64,
) -> (i32, i32) {
    let (h, v) = match anchor {
        "top-left" => ("left", "top"),
        "top-center" => ("center", "top"),
        "top-right" => ("right", "top"),
        "middle-left" => ("left", "middle"),
        "middle-center" => ("center", "middle"),
        "middle-right" => ("right", "middle"),
        "bottom-left" => ("left", "bottom"),
        "bottom-right" => ("right", "bottom"),
        _ => ("center", "bottom"), // bottom-center, and the unknown-anchor fallback
    };
    let x = match h {
        "left" => area_x + margin,
        "right" => area_x + area_w - pill_w - margin,
        _ => area_x + (area_w - pill_w) / 2.0,
    };
    let y = match v {
        "top" => area_y + margin,
        "middle" => area_y + (area_h - pill_h) / 2.0,
        _ => area_y + area_h - pill_h - margin,
    };
    (x as i32, y as i32)
}

/// Target (x, y) for the overlay window at `anchor`, without moving it —
/// shared by `position_overlay` (applies it immediately) and the hittest
/// loop (nudges it for the taskbar auto-hide bar first, then applies it).
/// Uses the monitor's WORK AREA, not its full size: a pinned taskbar's
/// reserved strip is excluded from the work area automatically, so this
/// alone fixes placement for anyone with a normal (non-auto-hide) taskbar.
/// Uses the window's real `outer_size()` rather than a hardcoded guess —
/// same fix already applied to the tray menu's positioning, see `build_tray`.
fn overlay_target_xy(w: &tauri::WebviewWindow, anchor: &str) -> Option<(i32, i32)> {
    let mon = w.current_monitor().ok()??;
    let work = mon.work_area();
    let size = w.outer_size().ok()?;
    Some(anchor_xy(
        work.position.x as f64,
        work.position.y as f64,
        work.size.width as f64,
        work.size.height as f64,
        size.width as f64,
        size.height as f64,
        anchor,
        OVERLAY_MARGIN * mon.scale_factor(),
    ))
}

/// Position the (always-on-top, transparent) overlay window at the
/// configured anchor.
fn position_overlay(w: &tauri::WebviewWindow, anchor: &str) {
    if let Some((x, y)) = overlay_target_xy(w, anchor) {
        let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
    }
}

/// How far Windows' bottom-docked auto-hide taskbar has slid onto the
/// primary monitor: 0 when fully hidden (a couple hover-trigger px poking
/// above the edge), up to the bar's own height when fully shown. Used to
/// nudge a bottom-anchored overlay up so the taskbar sliding in on hover
/// doesn't cover the very pill the user is reaching for.
/// ponytail: single Shell_TrayWnd probe on the primary monitor — switch to
/// ABM_GETAUTOHIDEBAREX if a secondary-monitor taskbar or a non-bottom dock
/// ever needs to be handled.
fn taskbar_intrusion_px() -> f64 {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GetSystemMetrics, GetWindowRect, SM_CYSCREEN,
    };
    use windows::core::PCWSTR;
    unsafe {
        let class: Vec<u16> = "Shell_TrayWnd\0".encode_utf16().collect();
        let Ok(tray) = FindWindowW(PCWSTR(class.as_ptr()), PCWSTR::null()) else {
            return 0.0;
        };
        if tray.is_invalid() {
            return 0.0;
        }
        let mut rect = RECT::default();
        if GetWindowRect(tray, &mut rect).is_err() {
            return 0.0;
        }
        let screen_h = GetSystemMetrics(SM_CYSCREEN) as f64;
        (screen_h - rect.top as f64).clamp(0.0, (rect.bottom - rect.top) as f64)
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
        let listening = controls.listening.clone();
        let transforms = controls.transforms.clone();
        let transforms_enabled = controls.transforms_enabled.clone();
        std::thread::spawn(move || {
            hotkey::run_listener(
                hotkey_idx, activation, tx, supp, paused, cancelled, listening, transforms, transforms_enabled,
            )
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

    // ── Dictation pipeline ───────────────────────────────────────────────
    //
    // Two threads, because they have incompatible jobs. The audio thread owns
    // the recorder (cpal's Stream is !Send — it can never leave the thread
    // that created it) and must always be free to answer the hotkey. The
    // transcribe thread owns the ASR model, the polisher and the retry stash,
    // and runs the slow transcribe → polish → inject pipeline.
    //
    // These used to be one thread, and that was the bug: a Start arriving
    // while a take was transcribing sat unread in the channel until that take
    // had been fully injected, so hitting the hotkey to dictate again during a
    // transcription appeared to do nothing. Splitting them buys both of the
    // things that were missing — recording starts immediately no matter what
    // the model is doing (takes queue and deliver in order), and the audio
    // thread can hand over finished chunks *during* recording, so the wait
    // after release is the tail of the take instead of all of it.
    let injection_mode = cfg.injection_mode;
    let mut polisher = polish::Polisher::new(controls.clone(), cfg.llm.clone());
    // The polisher decides at build time whether AI is available, so on a
    // first run it's rebuilt once the LLM finishes downloading (#54).
    let mut ai_ready = llm::Llm::is_available();
    let (polisher_controls, llm_cfg) = (controls.clone(), cfg.llm.clone());
    let history = controls.history.clone();
    let listening = controls.listening.clone();
    let level = controls.level.clone();
    let focus_target = controls.focus_target.clone();
    let cancelled = controls.cancelled.clone();
    let stats_enabled = controls.stats_enabled.clone();
    let retention_enabled = controls.retention_enabled.clone();
    let training_word = controls.training_word.clone();
    let microphone = controls.microphone.clone();
    let sound_enabled = controls.sound_enabled.clone();
    let take_info = controls.take_info.clone();
    let polish_mode = controls.polish_mode.clone();
    let chunking = cfg.chunked_transcription;
    let auto_tone = cfg.auto_tone;
    let (work_tx, work_rx) = crossbeam_channel::unbounded::<Work>();
    // True while a Pronunciation-Trainer peek transcription is in flight, so
    // the audio thread never piles a second one behind it — see Work::Peek.
    let peek_busy = Arc::new(AtomicBool::new(false));
    // The take being recorded (or last recorded), so a peek that finishes
    // after the next take has started can tell it's stale (#39).
    let live_take = Arc::new(AtomicU64::new(0));

    // N4: a take that was in flight (or sitting undelivered in the stash)
    // when Sotto last died comes back as the stash. Runs before the audio
    // thread exists, so none of this run's takes can pass for a leftover. No
    // focus target: a window handle from the last run could be anything now.
    let recovered = journal::recover(MIN_CLIP_SAMPLES).map(|(files, samples)| {
        tracing::info!(files = files.len(), secs = samples.len() / 16_000, "recovered an undelivered take");
        let mode = PolishMode::from_u8(polish_mode.load(Ordering::Relaxed));
        let mut take = Take::new(samples, 0, mode, files, None);
        take.reason = RECOVERED;
        take
    });
    *take_info.lock().unwrap() = recovered.as_ref().map(TakeInfo::from);
    journal::spawn();

    // Transcribe thread.
    {
        let app = app.clone();
        let listening = listening.clone();
        let cancelled = cancelled.clone();
        let stats_enabled = stats_enabled.clone();
        let retention_enabled = retention_enabled.clone();
        let peek_busy = peek_busy.clone();
        let live_take = live_take.clone();
        let lecture_on = controls.lecture.clone();
        std::thread::spawn(move || {
            let mut asr = asr::Asr::new();
            // Warm the ASR model before serving work, so the first dictation
            // doesn't eat the ~5s load. Anything that arrives meanwhile just
            // queues on the channel.
            asr.preload();
            // Same reasoning for Harper's lint set (~640 ms).
            polisher.warm_rules();
            // The last dictation, kept in memory so Escape/error is
            // recoverable via Retry. Cleared on successful delivery.
            let mut stash: Option<Take> = recovered;
            // Text from chunks transcribed while the user was still talking,
            // keyed by take. Usually holds at most one entry — but a queued
            // earlier take can still be waiting while the next one records.
            let mut partials: HashMap<u64, Partial> = HashMap::new();
            // Lecture mode (#23): the last chunk's text, for the next seam.
            let mut lecture_prev = String::new();

            loop {
                let work = match work_rx.recv_timeout(asr.idle_wait()) {
                    Ok(work) => work,
                    // No work for `asr.idle_unload_secs`: give the model's RAM
                    // back (#12); the next hotkey press reloads it. Not while
                    // recording: a long take with chunking off sends nothing
                    // between its Prewarm and its Finish. Nor mid-lecture.
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                        if lecture::may_unload(listening.load(Ordering::Relaxed), lecture_on.load(Ordering::Relaxed)) {
                            asr.unload();
                        }
                        continue;
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                };
                // Between takes, pick up Settings changes: engine (#10),
                // language, trained words for the prompt. "Between" = no take
                // holds chunk text, so every chunk of a take and its tail go
                // through one engine.
                if partials.is_empty() {
                    asr.sync(&app.state::<AppState>().cfg.lock().unwrap());
                }
                if !ai_ready && llm::Llm::is_available() {
                    tracing::info!("AI polish assets landed, picking them up without a restart");
                    polisher = polish::Polisher::new(polisher_controls.clone(), llm_cfg.clone());
                    ai_ready = true;
                }
                match work {
                    // An empty chunk is a silent one — register the take as
                    // chunked so Finish only transcribes the tail, but never
                    // hand the model room tone to put words to.
                    Work::Chunk { id, samples, .. } if samples.is_empty() => {
                        partials.entry(id).or_insert_with(|| Partial::Text(String::new()));
                    }
                    Work::Chunk { id, samples, seam } => match asr.transcribe(&samples) {
                        Ok(text) => {
                            if let Partial::Text(acc) =
                                partials.entry(id).or_insert_with(|| Partial::Text(String::new()))
                            {
                                *acc = join_text(acc, &text, seam);
                            }
                        }
                        // One failed chunk poisons the whole take: its partial
                        // text now has a hole in it, and delivering text with a
                        // silent gap is far worse than being slow. Marked here,
                        // re-transcribed whole at Finish. Chunking stays a pure
                        // optimization — it can never change what gets injected.
                        Err(err) => {
                            tracing::warn!(?err, id, "chunk failed — will re-transcribe whole take");
                            partials.insert(id, Partial::Poisoned);
                        }
                    },
                    Work::Finish { id, mut take } => {
                        match partials.remove(&id) {
                            Some(Partial::Text(text)) => take.prefix_text = text,
                            // Poisoned (or never chunked): start from zero.
                            Some(Partial::Poisoned) | None => take.sent = 0,
                        }
                        process_take(
                            &app, &mut asr, &polisher, &history, &suppressed, &cancelled,
                            &listening, injection_mode, &stats_enabled, &retention_enabled, take, &mut stash,
                        );
                        publish_take(&app, &take_info, &stash);
                    }
                    Work::Cancelled { id, mut take } => {
                        partials.remove(&id);
                        // Only worth stashing if there's enough audio to retry.
                        if take.samples.len() >= MIN_CLIP_SAMPLES {
                            record_outcome(&take, &stats_enabled, "cancelled");
                            take.reason = "Cancelled";
                            stash = Some(take);
                        }
                        publish_take(&app, &take_info, &stash);
                    }
                    Work::Retry => {
                        if let Some(take) = stash.take() {
                            cancelled.store(false, Ordering::SeqCst);
                            show_overlay(&app);
                            tracing::info!(has_text = take.raw_text.is_some(), "retrying last dictation");
                            process_take(
                                &app, &mut asr, &polisher, &history, &suppressed, &cancelled,
                                &listening, injection_mode, &stats_enabled, &retention_enabled, take, &mut stash,
                            );
                        } else {
                            tracing::info!("retry requested but nothing stashed");
                        }
                        publish_take(&app, &take_info, &stash);
                    }
                    Work::Dismiss => {
                        stash = None;
                        publish_take(&app, &take_info, &stash);
                    }
                    // Queued behind any in-flight work on purpose: if the model
                    // is busy the sidecar can wait, and the user is still
                    // talking either way. Same for the ASR model, if idle
                    // unloaded it or the engine just switched (#10, #12): it
                    // loads while the user speaks, and anything this take
                    // sends meanwhile waits in the queue behind it.
                    Work::Prewarm => {
                        polisher.prewarm();
                        asr.preload();
                    }
                    Work::Peek { id, samples, target } => {
                        // Transcribe the take-so-far; if the trained word is in
                        // it, tell the UI to ignite the glow. Best-effort — a
                        // failed/late peek just means the glow waits for the
                        // final sample. `peek_busy` gates the next one, and a
                        // stale peek still clears it: it was the one in flight.
                        if let Ok(text) = asr.transcribe(&samples) {
                            if peek_fires(&text, &target, id, live_take.load(Ordering::Relaxed)) {
                                let _ = app.emit("word-detected", target);
                            }
                        }
                        peek_busy.store(false, Ordering::Relaxed);
                    }
                    Work::Repolish(text) => {
                        let out = polisher.polish(&text);
                        if !out.text.is_empty() {
                            if let Ok(mut cb) = arboard::Clipboard::new() {
                                let _ = cb.set_text(out.text.clone());
                            }
                            // Brief "done" pill as the only feedback — the
                            // result is on the clipboard, nothing is injected.
                            show_overlay(&app);
                            emit_state(&app, "done");
                            tracing::info!("re-polished and copied {} chars", out.text.len());
                        }
                    }
                    Work::Transform(t) => {
                        run_transform(&app, &polisher, &suppressed, &cancelled, &listening, injection_mode, &t)
                    }
                    Work::Scratchpad(a) => scratchpad::run(&app, a, &suppressed, injection_mode),
                    // Where lecture mode leaves the dictation path (#23): a
                    // chunk's text goes to the transcript file as it lands.
                    // No partials, no Finish, so never `process_take`:
                    // nothing polished, typed, or put in history or stats.
                    // Each line stands alone, so an engine picked in Settings
                    // mid-lecture takes over at its next chunk (`sync` above).
                    Work::Lecture(piece) => lecture::write(|s| asr.transcribe(s), piece, &mut lecture_prev),
                }
            }
        });
    }

    // Audio thread.
    std::thread::spawn(move || {
        let mut recorder = audio::Recorder::new(level, microphone);
        // Everything captured for the take in progress, at 16 kHz, and how
        // much of it has already gone to the transcribe thread as chunks. The
        // full audio is always kept: chunking hands out *copies*, so a Retry
        // can still re-transcribe the take from scratch.
        let mut all: Vec<f32> = Vec::new();
        let mut sent: usize = 0;
        let mut take_id: u64 = 0;
        let mut recording = false;
        // Whether this take's audio goes to the crash journal (#44).
        let mut journaled = true;
        // The trainer word this take was started for, if any (#47).
        let mut take_training: Option<Training> = None;
        // Auto tone (#28): the field-context read started with this take.
        let mut take_context: Option<std::sync::mpsc::Receiver<String>> = None;
        // Pronunciation-Trainer detection peeks — last one's time, for cadence.
        let mut last_peek = std::time::Instant::now();
        // Lecture mode (#23): the lecture being captured, if any. It records
        // into `all`/`sent` like a take, on the same recorder and the same
        // poll; what differs is where its chunks go (see the poll below).
        let mut lecture: Option<lecture::Capture> = None;

        loop {
            // Poll only while recording — an idle Sotto has no reason to wake
            // up four times a second for the rest of the session.
            let event = if recording {
                match rx.recv_timeout(CHUNK_POLL) {
                    Ok(e) => Some(e),
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => None,
                    Err(_) => break,
                }
            } else {
                match rx.recv() {
                    Ok(e) => Some(e),
                    Err(_) => break,
                }
            };

            // Timed out mid-recording: collect what's been captured and hand
            // over a chunk if enough has piled up behind a pause.
            let Some(event) = event else {
                let fresh = recorder.drain();
                // A lecture is cut by the same `next_chunk` (whatever
                // `chunked_transcription` says: it has no one-shot path), but
                // each chunk is written to its file rather than held for a
                // Finish, and the audio behind it is dropped, not kept for a
                // Retry. No crash journal either: its transcript is on disk
                // already, and a journal would come back as a dictation.
                if let Some(capture) = lecture.as_mut() {
                    all.extend(fresh);
                    if let Some(piece) = capture.step(&mut all, &mut sent) {
                        let _ = work_tx.send(Work::Lecture(piece));
                    }
                    continue;
                }
                if journaled {
                    journal::append(take_id, &fresh);
                }
                all.extend(fresh);
                if chunking {
                    if let Some((from, end, speech)) = next_chunk(&all, sent) {
                        // A chunk with no speech in it is sent *empty*: fed
                        // room tone the model invents a phrase and it gets
                        // injected as if it were speech (one-shot could do that
                        // once per take; chunking would do it once per chunk).
                        //
                        // Empty rather than not at all — the take still has to
                        // be recorded as chunked, or Finish sees no partial,
                        // assumes chunking never happened, and re-transcribes
                        // the whole take from zero.
                        let _ = work_tx.send(Work::Chunk {
                            id: take_id,
                            samples: if speech { all[from..end].to_vec() } else { Vec::new() },
                            seam: from < sent,
                        });
                        tracing::info!(
                            id = take_id,
                            secs = (end - sent) as f64 / 16_000.0,
                            silent = !speech,
                            "chunk sent"
                        );
                        sent = end;
                    }
                }
                // Pronunciation Trainer: while a word is armed, peek-transcribe
                // the take-so-far ~1x/sec so the glow ignites on real detection,
                // not just mic level. peek_busy keeps peeks from piling up on
                // the transcribe thread; a training take is short and otherwise
                // leaves that thread idle, so one peek in flight is plenty.
                // A calibration sentence has no glow to light, so no peeks.
                if let Some(target) = take_training.as_ref().filter(|t| !t.calibration).map(|t| t.target.clone()) {
                    if all.len() >= MIN_CLIP_SAMPLES
                        && last_peek.elapsed() >= Duration::from_millis(900)
                        && !peek_busy.swap(true, Ordering::Relaxed)
                    {
                        last_peek = std::time::Instant::now();
                        let _ = work_tx.send(Work::Peek { id: take_id, samples: all.clone(), target });
                    }
                }
                continue;
            };

            match lecture::gate(lecture.is_some(), &event) {
                lecture::Gate::Refuse => {
                    // The trainer's arm goes with the Start, as always (#47).
                    arm_take(&mut training_word.lock().unwrap(), true);
                    emit_state(&app, "lectureon");
                    tracing::info!("start refused: lecture capture is on");
                    continue;
                }
                lecture::Gate::Ignore => continue,
                lecture::Gate::Pass => {}
            }

            match event {
                DictationEvent::Lecture(true) => {
                    // A take being recorded is never cut short for one.
                    if recording {
                        tracing::info!("lecture start ignored: already recording");
                        continue;
                    }
                    match recorder.start() {
                        Ok(()) => {
                            all.clear();
                            sent = 0;
                            recording = true;
                            if sound_enabled.load(Ordering::Relaxed) {
                                sounds::tick();
                            }
                            let capture = lecture::Capture::new();
                            tracing::info!(file = %capture.file().display(), "lecture capture started");
                            lecture = Some(capture);
                            publish_lecture(&app, true);
                        }
                        Err(err) => {
                            emit_state(&app, "error");
                            tracing::error!(?err, "failed to start lecture capture");
                        }
                    }
                }
                DictationEvent::Lecture(false) => {
                    let Some(capture) = lecture.take() else { continue };
                    recording = false;
                    if sound_enabled.load(Ordering::Relaxed) {
                        sounds::tock();
                    }
                    all.extend(recorder.stop().unwrap_or_default());
                    if let Some(piece) = capture.tail(&all, sent) {
                        let _ = work_tx.send(Work::Lecture(piece));
                    }
                    all.clear();
                    sent = 0;
                    publish_lecture(&app, false);
                    tracing::info!("lecture capture stopped");
                }
                DictationEvent::Start => {
                    // Start is idempotent mid-take. The hotkey, the overlay
                    // pill and the trainer button can all send it, and
                    // recorder.start() drops the stream and clears its buffer,
                    // so a second Start used to wipe the take and orphan its
                    // already-sent chunks (#37).
                    let training = arm_take(&mut training_word.lock().unwrap(), recording);
                    if recording {
                        tracing::info!(id = take_id, "start ignored: already recording");
                        continue;
                    }
                    cancelled.store(false, Ordering::SeqCst);
                    match recorder.start() {
                        Ok(()) => {
                            take_id += 1;
                            all.clear();
                            sent = 0;
                            recording = true;
                            last_peek = std::time::Instant::now();
                            // peek_busy is NOT reset here: a peek from the last
                            // take may still be transcribing, and it clears the
                            // flag itself when it lands (dropped as stale).
                            // Resetting it let a second peek queue behind it.
                            live_take.store(take_id, Ordering::Relaxed);
                            journaled = journals_take(training.is_some());
                            take_training = training;
                            listening.store(true, Ordering::Relaxed);
                            if sound_enabled.load(Ordering::Relaxed) {
                                sounds::tick();
                            }
                            // Capture the focus target NOW so an Alt-Tab during
                            // dictation doesn't reroute the injection.
                            focus_target.store(inject::capture_focus(), Ordering::Relaxed);
                            // Auto tone (#28): read the field's text off this
                            // thread, for the AI tier only (nothing else uses it).
                            let ai = PolishMode::from_u8(polish_mode.load(Ordering::Relaxed)) == PolishMode::Ai;
                            take_context = (auto_tone && ai).then(|| uia::spawn_read(focus_target.load(Ordering::Relaxed)));
                            // Warm the LLM sidecar while the user speaks.
                            let _ = work_tx.send(Work::Prewarm);
                            show_overlay(&app);
                            emit_state(&app, "listening");
                            tracing::info!(id = take_id, "listening");
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
                        recording = false;
                        if sound_enabled.load(Ordering::Relaxed) {
                            sounds::tock();
                        }
                        let rest = recorder.stop().unwrap_or_default();
                        if journaled {
                            journal::append(take_id, &rest);
                        }
                        all.extend(rest);
                        cancelled.store(false, Ordering::SeqCst); // consumed here
                        let take = Take::new(
                            std::mem::take(&mut all),
                            focus_target.swap(0, Ordering::Relaxed),
                            PolishMode::from_u8(polish_mode.load(Ordering::Relaxed)),
                            vec![journal::path_for(take_id)],
                            take_training.take(),
                        );
                        emit_state(&app, "cancelled");
                        tracing::info!("dictation cancelled while recording");
                        let _ = work_tx.send(Work::Cancelled { id: take_id, take });
                    }
                }
                DictationEvent::Stop => {
                    // A stray Stop (the take already ended: pill-click stop
                    // then a hold-mode key release, Escape then release, a
                    // failed start) is a no-op. Handling it emitted "idle",
                    // which hid a Transcribing pill or cut a cancelled/error
                    // toast short.
                    if !recording {
                        continue;
                    }
                    // Only tock if we were actually recording (a stray Stop —
                    // e.g. toggle-mode release edge — shouldn't chirp).
                    if listening.swap(false, Ordering::Relaxed)
                        && sound_enabled.load(Ordering::Relaxed)
                    {
                        sounds::tock();
                    }
                    recording = false;
                    match recorder.stop() {
                        Ok(s) => {
                            if journaled {
                                journal::append(take_id, &s);
                            }
                            all.extend(s)
                        }
                        Err(err) => {
                            emit_state(&app, "error");
                            tracing::error!(?err, "failed to stop capture");
                            // Everything drained while recording is still good
                            // audio — hand it over to be stashed rather than
                            // dropped, which also clears any chunk partials
                            // held for this take.
                            let take = Take::new(
                                std::mem::take(&mut all),
                                focus_target.swap(0, Ordering::Relaxed),
                                PolishMode::from_u8(polish_mode.load(Ordering::Relaxed)),
                                vec![journal::path_for(take_id)],
                                take_training.take(),
                            );
                            let _ = work_tx.send(Work::Cancelled { id: take_id, take });
                            sent = 0;
                            continue;
                        }
                    }
                    // Too short to be anything but an accidental tap, or an
                    // unchunked take with no speech in it: fed room tone the
                    // model invents a phrase, and it would be typed (#51).
                    // A chunked take is gated chunk by chunk instead.
                    if all.len() < MIN_CLIP_SAMPLES || (sent == 0 && !has_speech(&all, 0..all.len())) {
                        tracing::info!(
                            id = take_id,
                            secs = all.len() as f64 / 16_000.0,
                            "take dropped: too short or no speech"
                        );
                        emit_state(&app, "idle");
                        all.clear();
                        sent = 0;
                        // No Take to drop, so clean its journal up here.
                        journal::remove(journal::path_for(take_id));
                        continue;
                    }
                    let mut take = Take::new(
                        std::mem::take(&mut all),
                        focus_target.swap(0, Ordering::Relaxed),
                        PolishMode::from_u8(polish_mode.load(Ordering::Relaxed)),
                        vec![journal::path_for(take_id)],
                        take_training.take(),
                    );
                    take.sent = sent;
                    take.context = take_context.take().and_then(|rx| rx.try_recv().ok());
                    sent = 0;
                    // Cancel arrived during/right after recording.
                    if cancelled.swap(false, Ordering::SeqCst) {
                        emit_state(&app, "cancelled");
                        let _ = work_tx.send(Work::Cancelled { id: take_id, take });
                        continue;
                    }
                    let _ = work_tx.send(Work::Finish { id: take_id, take });
                }
                DictationEvent::Retry => {
                    let _ = work_tx.send(Work::Retry);
                }
                DictationEvent::Dismiss => {
                    let _ = work_tx.send(Work::Dismiss);
                }
                DictationEvent::Repolish(text) => {
                    let _ = work_tx.send(Work::Repolish(text));
                }
                DictationEvent::Transform(t) => {
                    let _ = work_tx.send(Work::Transform(t));
                }
                DictationEvent::Scratchpad(a) => {
                    let _ = work_tx.send(Work::Scratchpad(a));
                }
            }
        }
    });
}

/// Work handed from the audio thread to the transcribe thread. `id` ties a
/// chunk to the take it came from, so a chunk that lands after the next take
/// has already started recording still joins the right transcript.
enum Work {
    /// `seam`: the audio starts before the previous cut (see `seam_start`),
    /// so its text repeats the end of the last chunk's for `join_text` to drop.
    Chunk { id: u64, samples: Vec<f32>, seam: bool },
    Finish { id: u64, take: Take },
    Cancelled { id: u64, take: Take },
    Retry,
    Dismiss,
    Repolish(String),
    /// A Transform chord fired (#18): rewrite the focused app's selection.
    Transform(config::Transform),
    /// The Scratchpad's chord or a row's inject (#19).
    Scratchpad(scratchpad::Action),
    /// Spin the LLM sidecar up while the user is still speaking.
    Prewarm,
    /// Pronunciation Trainer: transcribe the take-so-far mid-recording and,
    /// if `target` is in it, fire "word-detected" so the UI can ignite the
    /// glow the instant the model hears the word — not just on mic level.
    /// Non-destructive: it never touches the take or its chunk partials.
    /// `id` is the take it peeked at: only the live take's peek may fire.
    Peek { id: u64, samples: Vec<f32>, target: String },
    /// A chunk of a lecture (#23), bound for its transcript file.
    Lecture(lecture::Piece),
}

/// Chunk text accumulated for a take that's still being recorded.
enum Partial {
    Text(String),
    /// A chunk failed to transcribe, so the accumulated text has a gap in it
    /// and must be thrown away in favour of one whole-take pass.
    Poisoned,
}

/// Push the current retry-stash state to the tray menu and Home's alert card.
/// Called after anything that can change the stash, so "Retry last dictation"
/// is never offering something that isn't there.
fn publish_take(
    app: &tauri::AppHandle,
    take_info: &Arc<Mutex<Option<TakeInfo>>>,
    stash: &Option<Take>,
) {
    let info = stash.as_ref().map(TakeInfo::from);
    *take_info.lock().unwrap() = info.clone();
    let _ = app.emit("take-changed", info);
}

/// Root-mean-square level of a slice — how loud it is, 0.0 for pure silence.
fn rms(s: &[f32]) -> f32 {
    if s.is_empty() {
        return 0.0;
    }
    (s.iter().map(|v| v * v).sum::<f32>() / s.len() as f32).sqrt()
}

/// Whether `take[part]` holds any speech: at least SPEECH_MIN_FRAMES 30 ms
/// frames louder than SPEECH_OVER_FLOOR times the take's noise floor (its
/// quietest 150 ms) and than SPEECH_MIN_RMS. One test for the chunk, seam,
/// tail and short-take gates.
///
/// Counted, not averaged: a whole-chunk mean let 10 s of quiet speech pass
/// for silence (#64), and a share of frames would still drown one short word
/// in a long chunk. Relative, not a fixed bar: a clause 20 dB under the rest
/// of the take fell under the old 0.015 and was dropped after the last cut
/// (#100), while a fan above it passed for speech.
///
/// ponytail: loudness only. Steady room tone stays under the bar, but a run
/// of keyboard clatter can clear it and reach the model. The upgrade is a
/// real VAD (Silero), which needs a model file: a Conductor call. The floor
/// is the quietest stretch, not a percentile, so continuous speech is never
/// its own floor. A device that goes silent for 150 ms mid-take (a dropout,
/// past FLOOR_SKIP) still drops the bar to SPEECH_MIN_RMS for that take.
fn has_speech(take: &[f32], part: std::ops::Range<usize>) -> bool {
    // The first FLOOR_SKIP of a take don't count toward its floor: many devices
    // open on a few frames of exact zero, which on an ungated mic would drop the
    // bar to SPEECH_MIN_RMS and let room tone through. A gated mic still finds
    // its zero floor in the take's later pauses.
    let settled = take.get(FLOOR_SKIP..).filter(|s| s.len() >= SPEECH_FRAME * SPEECH_MIN_FRAMES).unwrap_or(take);
    let floor = settled.chunks_exact(SPEECH_FRAME * SPEECH_MIN_FRAMES).map(rms).reduce(f32::min).unwrap_or(0.0);
    let bar = (floor * SPEECH_OVER_FLOOR).max(SPEECH_MIN_RMS);
    take[part].chunks(SPEECH_FRAME).filter(|f| rms(f) > bar).count() >= SPEECH_MIN_FRAMES
}

/// Where the audio after a cut at `cut` starts when it's handed to the model:
/// CHUNK_OVERLAP early if there's speech in that second, so a word the cut
/// clipped is heard whole on this side (#64); right at the cut if that second
/// was a pause, with nothing to re-hear. An early start is what tells
/// `join_text` to drop the words both sides heard.
fn seam_start(all: &[f32], cut: usize) -> usize {
    let from = cut.saturating_sub(CHUNK_OVERLAP);
    if has_speech(all, from..cut) { from } else { cut }
}

/// The next chunk due out of `all` (the take so far) once `sent` samples of it
/// have gone: `(from, end, speech)`, or `None` to keep recording. The new
/// audio is `sent..end`, `from` adds the overlap, and `speech` is judged on the
/// new audio alone, since the overlap was judged with the chunk before.
fn next_chunk(all: &[f32], sent: usize) -> Option<(usize, usize, bool)> {
    let end = sent + chunk_cut(&all[sent..])?;
    let from = seam_start(all, sent);
    let speech = has_speech(all, sent..end);
    // The seams, for auditing lost words against the audio.
    tracing::debug!(from, sent, end, speech, "chunk boundary");
    Some((from, end, speech))
}

/// Where to cut the next chunk out of the not-yet-sent audio (16 kHz mono),
/// or `None` to keep accumulating.
///
/// Cuts land in a pause, never mid-word. A boundary inside a word garbles it
/// on both sides, and one mid-sentence makes the following chunk come back
/// capitalized as though a new sentence had started — so the cut point is the
/// quietest 300 ms in the search window, and if nothing quiet enough turns up
/// the chunk simply keeps growing until the hard ceiling.
///
/// Below `CHUNK_MIN_SAMPLES` nothing is ever cut, so ordinary short dictations
/// take exactly the path they took before chunking existed.
fn chunk_cut(pending: &[f32]) -> Option<usize> {
    if pending.len() < CHUNK_MIN_SAMPLES {
        return None;
    }
    let hi = pending.len().min(CHUNK_MAX_SAMPLES);
    if hi < CHUNK_MIN_SAMPLES + SILENCE_WIN {
        // Not enough room to look for a pause yet.
        return (pending.len() >= CHUNK_MAX_SAMPLES).then_some(CHUNK_MAX_SAMPLES);
    }
    let mut best_rms = f32::MAX;
    let mut best_at = CHUNK_MIN_SAMPLES;
    let mut i = CHUNK_MIN_SAMPLES;
    while i + SILENCE_WIN <= hi {
        let level = rms(&pending[i..i + SILENCE_WIN]);
        if level < best_rms {
            best_rms = level;
            best_at = i;
        }
        i += SILENCE_STEP;
    }
    // A real pause, or the ceiling forcing our hand — cut in the middle of the
    // quietest window so both sides keep a little padding.
    (best_rms < SILENCE_RMS || pending.len() >= CHUNK_MAX_SAMPLES)
        .then_some(best_at + SILENCE_WIN / 2)
}

/// One dictation attempt kept in memory for a possible Retry, its audio
/// mirrored to the crash journal until it's dropped. `raw_text` is set once
/// ASR has run, so a retry after a successful transcription skips straight to
/// polish + injection.
struct Take {
    samples: Vec<f32>,
    raw_text: Option<String>,
    focus_target: isize,
    audio_ms: u64,
    tier: String,
    /// Text already transcribed from chunks handed over while recording. The
    /// final pass transcribes only `samples[sent..]` and appends it to this.
    prefix_text: String,
    /// How much of `samples` `prefix_text` already covers. Zero means "nothing
    /// was chunked" — the whole take still needs transcribing, which is also
    /// what a Retry or a poisoned chunk resets to.
    sent: usize,
    /// Why this take wasn't delivered, in the user's words — set at whichever
    /// stash site caught it, shown verbatim by Home's alert card.
    reason: &'static str,
    /// This take's crash-journal files (see `journal.rs`).
    journal: Vec<PathBuf>,
    /// Pronunciation-Trainer word this take was recorded for. Carried by the
    /// take, not read from the live arm, so only the take the user started
    /// from the trainer is a sample, a Retry of it stays one, and every later
    /// hotkey take is a normal dictation (#47).
    training: Option<Training>,
    /// Auto tone (#28): the focused field's text before the caret when the
    /// take started, if it was read in time. Memory only: never logged,
    /// never journaled, gone when the take drops.
    context: Option<String>,
}

/// Every way a take ends (delivered, dismissed, replaced in the stash, too
/// short to keep) drops it, so this is the one place its journal is cleaned
/// up. A take still alive at a crash or quit never drops, which is exactly
/// what leaves its journal behind for the next launch to recover.
impl Drop for Take {
    fn drop(&mut self) {
        for f in self.journal.drain(..) {
            journal::remove(f);
        }
    }
}

impl From<&Take> for TakeInfo {
    fn from(t: &Take) -> Self {
        TakeInfo {
            reason: t.reason.to_string(),
            words: t.raw_text.as_deref().map_or(0, |s| s.split_whitespace().count()),
            audio_ms: t.audio_ms,
        }
    }
}

impl Take {
    /// `mode` is read from the shared atom rather than from the `Polisher`,
    /// which now lives on the transcribe thread and isn't reachable from the
    /// audio thread that builds takes.
    fn new(samples: Vec<f32>, focus_target: isize, mode: PolishMode, journal: Vec<PathBuf>, training: Option<Training>) -> Self {
        // Recorder returns 16 kHz mono, so ms = samples / 16.
        let audio_ms = samples.len() as u64 * 1000 / 16_000;
        Take {
            samples,
            raw_text: None,
            focus_target,
            audio_ms,
            tier: mode_str(mode).into(),
            reason: "",
            prefix_text: String::new(),
            sent: 0,
            journal,
            training,
            context: None,
        }
    }
}

/// Glue two transcript pieces together: the chunk text so far and the next
/// chunk's, or the tail's. Across a pause each side is already punctuated by
/// the model, so a single space is the right join — and either side being
/// empty is normal (a silent chunk, or no chunking at all).
///
/// With `seam`, `b`'s audio started before the cut (see `seam_start`), so `b`
/// opens by repeating the last words of `a`. The longest such repeat is where
/// the two splice: `a` up to it, `b` after it. Up to SEAM_SLACK words either
/// side of it go too: the halves of a word the cut clipped, which the other
/// side heard whole. A lone repeated word must sit right at the seam, as a
/// lone "the" further off is a coincidence. The joining word keeps `a`'s
/// letters (it heard what came before) and `b`'s punctuation (it heard what
/// came after), so no stray full stop or capital is left mid-sentence.
///
/// ponytail: text matching, no word timestamps. If both sides mishear the
/// overlap differently, nothing matches and its words come out twice.
fn join_text(a: &str, b: &str, seam: bool) -> String {
    let (aw, bw): (Vec<&str>, Vec<&str>) =
        (a.split_whitespace().collect(), b.split_whitespace().collect());
    if let Some((i, j, k)) = seam_repeat(&aw, &bw).filter(|_| seam) {
        let bare = |w: &str| w.trim_end_matches(|c: char| !c.is_alphanumeric()).len();
        let (last_a, last_b) = (aw[i + k - 1], bw[j + k - 1]);
        let joint = format!("{}{}", &last_a[..bare(last_a)], &last_b[bare(last_b)..]);
        return aw[..i + k - 1]
            .iter()
            .copied()
            .chain([joint.as_str()])
            .chain(bw[j + k..].iter().copied())
            .collect::<Vec<_>>()
            .join(" ");
    }
    match (a.trim(), b.trim()) {
        ("", b) => b.to_string(),
        (a, "") => a.to_string(),
        (a, b) => format!("{a} {b}"),
    }
}

/// The repeat `join_text` splices on: `(i, j, k)`, where `aw[i..i+k]` comes
/// again as `bw[j..j+k]`. Its own function so a lecture line can drop the
/// same words (`lecture::fresh`).
fn seam_repeat(aw: &[&str], bw: &[&str]) -> Option<(usize, usize, usize)> {
    // (i, j, k, dropped): `dropped` words of `a` follow the repeat.
    (aw.len().saturating_sub(SEAM_WORDS)..aw.len())
        .flat_map(|i| (0..=SEAM_SLACK).map(move |j| (i, j)))
        .map(|(i, j)| {
            let k = (0..)
                .take_while(|&k| i + k < aw.len() && j + k < bw.len() && asr::same_word(aw[i + k], bw[j + k]))
                .count();
            (i, j, k, aw.len() - i - k)
        })
        .filter(|&(_, j, k, dropped)| dropped <= SEAM_SLACK && (k >= 2 || (k == 1 && j + dropped <= 1)))
        .max_by_key(|&(_, j, k, dropped)| (k, std::cmp::Reverse(j + dropped)))
        .map(|(i, j, k, _)| (i, j, k))
}

fn mode_str(m: PolishMode) -> &'static str {
    match m {
        PolishMode::Off => "off",
        PolishMode::Rules => "rules",
        PolishMode::Ai => "ai",
    }
}

/// The config as it may appear in the log (#48): dictionary/snippet entries
/// and trained vocabulary are the user's own words (emails, phone numbers,
/// names), so they're dropped; `main` logs their counts instead. Everything
/// else is settings, safe to show.
fn config_for_log(cfg: &Config) -> Config {
    let mut safe = cfg.clone();
    safe.dictionary.clear();
    safe.polish.vocabulary.clear();
    // A custom transform's instruction is the user's own text.
    safe.transforms.iter_mut().for_each(|t| t.prompt.clear());
    safe
}

/// True if `text` says `target`: its words appear in `text` in a row, whole
/// words only, ignoring case and the punctuation around each word. The
/// trainer's one match rule (#49): the peek's glow and the sample's
/// hit/miss both ask this, so "Claude." can't light "Got it" and then count
/// as a miss.
fn heard_word(text: &str, target: &str) -> bool {
    let words = |s: &str| -> Vec<String> {
        s.split_whitespace()
            .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase())
            .filter(|w| !w.is_empty())
            .collect()
    };
    let (text, target) = (words(text), words(target));
    !target.is_empty() && text.windows(target.len()).any(|w| w == target)
}

/// What a trainer take is for: the word typed on the Pronunciation page, or
/// a whole Calibrate sentence (#22). Both come back as a
/// "pronunciation-sample"; only a word is scored.
#[derive(Clone)]
pub struct Training {
    target: String,
    calibration: bool,
}

/// Whether a peek's transcript should ignite the glow: the word was heard
/// AND the peek belongs to the take being recorded now. A peek still in
/// flight when the next take starts would otherwise glow and auto-stop the
/// new take before its word is said (#39).
fn peek_fires(text: &str, target: &str, peek_take: u64, live_take: u64) -> bool {
    peek_take == live_take && heard_word(text, target)
}

/// What a Start does with the trainer arm (#47): it always consumes it, so
/// the arm can never outlive one Start and swallow later hotkey takes
/// however the trainer take ends (sample, error, cancel, too short, or a
/// UI that never sends `None`). The new take becomes a trainer take only if
/// it really starts; a Start ignored because a take is already recording
/// leaves that take a normal dictation.
fn arm_take<T>(armed: &mut Option<T>, recording: bool) -> Option<T> {
    let word = armed.take();
    if recording { None } else { word }
}

/// Whether a take's audio is crash-journaled (#44). Not while a
/// Pronunciation-Trainer word is armed: a recovered trainer take would come
/// back as a normal dictation, and Retry would inject the training word into
/// whatever app has focus. A practice sample has nothing worth recovering.
/// Decided once at Start so a take is journaled whole or not at all.
fn journals_take(training_armed: bool) -> bool {
    !training_armed
}

/// Auto-send (#21): press Enter after this dictation? Only into an app on the
/// list, by exact name (no wildcard: never global), and only once the whole
/// text `landed` in the window it was spoken into. An Enter after a failed,
/// partial or misdirected injection would send something else. Only
/// `process_take`'s delivery asks: a voice correction or a Transform edits
/// text already there and never sends it.
fn auto_sends(apps: &[String], app_name: &str, landed: bool) -> bool {
    landed && !app_name.is_empty() && apps.iter().any(|a| a.eq_ignore_ascii_case(app_name))
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
    listening: &Arc<AtomicBool>,
    injection_mode: InjectionMode,
    stats_enabled: &Arc<AtomicBool>,
    retention_enabled: &Arc<AtomicBool>,
    mut take: Take,
    stash: &mut Option<Take>,
) {
    // Overlay states from this pipeline are suppressed while a *new* take is
    // being recorded: now that takes can overlap, a background one finishing
    // must not yank the pill off the live "listening" state and flash "done"
    // at someone who is still mid-sentence. Delivery itself is unaffected.
    let emit_state = |app: &tauri::AppHandle, s: &str| {
        if !listening.load(Ordering::Relaxed) {
            emit_state(app, s);
        }
    };
    // Stage 1 — transcription (skipped on a retry that already has text).
    if take.raw_text.is_none() {
        emit_state(app, "transcribing");
        // Only the tail: `sent` bytes were already transcribed as chunks while
        // the user was speaking, and `prefix_text` holds that result. `sent` is
        // 0 whenever chunking didn't happen or can't be trusted, which makes
        // this the original whole-take call. Like a chunk, the tail re-hears
        // the second before the last cut (`seam_start`).
        let sent = take.sent.min(take.samples.len());
        let from = seam_start(&take.samples, sent);
        // Releasing the key just after a chunk cut leaves a tail with nothing
        // in it. Handed that, the model invents a word to tack onto the end,
        // so a tail with no speech is treated as the silence it is. It's
        // judged against the whole take's floor, so a softer last clause
        // still counts (#100).
        let tail_text = if sent > 0 && !has_speech(&take.samples, sent..take.samples.len()) {
            Ok(String::new())
        } else {
            asr.transcribe(&take.samples[from..])
        };
        match tail_text {
            Ok(t) => take.raw_text = Some(join_text(&take.prefix_text, &t, from < sent)),
            Err(err) => {
                tracing::error!(?err, "transcription failed");
                // Drop the chunked prefix so a Retry re-transcribes cleanly
                // rather than gluing new text onto a stale half-transcript.
                take.prefix_text.clear();
                take.sent = 0;
                // Distinguish "the model isn't on disk yet" (first-run
                // download still in flight) from a real failure — the take is
                // stashed either way, so ↻ works once the download lands.
                let model_missing = !config::asr_model_present(asr.engine());
                emit_state(app, if model_missing { "nomodel" } else { "error" });
                record_outcome(&take, stats_enabled, "error");
                take.reason = if model_missing { "Speech model still downloading" } else { "Couldn't transcribe it" };
                *stash = Some(take);
                return;
            }
        }
    }
    if cancelled.swap(false, Ordering::SeqCst) {
        emit_state(app, "cancelled");
        record_outcome(&take, stats_enabled, "cancelled");
        take.reason = "Cancelled";
        *stash = Some(take);
        return;
    }
    let raw = take.raw_text.clone().unwrap_or_default();
    if raw.trim().is_empty() {
        emit_state(app, "error");
        record_outcome(&take, stats_enabled, "error");
        take.reason = "Didn't catch any speech";
        *stash = Some(take);
        return;
    }

    // Pronunciation Trainer short-circuit: a training utterance is never
    // meant to land in a real document, so it skips polish/injection/
    // history/stats entirely and just reports what the ASR engine actually
    // heard, raw, compared against the target word. ponytail: a training
    // attempt that comes back empty ("Didn't catch any speech", above)
    // still falls through as a normal failed take rather than reporting
    // into the panel too — the user just tries again; not worth a second
    // short-circuit point for a case the overlay's error state already
    // covers.
    if let Some(Training { target, calibration }) = take.training.clone() {
        let heard = raw.trim().to_string();
        let matched = heard_word(&heard, &target);
        // Persist this attempt into the word's rolling success history —
        // regardless of whether the user goes on to log a correction for a
        // miss, "did this land" is its own signal. Mirrors
        // `add_pronunciation_correction`'s save-then-refresh-the-live-mirror
        // pattern, just reached via AppHandle since this runs on the worker
        // thread rather than inside a #[tauri::command]. A calibration
        // sentence isn't a word to score, and it saves nothing until the
        // user confirms a correction (#22).
        if !calibration {
            let app_state = app.state::<AppState>();
            let mut cfg = app_state.cfg.lock().unwrap();
            config::record_attempt(&mut cfg.polish.vocabulary, &target, matched);
            *app_state.controls.vocabulary.lock().unwrap() = cfg.polish.vocabulary.clone();
            let _ = cfg.save();
        }
        let _ = app.emit("pronunciation-sample", PronunciationSampleDto { word: target, heard, matched });
        emit_state(app, "done");
        *stash = None;
        return;
    }

    // Stage 2 — polish. Resolve the focused app once, here — it's the tone
    // lookup key below and the stats line further down, so one syscall covers
    // both instead of querying Windows twice.
    let app_name = stats::app_name(take.focus_target);
    // Voice correction (#20): "correction: Claude, not clawed" fixes the last
    // dictation instead of being typed. Not a dictation itself: no history,
    // no stats.
    let voice_correction = app.state::<AppState>().cfg.lock().unwrap().voice_correction;
    if let Some((right, wrong)) = correction::parse(&raw).filter(|_| voice_correction) {
        run_correction(app, &right, &wrong, take.focus_target, &app_name, suppressed, listening, injection_mode);
        return;
    }
    // Backtrack (#26): "scratch that" deletes the last dictation instead of
    // being typed. Not a dictation either: no history, no stats.
    let backtrack = app.state::<AppState>().cfg.lock().unwrap().backtrack;
    if backtrack && correction::is_backtrack(&raw) {
        run_backtrack(app, take.focus_target, &app_name, suppressed, listening, injection_mode);
        return;
    }
    if polisher.uses_ai_tier(&raw) {
        emit_state(app, "polishing");
    }
    let result = polisher.polish_for(&raw, &app_name, take.context.as_deref());
    take.tier = result.tier.into(); // the tier that ran, not the configured one (#67)
    if cancelled.swap(false, Ordering::SeqCst) {
        emit_state(app, "cancelled");
        record_outcome(&take, stats_enabled, "cancelled");
        take.reason = "Cancelled";
        *stash = Some(take);
        return;
    }
    if result.text.is_empty() {
        emit_state(app, "error");
        record_outcome(&take, stats_enabled, "error");
        take.reason = "Polish came back empty";
        *stash = Some(take);
        return;
    }

    // Heard-vs-typed trail, at debug only (#48): the log is what users attach
    // to public bug reports, and the README promises it never holds dictated
    // text. `SOTTO_LOG=debug` brings it back for a diagnosis session; opt-in
    // retention keeps the same pair in recordings/index.jsonl.
    tracing::debug!(raw = %raw, polished = %result.text, "transcript");
    tracing::info!(raw_chars = raw.chars().count(), chars = result.text.chars().count(), "transcript");

    // Scratchpad (#19): a take spoken into the pad is kept there, not typed.
    // No history, stats or clipboard: the pad is its own private list.
    if scratchpad::catches(app, take.focus_target) {
        let saved = scratchpad::land(app, &result.text);
        emit_state(app, if saved { "done" } else { "error" });
        if !saved {
            take.reason = "Couldn't save it to the Scratchpad";
            *stash = Some(take);
        } else if !stash.as_ref().is_some_and(|t| t.reason == RECOVERED) {
            *stash = None;
        }
        return;
    }

    // Stage 3 — inject into the original window.
    inject::restore_focus(take.focus_target);
    suppressed.store(true, Ordering::SeqCst);
    let injected = inject::inject_text(&result.text, injection_mode);
    // Auto-send (#21). The settle lets the app take the text in first (a
    // terminal's paste is async), and a modifier still down would make the
    // Enter a Ctrl+Enter or Shift+Enter. ponytail: one fixed settle; make it
    // per app if one ever needs longer.
    let landed = injected.is_ok() && inject::capture_focus() == take.focus_target;
    let auto_send = auto_sends(&app.state::<AppState>().cfg.lock().unwrap().auto_send, &app_name, landed);
    let auto_sent = auto_send && {
        std::thread::sleep(Duration::from_millis(100));
        inject::wait_for_modifiers_released(Duration::from_millis(500)) && inject::press_enter().is_ok()
    };
    suppressed.store(false, Ordering::SeqCst);
    match injected {
        Ok(()) => {
            // Leave the text on the clipboard as a mis-focus safety net.
            if let Ok(mut cb) = arboard::Clipboard::new() {
                let _ = cb.set_text(result.text.clone());
            }
            correction::remember(&result.text, take.focus_target);
            history.push(raw.clone(), result.text.clone(), result.tier, result.fallback);
            emit_state(app, "done");
            // "Something smart just happened": when a phonetic correction fired
            // this take, the overlay swaps its plain done checkmark for a flyout
            // showing "heard -> corrected". Emitted right after the "done" state
            // (so the Rust-side overlay_state the hit-test reads stays "done" —
            // non-clickable — while overlay.js shows the flyout visually), and
            // gated on !listening for the same reason as emit_state: a
            // background take finishing must not interrupt a live recording.
            if !listening.load(Ordering::Relaxed) && !result.notable.is_empty() {
                let (heard, corrected) = result.notable[0].clone();
                let _ = app.emit(
                    "overlay-flyout",
                    FlyoutDto { heard, corrected, more: result.notable.len().saturating_sub(1) },
                );
            }
            emit_history(app, history);
            if retention_enabled.load(Ordering::Relaxed) {
                // ponytail: synchronous write, right here on the worker
                // thread — it lands after injection, so the user already
                // has their text, and a mu-law WAV of a short take is a
                // sub-millisecond write on any real disk. Move this to a
                // channel if a very long take ever measurably stalls the
                // next one.
                let max_mb = app.state::<AppState>().cfg.lock().unwrap().retention.max_mb;
                recordings::record(
                    &take.samples,
                    &raw,
                    &result.text,
                    &app_name,
                    asr.engine(),
                    &take.tier,
                    take.audio_ms,
                    max_mb,
                    result.fallback,
                );
            }
            if stats_enabled.load(Ordering::Relaxed) {
                let words = result.text.split_whitespace().count();
                stats::record(&stats::entry_now(
                    words,
                    take.audio_ms,
                    app_name.clone(),
                    &take.tier,
                    result.corrected_words,
                    result.dict_hits,
                    "injected",
                ));
            }
            // Delivered — nothing to retry. Except a take recovered after a
            // restart: Sotto usually starts hidden, so nobody may have seen
            // its card yet, and it can't be redone by just speaking again.
            if !stash.as_ref().is_some_and(|t| t.reason == RECOVERED) {
                *stash = None;
            }
            tracing::info!(chars = result.text.chars().count(), auto_sent, "injected");
        }
        Err(err) => {
            tracing::error!(?err, "injection failed");
            emit_state(app, "error");
            record_outcome(&take, stats_enabled, "error");
            take.reason = "Couldn't paste into that app";
            *stash = Some(take);
        }
    }
}

/// A Transform chord (#18): rewrite the focused app's selection in place. The
/// order and the clipboard restore are `transform::run`'s; this feeds it the
/// real clipboard, keys and model, and puts the outcome on the pill.
fn run_transform(
    app: &tauri::AppHandle,
    polisher: &polish::Polisher,
    suppressed: &Arc<AtomicBool>,
    cancelled: &Arc<AtomicBool>,
    listening: &Arc<AtomicBool>,
    injection_mode: InjectionMode,
    t: &config::Transform,
) {
    // Same rule as process_take: never pull the pill off a live take.
    let emit_state = |s: &str| {
        if !listening.load(Ordering::Relaxed) {
            emit_state(app, s);
        }
    };
    if inject::foreground_is_ours() {
        return; // pressed in Sotto's own window, e.g. while setting a chord
    }
    let Some(llm) = polisher.llm() else {
        tracing::warn!(name = %t.name, "transform: the AI model isn't installed");
        emit_state("kept");
        return;
    };
    let mut clip = match transform::SystemClipboard::new() {
        Ok(c) => c,
        Err(err) => {
            tracing::error!(?err, "transform: clipboard unavailable");
            emit_state("kept");
            return;
        }
    };
    cancelled.store(false, Ordering::SeqCst); // only an Escape from here on counts
    // The model takes seconds: paste back where the text came from even if the
    // user Alt-Tabbed meanwhile, same as a take's focus target.
    let target = inject::capture_focus();
    let t0 = Instant::now();
    let outcome = transform::run(
        &mut clip,
        || {
            // The chord's own Ctrl/Alt may still be down, and a Ctrl+C under
            // a held Alt is Ctrl+Alt+C. Waited out before suppressing, so the
            // listener still sees the user's own key releases.
            anyhow::ensure!(inject::wait_for_modifiers_released(Duration::from_secs(2)), "chord keys still held");
            suppressed.store(true, Ordering::SeqCst);
            let sent = inject::send_ctrl_c();
            std::thread::sleep(Duration::from_millis(30)); // our own keys pass the hook while suppressed
            suppressed.store(false, Ordering::SeqCst);
            sent
        },
        |text| {
            emit_state("polishing");
            let out = llm
                .rewrite(text, &t.prompt)
                .map_err(|err| tracing::warn!(error = %err, "transform: rewrite failed"))
                .ok()?;
            if cancelled.swap(false, Ordering::SeqCst) {
                return None; // Escape while the model ran
            }
            transform::accept(text, &out, t.keep_words)
        },
        |out| {
            inject::restore_focus(target);
            suppressed.store(true, Ordering::SeqCst);
            let pasted = inject::inject_text(out, injection_mode);
            suppressed.store(false, Ordering::SeqCst);
            pasted
        },
    );
    // Counts only: the selection is the user's text, same rule as the transcript (#48).
    tracing::info!(name = %t.name, ?outcome, ms = t0.elapsed().as_millis(), "transform");
    emit_state(match outcome {
        transform::Outcome::Replaced => "done",
        transform::Outcome::NothingSelected => "noselection",
        transform::Outcome::Kept => "kept",
    });
}

/// Voice correction (#20): swap `wrong` for `right` in the last dictation,
/// teach the pair, and type the fixed text back over the old one. The retype
/// needs the command to come from the window that dictation went to, and a
/// copy of the re-selected span to prove it's still there, unchanged. Any
/// other case leaves the document alone and puts the fix on the clipboard.
#[allow(clippy::too_many_arguments)]
fn run_correction(
    app: &tauri::AppHandle,
    right: &str,
    wrong: &str,
    focus: isize,
    app_name: &str,
    suppressed: &Arc<AtomicBool>,
    listening: &Arc<AtomicBool>,
    injection_mode: InjectionMode,
) {
    // Same rule as process_take: never pull the pill off a live take.
    let quiet = || listening.load(Ordering::Relaxed);
    let found = correction::last()
        .and_then(|(old, hwnd)| Some((correction::replace_word(&old, wrong, right)?, old, hwnd)));
    let Some((fixed, old, hwnd)) = found else {
        // Counts and flags only, here and below: the words are the user's (#48).
        tracing::info!("voice correction: the word isn't in the last dictation");
        if !quiet() {
            let _ = app.emit("overlay-note", wrong);
            emit_state(app, "notfound");
        }
        return;
    };
    let taught = correction::harvests(right, wrong);
    if taught {
        add_pronunciation_correction(right.to_string(), wrong.to_string(), app.state::<AppState>());
    }
    let retyped =
        hwnd == focus && !correction::is_terminal(app_name) && retype(&old, &fixed, hwnd, suppressed, injection_mode);
    tracing::info!(retyped, taught, chars = fixed.chars().count(), "voice correction");
    // The fix is now the last dictation (a second correction builds on it)
    // and the clipboard's safety net, as after any delivery.
    correction::remember(&fixed, hwnd);
    copy_text(fixed);
    if quiet() {
        return;
    }
    if retyped {
        emit_state(app, "done");
        let _ = app.emit("overlay-flyout", FlyoutDto { heard: wrong.to_string(), corrected: right.to_string(), more: 0 });
    } else {
        emit_state(app, "fixcopied");
    }
}

/// Backtrack (#26): delete the last dictation, under a voice correction's
/// guard: said in the window it went to, not a terminal, and a copy of the
/// re-selected span proves it's still there, unchanged. Anything else leaves
/// the document alone. One level: a deleted dictation is forgotten, so a
/// second "scratch that" has nothing to undo.
fn run_backtrack(
    app: &tauri::AppHandle,
    focus: isize,
    app_name: &str,
    suppressed: &Arc<AtomicBool>,
    listening: &Arc<AtomicBool>,
    injection_mode: InjectionMode,
) {
    let state = match correction::undo_target(correction::last(), focus, app_name) {
        Ok(old) if retype(&old, "", focus, suppressed, injection_mode) => {
            correction::forget();
            "done"
        }
        Ok(_) => "notundone",
        Err(state) => state,
    };
    // The outcome only: the words are the user's (#48).
    tracing::info!(state, "backtrack");
    // Same rule as process_take: never pull the pill off a live take.
    if !listening.load(Ordering::Relaxed) {
        emit_state(app, state);
    }
}

/// Re-select the last dictation (Shift+Left × its length), copy it with
/// Ctrl+Insert, and type `fixed` over it only if the copy is that dictation.
/// `transform::run` owns the clipboard around the copy.
fn retype(old: &str, fixed: &str, hwnd: isize, suppressed: &Arc<AtomicBool>, injection_mode: InjectionMode) -> bool {
    let Ok(mut clip) = transform::SystemClipboard::new() else {
        return false;
    };
    inject::restore_focus(hwnd);
    // A modifier still down turns Shift+Left into Ctrl+Shift+Left (a word).
    // Waited out before suppressing, so the listener sees the user's releases.
    if !inject::wait_for_modifiers_released(Duration::from_secs(2)) {
        return false;
    }
    let n = old.chars().count();
    suppressed.store(true, Ordering::SeqCst);
    let outcome = transform::run(
        &mut clip,
        || inject::select_back(n).and_then(|()| inject::send_ctrl_insert()),
        |copied| correction::same_text(copied, old).then(|| fixed.to_string()),
        // An empty `fixed` is a backtrack (#26): delete the selection.
        |out| if out.is_empty() { inject::press_backspace() } else { inject::inject_text(out, injection_mode) },
    );
    // Leave the caret where it was.
    let _ = inject::press_right(correction::caret_back(&outcome));
    std::thread::sleep(Duration::from_millis(30)); // our own keys pass the hook while suppressed
    suppressed.store(false, Ordering::SeqCst);
    outcome == transform::Outcome::Replaced
}

fn emit_state(app: &tauri::AppHandle, s: &str) {
    let _ = app.emit("overlay-state", s);
    *app.state::<AppState>().controls.overlay_state.lock().unwrap() = s.to_string();
    set_tray_icon(app, s == "listening");
    let always_visible = app.state::<AppState>().controls.overlay_always_visible.load(Ordering::Relaxed);
    if s != "idle" || always_visible {
        show_overlay(app);
    }
}

/// The tray tile is lilac while the microphone is open: a take being
/// recorded, or a lecture for as long as it runs (#23), whatever the pill
/// shows meanwhile.
fn set_tray_icon(app: &tauri::AppHandle, listening: bool) {
    let state = app.state::<AppState>();
    if let Some(tray) = app.tray_by_id("main") {
        let dark = tray::dark_for(&state.cfg.lock().unwrap().theme);
        let _ = tray.set_icon(Some(tray::icon(listening || state.controls.lecture.load(Ordering::Relaxed), dark)));
    }
}

/// Lecture capture went on or off (#23): the live flag, the tray tile, and
/// the windows that show it (Home's card, the tray menu).
fn publish_lecture(app: &tauri::AppHandle, on: bool) {
    app.state::<AppState>().controls.lecture.store(on, Ordering::Relaxed);
    set_tray_icon(app, false);
    let _ = app.emit("lecture-changed", on);
}

/// Makes the overlay's ✕/↻ buttons clickable without the invisible window
/// margins eating clicks: the window stays click-through except while the
/// cursor is actually inside the pill's rectangle. A 30 ms cursor poll is the
/// only way to do this — mouse events can't reach the webview while
/// click-through is on, so JS can't hit-test for us.
fn spawn_overlay_hittest(
    app: tauri::AppHandle,
    ui_state: Arc<Mutex<String>>,
    always_visible: Arc<AtomicBool>,
    position: Arc<Mutex<String>>,
) {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    std::thread::spawn(move || {
        let mut ignoring = true; // window starts click-through
        // Eased auto-hide-taskbar clearance, in physical px — see
        // `taskbar_intrusion_px`. Persists across ticks so the nudge
        // animates instead of snapping.
        let mut taskbar_clear = 0.0_f64;
        loop {
            // Pill width per state — must mirror pillWidthFor() in overlay.js.
            // idle only gets a hitbox in always-visible mode, where the whole
            // pill body is the click target (start a dictation) rather than a
            // button.
            // (width, height) per state. Idle is the tucked pill, whose HOVER
            // size (62x20) is the target — not its resting 46x16. It grows the
            // instant the cursor arrives, so the hover box is what's actually
            // under the pointer; using the smaller rect would hand the cursor
            // back to the desktop mid-grow and make the pill flicker.
            let pill = match ui_state.lock().unwrap().as_str() {
                "error" => Some((236.0, 40.0)),
                "cancelled" => Some((220.0, 40.0)),
                "nomodel" => Some((248.0, 40.0)),
                "listening" | "transcribing" | "polishing" => Some((148.0, 40.0)),
                "idle" if always_visible.load(Ordering::Relaxed) => Some((62.0, 20.0)),
                _ => None, // idle (default) / done: no buttons
            };
            let Some((pill_w, pill_h)) = pill else {
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
                // Auto-hide taskbar clearance: only a bottom anchor can ever
                // sit inside a bottom-docked bar's band, so this is a no-op
                // for every other anchor. Eases toward the target over a few
                // ticks so the bar sliding in doesn't yank the pill.
                let anchor_now = position.lock().unwrap().clone();
                if anchor_now.starts_with("bottom") {
                    let intrusion = taskbar_intrusion_px();
                    taskbar_clear += (intrusion - taskbar_clear) * 0.4;
                    if let Some((bx, by)) = overlay_target_xy(&w, &anchor_now) {
                        let target_y = by - taskbar_clear.round() as i32;
                        let needs_move =
                            w.outer_position().map(|p| p.y != target_y).unwrap_or(false);
                        if needs_move {
                            let _ = w.set_position(tauri::PhysicalPosition::new(bx, target_y));
                        }
                    }
                } else {
                    taskbar_clear = 0.0;
                }

                let inside = (|| {
                    let pos = w.outer_position().ok()?;
                    let size = w.outer_size().ok()?;
                    let scale = w.scale_factor().ok()?;
                    let mut pt = POINT::default();
                    unsafe { GetCursorPos(&mut pt).ok()? };
                    // Mirror pillOrigin() in overlay.js: the pill is anchored to
                    // the same screen edge as the window, NOT centred in it, so
                    // the clickable rect has to be derived the same way or it
                    // sits ~60px away from the pill you can actually see.
                    let anchor = position.lock().unwrap().clone();
                    let (cw, ch) = (size.width as f64 / scale, size.height as f64 / scale);
                    let (ox, oy) = overlay_pill_origin(cw, ch, pill_w, pill_h, &anchor);
                    let left = pos.x as f64 + ox * scale;
                    let top = pos.y as f64 + oy * scale;
                    let (pw, ph) = (pill_w * scale, pill_h * scale);
                    let pad = 4.0 * scale;
                    Some(
                        (pt.x as f64) >= left - pad
                            && (pt.x as f64) <= left + pw + pad
                            && (pt.y as f64) >= top - pad
                            && (pt.y as f64) <= top + ph + pad,
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
        let anchor = app.state::<AppState>().controls.overlay_position.lock().unwrap().clone();
        position_overlay(&w, &anchor);
        harden_utility_window(&w, true);
        let _ = w.show();
        harden_utility_window(&w, true);
    }
}

/// Keep a borderless utility popup (the overlay pill / tray menu) out of
/// Alt-Tab and the taskbar's own preview surfaces.
///
/// `set_skip_taskbar` (below) only removes the taskbar *button* — it's
/// `ITaskbarList::DeleteTab` under the hood and has no bearing on Alt-Tab
/// eligibility at all. `WS_EX_TOOLWINDOW` is the actual Alt-Tab exclusion,
/// and it's a create-time style: Windows decides Alt-Tab membership when a
/// window is first shown, so this must run *before* `w.show()` too, not only
/// after.
///
/// It must ALSO run again immediately *after* `w.show()` for the overlay
/// (transparent + undecorated) — confirmed live via a debug readback:
/// `show()` itself resets GWL_EXSTYLE back to Tao's default
/// (TOPMOST|ACCEPTFILES|TRANSPARENT|WINDOWEDGE|APPWINDOW|LAYERED, no
/// TOOLWINDOW/NOACTIVATE) as part of making a layered window visible, wiping
/// out whatever this function set moments earlier. A single pre-show call
/// alone silently does nothing by the time the user can interact with the
/// window — this was the actual cause of the Win+Up-maximizes-the-pill bug,
/// not `spawn_overlay_hittest`'s live cursor-event toggling (that was the
/// first, wrong suspect — checked and ruled out via the same readback,
/// before any hover ever occurred).
///
/// `no_activate` additionally sets `WS_EX_NOACTIVATE` — for the overlay
/// only, never the tray menu (which calls `set_focus()` right after this to
/// support click-outside-to-dismiss). Without it, clicking the always-visible
/// idle pill's clickable region (N1) makes Windows treat the overlay as the
/// active window; a subsequent Win+Up then hits it with the OS's native
/// maximize-the-active-window shortcut, blowing a 280x120 borderless pill up
/// to full screen. `WS_EX_NOACTIVATE` still lets clicks land on the webview —
/// it only stops the window from ever becoming "active" — so N1 keeps
/// working exactly as before.
fn harden_utility_window(w: &tauri::WebviewWindow, no_activate: bool) {
    let _ = w.set_skip_taskbar(true);
    if let Ok(raw) = w.hwnd() {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{
            GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
        };
        // Tauri's `hwnd()` returns its own `windows`-crate HWND, pinned to a
        // different version than the one we depend on directly — same Win32
        // handle, incompatible Rust type. Round-trip through the raw pointer.
        let hwnd = HWND(raw.0 as _);
        unsafe {
            let mut style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            style = (style | WS_EX_TOOLWINDOW.0 as isize) & !(WS_EX_APPWINDOW.0 as isize);
            if no_activate {
                style |= WS_EX_NOACTIVATE.0 as isize;
            }
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style);
        }
    }
}

fn emit_history(app: &tauri::AppHandle, history: &history::History) {
    let dto: Vec<HistoryDto> = history.snapshot().into_iter().map(HistoryDto::from).collect();
    let _ = app.emit("history-updated", dto);
}

/// Size on disk in MB (whole number): sum of files if `path` is a directory
/// (Parakeet's ONNX parts), or the file's own length if it's a single file
/// (Whisper's one .bin) — `read_dir` on a file just errors, which used to
/// silently report a blank size for Whisper instead of falling back to this.
/// None if `path` doesn't exist (not downloaded yet) or is unreadable.
fn dir_size_mb(path: PathBuf) -> Option<u64> {
    let meta = std::fs::metadata(&path).ok()?;
    if meta.is_file() {
        return Some(meta.len() / 1_000_000);
    }
    let mut total = 0u64;
    for entry in std::fs::read_dir(path).ok()?.flatten() {
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
    // `--app "VS Code"` polishes as if dictated into that app (per-app tone,
    // variable recognition #27).
    let out = match arg_value("--app") {
        Some(app) => polisher.polish_for(raw, &app, None),
        None => polisher.polish(raw),
    };
    println!("raw      => {raw:?}");
    println!(
        "polished => {:?}  ({} ms, {} words corrected, {} dict fixes)",
        out.text, t.elapsed().as_millis(), out.corrected_words, out.dict_hits
    );
    Ok(())
}

/// `--replay-flags [path]` (#8): every ⚑ flag's raw transcript through the
/// current polish chain and config, diffed against the last run; see
/// `bug_reports::replay`. Rules tier, or AI for a flag taken in AI mode.
/// Exits non-zero when any output changed, so it works as a regression check.
fn run_replay_flags(path: Option<String>) -> anyhow::Result<()> {
    let cfg = Config::load_or_init().unwrap_or_default();
    let controls = Controls::from_config(&cfg);
    let polisher = polish::Polisher::new(controls.clone(), cfg.llm.clone());
    let path = path.map(PathBuf::from).unwrap_or_else(bug_reports::path);
    let changed = bug_reports::replay(&path, |f| {
        let mode = if f.polish_mode == "ai" { PolishMode::Ai } else { PolishMode::Rules };
        controls.polish_mode.store(mode.as_u8(), Ordering::Relaxed);
        polisher.polish(&f.raw).text
    })?;
    anyhow::ensure!(changed == 0, "{changed} flag(s) changed since the last replay");
    Ok(())
}

fn run_transcribe_once(path: &str) -> anyhow::Result<()> {
    let samples = transcribe_rs::audio::read_wav_samples(std::path::Path::new(path))
        .map_err(|e| anyhow::anyhow!("failed to read {path}: {e}"))?;
    let mut asr = asr::Asr::new();
    let t = Instant::now();
    let text = asr.transcribe(&samples)?;
    println!("({} samples, {} ms) => {text:?}", samples.len(), t.elapsed().as_millis());
    // A clip long enough to chunk is also run the way a live take is: fed in
    // CHUNK_POLL at a time, cut and joined by the same helpers, tail last. A
    // seam that loses words shows up as a diff against the line above.
    if samples.len() >= CHUNK_MIN_SAMPLES {
        let poll = CHUNK_POLL.as_millis() as usize * 16;
        let (mut sent, mut text) = (0, String::new());
        for have in (poll..samples.len()).step_by(poll) {
            if let Some((from, end, speech)) = next_chunk(&samples[..have], sent) {
                if speech {
                    text = join_text(&text, &asr.transcribe(&samples[from..end])?, from < sent);
                }
                sent = end;
            }
        }
        let from = seam_start(&samples, sent);
        if sent == 0 || has_speech(&samples, sent..samples.len()) {
            text = join_text(&text, &asr.transcribe(&samples[from..])?, from < sent);
        }
        println!("chunked => {text:?}");
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn take(raw: Option<&str>, audio_ms: u64, reason: &'static str) -> Take {
        Take {
            samples: vec![],
            raw_text: raw.map(String::from),
            focus_target: 0,
            audio_ms,
            tier: "off".into(),
            reason,
            prefix_text: String::new(),
            sent: 0,
            journal: vec![],
            training: None,
            context: None,
        }
    }

    #[test]
    fn logged_config_holds_no_dictionary_or_vocabulary_text() {
        let mut cfg = Config::default();
        cfg.dictionary.push(crate::config::DictEntry {
            spoken: "my work email".into(),
            replacement: "private.person@example.com".into(),
            aliases: vec!["office address".into()],
            enabled: true,
            kind: Some(crate::config::EntryKind::Snippet),
        });
        cfg.polish.vocabulary.push(crate::config::VocabEntry {
            word: "Zyxwidget".into(),
            heard_as: vec!["zicks widget".into()],
            recent: vec![],
        });
        cfg.transforms.push(crate::config::Transform {
            name: "Reply".into(),
            chord: "Ctrl+Alt+Digit3".into(),
            prompt: "Answer as Zyxperson from Zyxcorp".into(),
            keep_words: false,
        });
        let logged = format!("{:?}", config_for_log(&cfg));
        for secret in ["private.person@example.com", "my work email", "office address", "Zyxwidget", "zicks widget", "Zyxperson"] {
            assert!(!logged.contains(secret), "{secret:?} leaked into the logged config");
        }
        assert!(logged.contains("hotkey"), "settings still logged");
    }

    #[test]
    fn stale_peek_never_fires() {
        // Live take's peek that heard the word: glow.
        assert!(peek_fires("say sotto now", "Sotto", 4, 4));
        // Last take's peek landing during the next take: dropped, even
        // though it heard the word.
        assert!(!peek_fires("say sotto now", "Sotto", 3, 4));
        // Live take, word not heard: nothing.
        assert!(!peek_fires("say nothing", "Sotto", 4, 4));
    }

    #[test]
    fn a_start_always_consumes_the_trainer_arm() {
        let mut armed = Some("Sotto".to_string());
        assert_eq!(arm_take(&mut armed, false).as_deref(), Some("Sotto"), "the trainer's own take");
        assert_eq!(armed, None, "disarmed once consumed");
        assert_eq!(arm_take(&mut armed, false), None, "the next hotkey take is a normal dictation");

        // Armed while a take is already recording: that take stays normal,
        // and the arm is still gone.
        let mut armed = Some("Sotto".to_string());
        assert_eq!(arm_take(&mut armed, true), None);
        assert_eq!(armed, None);
    }

    #[test]
    fn trainer_takes_are_not_journaled() {
        assert!(journals_take(false), "a normal take is crash-journaled");
        assert!(!journals_take(true), "a trainer take must never come back as a dictation");
    }

    #[test]
    fn auto_send_only_for_a_listed_app_and_a_whole_landed_injection() {
        let apps = vec!["Slack".to_string(), "Terminal".to_string()];
        assert!(auto_sends(&apps, "slack", true), "matched like app_tones, ignoring case");
        assert!(auto_sends(&apps, "Terminal", true), "Windows Terminal, as app_name names it");
        assert!(!auto_sends(&apps, "Slack", false), "a failed, partial or misdirected injection never sends");
        assert!(!auto_sends(&apps, "notepad", true), "an app not on the list");
        assert!(!auto_sends(&[], "Slack", true), "off by default");
        // Never global: no wildcard, and an unknown app matches nothing.
        assert!(!auto_sends(&["*".to_string()], "Slack", true));
        assert!(!auto_sends(&[String::new()], "", true));
    }

    #[test]
    fn heard_word_matches_whole_words_and_multiword_targets() {
        // Single-word: whole-word, case-insensitive, punctuation-tolerant.
        assert!(heard_word("I asked Claude, thanks", "claude"));
        assert!(heard_word("CLAUDE is here", "Claude"));
        // Not a substring of a bigger word.
        assert!(!heard_word("the clauded output", "claude"));
        assert!(!heard_word("nothing relevant here", "Claude"));
        // Multi-word target: its words in a row, same rules.
        assert!(heard_word("open kai's flow now", "Kai's Flow"));
        assert!(heard_word("Try the Gemini CLI.", "gemini cli"));
        assert!(!heard_word("the gemini clip", "Gemini CLI"));
        assert!(!heard_word("gemini and the cli", "Gemini CLI"));
        assert!(!heard_word("", "Claude"));
        assert!(!heard_word("Claude", "  "));
    }

    #[test]
    fn glow_and_sample_agree_on_trailing_punctuation() {
        // #49: the whole transcript of a trainer take is "Claude." The peek
        // glows on it, so the sample must count it as a match too.
        assert!(peek_fires("Claude.", "Claude", 1, 1));
        assert!(heard_word("Claude.", "Claude"));
        assert!(heard_word("\"Claude?\"", "claude"));
        assert!(heard_word("Kai's Flow!", "Kai's Flow"));
        // Arabic has no case; its words still match whole.
        assert!(heard_word("قول كشري.", "كشري"));
        assert!(!heard_word("كشريات", "كشري"));
    }

    #[test]
    fn take_info_counts_words_once_transcribed() {
        let i = TakeInfo::from(&take(Some("hello there my friend"), 4000, "Couldn't paste into that app"));
        assert_eq!(i.words, 4);
        assert_eq!(i.reason, "Couldn't paste into that app");
    }

    #[test]
    fn take_info_has_no_word_count_before_transcription() {
        // Cancelled mid-recording: there is no transcript, so the card has to
        // fall back to audio length. Reporting "0 words" would be a lie about
        // what's actually sitting in the stash.
        let i = TakeInfo::from(&take(None, 4200, "Cancelled"));
        assert_eq!(i.words, 0);
        assert_eq!(i.audio_ms, 4200);
    }

    // 1920x1080 work area starting at the monitor origin, the real 280x120
    // overlay window size, 8px margin — same numbers `position_overlay`
    // passes in practice at scale 1.0 on a single monitor with a normal
    // (non-auto-hide) taskbar.
    const AREA_X: f64 = 0.0;
    const AREA_Y: f64 = 0.0;
    const MON_W: f64 = 1920.0;
    const MON_H: f64 = 1080.0;
    const PILL_W: f64 = 280.0;
    const PILL_H: f64 = 120.0;
    const MARGIN: f64 = 8.0;

    fn anchor(name: &str) -> (i32, i32) {
        anchor_xy(AREA_X, AREA_Y, MON_W, MON_H, PILL_W, PILL_H, name, MARGIN)
    }

    #[test]
    fn anchor_xy_top_left() {
        assert_eq!(anchor("top-left"), (8, 8));
    }
    #[test]
    fn anchor_xy_top_center() {
        assert_eq!(anchor("top-center"), (820, 8));
    }
    #[test]
    fn anchor_xy_top_right() {
        assert_eq!(anchor("top-right"), (1632, 8));
    }
    #[test]
    fn anchor_xy_middle_left() {
        assert_eq!(anchor("middle-left"), (8, 480));
    }
    #[test]
    fn anchor_xy_middle_center() {
        assert_eq!(anchor("middle-center"), (820, 480));
    }
    #[test]
    fn anchor_xy_respects_monitor_origin() {
        // A secondary monitor to the right of a 1920-wide primary: every
        // coordinate anchor_xy returns must be relative to ITS origin, not
        // the desktop's (0, 0) — this is what a work-area-aware caller
        // relies on for anything but the primary monitor.
        let (x, y) = anchor_xy(1920.0, 0.0, MON_W, MON_H, PILL_W, PILL_H, "bottom-right", MARGIN);
        assert_eq!((x, y), (1920 + 1632, 952));
    }
    // ── E2: chunked transcription ────────────────────────────────────
    //
    // `speech` is deliberately loud enough to sit above SILENCE_RMS and
    // `quiet` below it — these tests are about where the cut lands, and a
    // signal that straddles the threshold would make them meaningless.
    fn speech(n: usize) -> Vec<f32> {
        (0..n).map(|i| if i % 2 == 0 { 0.4 } else { -0.4 }).collect()
    }
    fn quiet(n: usize) -> Vec<f32> {
        vec![0.0; n]
    }

    #[test]
    fn short_takes_are_never_chunked() {
        // The common case: a few seconds of dictation must take exactly the
        // path it took before chunking existed.
        assert_eq!(chunk_cut(&speech(16_000 * 5)), None);
        assert_eq!(chunk_cut(&speech(CHUNK_MIN_SAMPLES - 1)), None);
    }

    #[test]
    fn no_cut_while_still_mid_sentence() {
        // Past the minimum but talking straight through with no pause — the
        // chunk keeps growing rather than cutting a word in half.
        assert_eq!(chunk_cut(&speech(CHUNK_MIN_SAMPLES + 16_000)), None);
    }

    #[test]
    fn cuts_inside_the_pause() {
        // 10s of speech, a 1s pause, then more speech. The cut has to land
        // inside the pause, not in either burst of speech.
        let pause_at = CHUNK_MIN_SAMPLES + 16_000 / 2;
        let mut audio = speech(pause_at);
        audio.extend(quiet(16_000));
        audio.extend(speech(16_000 * 3));
        let cut = chunk_cut(&audio).expect("a clear pause should produce a cut");
        assert!(
            cut >= pause_at && cut <= pause_at + 16_000,
            "cut at {cut} is outside the pause [{pause_at}, {}]",
            pause_at + 16_000
        );
    }

    #[test]
    fn unbroken_speech_still_cuts_at_the_ceiling() {
        // Someone who never pauses can't be allowed to defer transcription
        // forever — past the ceiling we cut anyway and accept the seam.
        let cut = chunk_cut(&speech(CHUNK_MAX_SAMPLES + 16_000))
            .expect("must cut once past CHUNK_MAX_SAMPLES");
        assert!(cut >= CHUNK_MIN_SAMPLES && cut <= CHUNK_MAX_SAMPLES);
    }

    /// Syllable-like bursts: 200 ms of a 150 Hz tone at peak `amp`, then 300
    /// ms of nothing, over and over.
    fn bursts(n: usize, amp: f32) -> Vec<f32> {
        let tone = |i: usize| amp * (i as f32 * 150.0 * std::f32::consts::TAU / 16_000.0).sin();
        (0..n).map(|i| if i % 8_000 < 3_200 { tone(i) } else { 0.0 }).collect()
    }
    /// Steady room tone at `level` RMS: deterministic white noise.
    fn room_tone(n: usize, level: f32) -> Vec<f32> {
        let mut x: u32 = 1;
        (0..n)
            .map(|_| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((x >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0) * level * 3f32.sqrt()
            })
            .collect()
    }
    /// `v` played over room tone at `level` RMS.
    fn on_bed(v: Vec<f32>, level: f32) -> Vec<f32> {
        v.iter().zip(room_tone(v.len(), level)).map(|(a, b)| a + b).collect()
    }
    /// A whole short take or chunk judged on its own.
    fn any_speech(s: &[f32]) -> bool {
        has_speech(s, 0..s.len())
    }

    #[test]
    fn silence_is_recognised_as_silence() {
        // The guard that stops pure room tone from being handed to the model,
        // which answers it with an invented phrase — a whole chunk (#64) or
        // a whole short take (#51) of it.
        assert!(!any_speech(&quiet(CHUNK_MAX_SAMPLES)));
        let mut tone = room_tone(CHUNK_MAX_SAMPLES, 0.004);
        // A few clicks (a desk knock, a key) are not speech either.
        for at in [16_000, 16_000 * 7, 16_000 * 20] {
            tone[at] = 0.5;
        }
        assert!(!any_speech(&tone));
        assert!(!any_speech(&room_tone(16_000 * 3, 0.004)), "a silent short take must not reach the model");
        // #100: a fan louder than the old fixed bar is room tone all the same.
        let fan = room_tone(CHUNK_MAX_SAMPLES, 0.03);
        assert!(rms(&fan) > SILENCE_RMS, "the old fixed bar took this for speech");
        assert!(!any_speech(&fan));
        assert!(!any_speech(&room_tone(16_000 * 3, 0.03)));
    }

    #[test]
    fn quiet_speech_is_not_silence() {
        // #64: a quiet speaker's 10 s chunk averaged under SILENCE_RMS and was
        // thrown away whole. The mean was the wrong question; this is speech.
        let quiet_speech = bursts(CHUNK_MIN_SAMPLES, 0.03);
        assert!(rms(&quiet_speech) < SILENCE_RMS, "the old whole-chunk mean called this silence");
        assert!(any_speech(&quiet_speech));
        // A whole take at mean RMS ~0.01, in a room with some hiss.
        let quieter = on_bed(bursts(CHUNK_MIN_SAMPLES, 0.022), 0.001);
        assert!((rms(&quieter) - 0.01).abs() < 0.001);
        assert!(any_speech(&quieter));
        // One short word in a long quiet chunk survives too: counting voiced
        // frames, not their share, keeps it from drowning.
        let mut one_word = room_tone(CHUNK_MAX_SAMPLES, 0.003);
        one_word.extend(bursts(16_000 * 3 / 10, 0.05));
        assert!(any_speech(&one_word));
        // #51: a short take that is mostly quiet with one spoken second.
        let mut short = quiet(16_000 * 2);
        short.extend(bursts(16_000, 0.03));
        assert!(any_speech(&short), "a spoken second inside a quiet take must survive");
    }

    #[test]
    fn a_quiet_last_clause_after_the_cut_is_kept() {
        // #100: speech, a pause the last cut lands in, then a clause said 20 dB
        // under the rest. Every frame of it sat under the old fixed bar, so the
        // tail was called silence and dropped.
        // Cut the way the live loop does, as soon as the pause is heard.
        let heard = CHUNK_MIN_SAMPLES + 16_000;
        let take = on_bed([bursts(CHUNK_MIN_SAMPLES, 0.1), quiet(16_000 * 2), bursts(16_000 * 2, 0.01)].concat(), 0.001);
        let (_, cut, _) = next_chunk(&take[..heard], 0).expect("the pause should produce a cut");
        assert!(take[cut..].chunks(SPEECH_FRAME).all(|f| rms(f) < SILENCE_RMS), "the old bar called this silence");
        assert!(has_speech(&take, cut..take.len()));
        // The same on a quiet speaker (mean RMS 0.012) whose mic gates its
        // pauses to digital zero: the clause is 20 dB under even that.
        let soft = [bursts(CHUNK_MIN_SAMPLES, 0.027), quiet(16_000 * 2), bursts(16_000 * 2, 0.0027)].concat();
        assert!((rms(&soft[..CHUNK_MIN_SAMPLES]) - 0.012).abs() < 0.001);
        let (_, cut, _) = next_chunk(&soft[..heard], 0).expect("the pause should produce a cut");
        assert!(has_speech(&soft, cut..soft.len()));
    }

    #[test]
    fn a_device_that_opens_on_digital_silence_doesnt_lower_the_bar() {
        // Many capture devices open on a few frames of exact zero. Counted in
        // the floor, those drop the bar to SPEECH_MIN_RMS on an ungated mic,
        // and ordinary room tone passes for speech: the model then gets it and
        // invents a phrase (what #64's silent-chunk rule exists to stop).
        let mut take = vec![0.0; 16_000 / 5];
        take.extend(room_tone(16_000 * 3, 0.004));
        let n = take.len();
        assert!(!has_speech(&take, 0..n), "startup zeros turned room tone into speech");
        // A gated mic's pauses are zero later in the take too: its floor stays
        // zero and quiet speech still counts.
        let mut gated = vec![0.0; 16_000];
        gated.extend(bursts(16_000 * 2, 0.01));
        gated.extend(vec![0.0; 16_000]);
        let m = gated.len();
        assert!(has_speech(&gated, 0..m));
    }

    #[test]
    fn a_room_tone_tail_is_still_dropped() {
        // #64's protection stays: the key released a while after the last
        // word leaves a tail of room tone, which the model would answer with
        // an invented word. Quiet room, hissy room, a fan above the old bar.
        for level in [0.0, 0.001, 0.004, 0.03] {
            let take = on_bed([bursts(CHUNK_MIN_SAMPLES, 0.3), quiet(16_000 * 3)].concat(), level);
            let cut = CHUNK_MIN_SAMPLES + 16_000;
            assert!(has_speech(&take, 0..cut), "speech over room tone at {level}");
            assert!(!has_speech(&take, cut..take.len()), "a room-tone tail at {level}");
        }
    }

    #[test]
    fn chunks_overlap_the_cut_only_when_there_is_speech_to_re_hear() {
        let pause_at = CHUNK_MIN_SAMPLES + 16_000 / 2;
        let mut audio = speech(pause_at);
        audio.extend(quiet(16_000));
        audio.extend(speech(16_000 * 3));
        let (from, end, speech_in) = next_chunk(&audio, 0).expect("a clear pause should produce a cut");
        assert_eq!(from, 0, "the first chunk has nothing before it");
        assert!(speech_in);
        // The cut sits just inside the pause, so the second before it is the
        // end of the sentence: the next piece re-hears it.
        assert_eq!(seam_start(&audio, end), end - CHUNK_OVERLAP);
        // After a pause longer than the overlap there's nothing to re-hear,
        // and no overlap means no dedupe at the join.
        let mut long_pause = speech(CHUNK_MIN_SAMPLES);
        long_pause.extend(quiet(16_000 * 2));
        let cut = CHUNK_MIN_SAMPLES + 16_000 * 3 / 2;
        assert_eq!(seam_start(&long_pause, cut), cut);
    }

    #[test]
    fn join_text_handles_empty_sides() {
        // Empty sides are routine: a silent chunk, or no chunking at all.
        for seam in [false, true] {
            assert_eq!(join_text("", "hello", seam), "hello");
            assert_eq!(join_text("hello", "", seam), "hello");
            assert_eq!(join_text("", "", seam), "");
            // No double space when a piece arrives already padded.
            assert_eq!(join_text("first. ", " Second.", seam), "first. Second.");
        }
        assert_eq!(join_text("first part.", "Second part.", false), "first part. Second part.");
    }

    #[test]
    fn seam_join_drops_the_words_both_sides_heard() {
        // The plain case: the overlap repeats the last few words.
        assert_eq!(
            join_text("we should check the numbers again.", "the numbers again. Then send it.", true),
            "we should check the numbers again. Then send it."
        );
        // The first side closed the sentence and the second opened one; the
        // joining word takes the first's letters and the second's punctuation.
        assert_eq!(
            join_text("I want to see the report.", "The report and then we go.", true),
            "I want to see the report and then we go."
        );
        // The first side lost its last words at the cut (the #64 seam loss):
        // the overlap brings them back.
        assert_eq!(join_text("I think he", "he only mentioned it once.", true), "I think he only mentioned it once.");
        // The overlap started mid-word, leaving a fragment in front.
        assert_eq!(
            join_text("check the numbers again.", "ers again. Then send it.", true),
            "check the numbers again. Then send it."
        );
        // The cut clipped the first side's last word; the second heard it whole.
        assert_eq!(join_text("we went to the sto", "to the store and back.", true), "we went to the store and back.");
    }

    #[test]
    fn seam_join_leaves_coincidences_alone() {
        // No overlap (a pause at the cut): a repeated word is just speech.
        assert_eq!(join_text("I'll fix it.", "It works now.", false), "I'll fix it. It works now.");
        // A lone common word away from the seam is not a repeat.
        assert_eq!(join_text("I read the book today.", "The day was long.", true), "I read the book today. The day was long.");
        // Nothing repeated: the second side's model dropped the overlap.
        assert_eq!(join_text("That was the plan.", "Next we test.", true), "That was the plan. Next we test.");
    }

    #[test]
    fn anchor_xy_middle_right() {
        assert_eq!(anchor("middle-right"), (1632, 480));
    }
    #[test]
    fn anchor_xy_bottom_left() {
        assert_eq!(anchor("bottom-left"), (8, 952));
    }
    #[test]
    fn anchor_xy_bottom_center() {
        assert_eq!(anchor("bottom-center"), (820, 952));
    }
    #[test]
    fn anchor_xy_bottom_right() {
        assert_eq!(anchor("bottom-right"), (1632, 952));
    }
    #[test]
    fn anchor_xy_unknown_falls_back_to_bottom_center() {
        assert_eq!(anchor("garbage"), anchor("bottom-center"));
    }

    // ── pill origin inside the 280x120 canvas ───────────────────────────
    // The bug these guard: the pill used to be centred in the canvas, so a
    // 46x16 tucked pill sat 60px below the top edge (and 125px in from the
    // left) instead of tucked against it.
    const CW: f64 = 280.0;
    const CH: f64 = 120.0;
    const TW: f64 = 46.0; // tucked
    const TH: f64 = 16.0;

    #[test]
    fn tucked_pill_hugs_the_top_edge_not_the_canvas_centre() {
        let (_, y) = overlay_pill_origin(CW, CH, TW, TH, "top-center");
        assert_eq!(y, OVERLAY_EDGE_INSET);
        assert_ne!(y, (CH - TH) / 2.0, "centring is the bug this replaced");
    }

    #[test]
    fn tucked_pill_hugs_the_bottom_edge() {
        let (_, y) = overlay_pill_origin(CW, CH, TW, TH, "bottom-center");
        assert_eq!(y, CH - TH - OVERLAY_EDGE_INSET);
    }

    #[test]
    fn tucked_pill_hugs_left_and_right_edges() {
        assert_eq!(overlay_pill_origin(CW, CH, TW, TH, "middle-left").0, OVERLAY_EDGE_INSET);
        assert_eq!(overlay_pill_origin(CW, CH, TW, TH, "middle-right").0, CW - TW - OVERLAY_EDGE_INSET);
    }

    #[test]
    fn centre_anchors_still_centre_on_that_axis() {
        let (x, y) = overlay_pill_origin(CW, CH, TW, TH, "middle-center");
        assert_eq!((x, y), (((CW - TW) / 2.0).round(), ((CH - TH) / 2.0).round()));
    }

    /// An expanded toast is wider than the tucked pill, so anchoring must grow
    /// it *inward* from the edge rather than pushing it off-screen.
    #[test]
    fn expanded_pill_grows_inward_from_the_anchored_edge() {
        let (x, y) = overlay_pill_origin(CW, CH, 248.0, 40.0, "bottom-right");
        assert_eq!(x, CW - 248.0 - OVERLAY_EDGE_INSET);
        assert_eq!(y, CH - 40.0 - OVERLAY_EDGE_INSET);
        assert!(x >= 0.0 && y >= 0.0, "must stay inside the canvas");
    }
}
