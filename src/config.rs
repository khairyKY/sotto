use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActivationMode {
    Hold,
    Toggle,
}

impl ActivationMode {
    /// Stable u8 encoding so the mode can live in an `AtomicU8` the settings
    /// window flips and the hotkey listener reads live.
    pub fn as_u8(self) -> u8 {
        match self {
            ActivationMode::Hold => 0,
            ActivationMode::Toggle => 1,
        }
    }
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => ActivationMode::Toggle,
            _ => ActivationMode::Hold,
        }
    }
}

/// A dictation dictionary / snippet entry: when the transcript contains
/// `spoken` (case-insensitive, whole phrase), it's replaced with `replacement`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DictEntry {
    pub spoken: String,
    pub replacement: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InjectionMode {
    Unicode,
    Paste,
}

/// How much cleanup to apply to the raw transcript before injection.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PolishMode {
    /// Inject the raw transcript verbatim (only trimmed).
    Off,
    /// Tier 0 only: instant, local, zero-cost rules cleanup.
    Rules,
    /// Tier 1: route longer dictations through the local LLM, falling back to
    /// Tier 0 for short ones and whenever the LLM is unavailable or times out.
    Ai,
}

impl PolishMode {
    /// Stable u8 encoding so the mode can live in an `AtomicU8` that the tray
    /// (and later the settings window) flips and the polisher reads live.
    pub fn as_u8(self) -> u8 {
        match self {
            PolishMode::Off => 0,
            PolishMode::Rules => 1,
            PolishMode::Ai => 2,
        }
    }
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => PolishMode::Off,
            2 => PolishMode::Ai,
            _ => PolishMode::Rules,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PolishConfig {
    pub mode: PolishMode,
    /// Dictations with at least this many words go through the AI tier (when
    /// `mode = ai`). Shorter clips stay on the instant rules tier — the LLM
    /// round-trip isn't worth it for a few words.
    pub ai_min_words: usize,
}

impl Default for PolishConfig {
    fn default() -> Self {
        Self {
            mode: PolishMode::Rules,
            ai_min_words: 18,
        }
    }
}

/// Tunables for the Tier 1 llama.cpp sidecar. Model and executable *paths* are
/// derived from `data_dir()` (see `llm_model_path` / `llama_server_exe`) rather
/// than stored here, so config.toml stays machine-independent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmConfig {
    /// Loopback port the sidecar listens on.
    pub port: u16,
    /// GPU layers to offload (99 = all; the 1.5B Q4 fits the RTX 3050 fully).
    pub n_gpu_layers: u32,
    pub ctx_size: u32,
    pub temperature: f32,
    pub max_tokens: u32,
    /// Hard wall-clock cap on a single completion before falling back to rules.
    pub request_timeout_ms: u64,
    /// How long to wait for a cold server to load the model and go healthy.
    pub spawn_timeout_secs: u64,
    /// Kill the sidecar (freeing VRAM) after this much inactivity.
    pub idle_kill_secs: u64,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            port: 8177,
            n_gpu_layers: 99,
            ctx_size: 2048,
            temperature: 0.2,
            max_tokens: 1024,
            request_timeout_ms: 8000,
            spawn_timeout_secs: 30,
            idle_kill_secs: 300,
        }
    }
}

/// Which speech engine to transcribe with, and what language to expect.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AsrConfig {
    /// "parakeet-v3" (default: English-only, fast, small download) or
    /// "whisper-turbo" (multilingual, larger download).
    pub model: String,
    /// BCP-47 code (e.g. "en", "ar"), or "auto" to let the engine detect it.
    /// Parakeet is English-only and ignores this either way.
    pub language: String,
}

impl Default for AsrConfig {
    fn default() -> Self {
        Self {
            model: "parakeet-v3".to_string(),
            language: "auto".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// rdev key name, e.g. "ControlRight", "F13", "AltRight".
    pub hotkey: String,
    pub activation_mode: ActivationMode,
    pub injection_mode: InjectionMode,
    #[serde(default)]
    pub polish: PolishConfig,
    #[serde(default)]
    pub llm: LlmConfig,
    #[serde(default)]
    pub asr: AsrConfig,
    /// Dictation dictionary / snippet replacements, edited in the settings window.
    #[serde(default)]
    pub dictionary: Vec<DictEntry>,
    /// Start minimized to the tray (no window shown on launch).
    #[serde(default = "default_true")]
    pub start_hidden: bool,
    /// Record local usage stats (counts + timings only, never text) for the
    /// Insights dashboard. Fully local either way.
    #[serde(default = "default_true")]
    pub stats_enabled: bool,
    /// UI theme: "light", "dark", or "system" (follow OS preference).
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Input device name to record from, or `None` for the OS default.
    #[serde(default)]
    pub microphone: Option<String>,
    /// Soft tick when recording starts and stops.
    #[serde(default = "default_true")]
    pub sound_enabled: bool,
    /// App-window zoom (1.0 = 100%). Applied via the webview's native zoom,
    /// so layout stays correct at any factor.
    #[serde(default = "default_zoom")]
    pub zoom: f64,
    /// Where the ~2.8 GB of models + llama runtime live. Empty = keep them
    /// next to this config in `data_dir()`.
    ///
    /// This exists because the assets are ~2.8 GB and the config is ~400 bytes:
    /// anyone whose system drive is tight needs to put the big half elsewhere
    /// (`D:\sotto`, an external disk, …) without moving their settings. Only
    /// the assets are relocatable — config/stats/logs always stay in
    /// `data_dir()`, because `config.toml` can't tell us where `config.toml` is.
    #[serde(default)]
    pub assets_dir: String,
}

fn default_zoom() -> f64 {
    1.0
}

fn default_theme() -> String {
    "system".to_string()
}

fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: "ControlRight".to_string(),
            activation_mode: ActivationMode::Hold,
            // Paste proved far more reliable than raw Unicode injection across
            // apps during Phase 0 testing; Unicode is an opt-in override.
            injection_mode: InjectionMode::Paste,
            polish: PolishConfig::default(),
            llm: LlmConfig::default(),
            asr: AsrConfig::default(),
            dictionary: Vec::new(),
            start_hidden: true,
            stats_enabled: true,
            theme: default_theme(),
            microphone: None,
            sound_enabled: true,
            zoom: default_zoom(),
            assets_dir: String::new(),
        }
    }
}

/// Locate a downloaded asset (model / runtime DLL). Resolution order:
/// `SOTTO_DATA_DIR` env → a `resources/` dir next to the exe (portable builds)
/// → `assets_dir()`.
fn find_asset(relative_path: PathBuf) -> PathBuf {
    if let Ok(dir) = std::env::var("SOTTO_DATA_DIR") {
        return PathBuf::from(dir).join(&relative_path);
    }
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let asset_path = exe_dir.join("resources").join(&relative_path);
            if asset_path.exists() {
                return asset_path;
            }
        }
    }
    assets_dir().join(&relative_path)
}

/// Writable root for Sotto's *small* state: config.toml, stats.jsonl, logs.
/// A few hundred KB, and always in the OS-standard per-user location.
///
/// This deliberately can't be configured: `Config::path()` is
/// `data_dir()/config.toml`, so anything that decided this location would have
/// to be read before we know where the config is. The big, relocatable half is
/// `assets_dir()` instead — that's the ~2.8 GB that actually needs a choice.
///
/// Override with `SOTTO_DATA_DIR` (tests, portable installs).
pub fn data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SOTTO_DATA_DIR") {
        return PathBuf::from(dir);
    }
    match std::env::var("APPDATA") {
        Ok(appdata) => PathBuf::from(appdata).join("sotto"),
        // APPDATA is always set on a real Windows session; this only trips in
        // odd service contexts. Keep the app runnable rather than panicking.
        Err(_) => PathBuf::from(r"C:\ProgramData\sotto"),
    }
}

/// Root for the ~2.8 GB of downloaded models + llama runtime.
///
/// `assets_dir` in config.toml when set (e.g. `D:\sotto` to keep a tight system
/// drive free), otherwise alongside the config in `data_dir()`.
///
/// Read straight from disk rather than from the live `Config` because the ORT
/// dll path has to be resolved during `init_ort()`, before Tauri state exists.
/// It's a ~400-byte file read a handful of times at startup.
pub fn assets_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SOTTO_DATA_DIR") {
        return PathBuf::from(dir);
    }
    let configured = std::fs::read_to_string(Config::path())
        .ok()
        .and_then(|s| toml::from_str::<Config>(&s).ok())
        .map(|c| c.assets_dir)
        .unwrap_or_default();
    if configured.trim().is_empty() {
        data_dir()
    } else {
        PathBuf::from(configured.trim())
    }
}

/// Which ASR engine is configured, read the same way as `assets_dir()` — a
/// raw disk read rather than `Config::load_or_init()`, so asset provisioning
/// (which can run before or without full app state) never has the side
/// effect of writing a fresh config.toml just to check this.
pub fn asr_model() -> String {
    std::fs::read_to_string(Config::path())
        .ok()
        .and_then(|s| toml::from_str::<Config>(&s).ok())
        .map(|c| c.asr.model)
        .unwrap_or_else(|| AsrConfig::default().model)
}

/// Directory the Parakeet v3 int8 model files live in.
pub fn model_dir() -> PathBuf {
    find_asset(PathBuf::from("models").join("parakeet-tdt-0.6b-v3-int8"))
}

/// GGML model file for the Whisper engine (`asr.model = "whisper-turbo"`).
/// A single file, unlike Parakeet's directory of ONNX parts.
pub fn whisper_model_path() -> PathBuf {
    find_asset(PathBuf::from("models").join("ggml-large-v3-turbo-q5_0.bin"))
}

/// Is the *configured* speech model actually on disk yet?
///
/// Lives here rather than at the call site because the answer differs per
/// engine — Parakeet is a directory of ONNX parts, Whisper is one .bin — and a
/// caller that hardcodes either one silently misreports the other. It drives
/// the "still downloading" vs "real failure" split the overlay shows, so
/// getting it wrong tells the user to wait for a download that already
/// finished.
pub fn asr_model_present() -> bool {
    match asr_model().as_str() {
        "whisper-turbo" => whisper_model_path().exists(),
        _ => model_dir().join("encoder-model.int8.onnx").exists(),
    }
}

/// Path to the ONNX Runtime shared library (`ort` load-dynamic target).
pub fn onnxruntime_dll() -> PathBuf {
    find_asset(PathBuf::from("onnxruntime.dll"))
}

/// GGUF model file for the Tier 1 LLM polish sidecar.
pub fn llm_model_path() -> PathBuf {
    find_asset(PathBuf::from("models").join("qwen2.5-1.5b-instruct-q4_k_m.gguf"))
}

/// The bundled llama.cpp server executable.
pub fn llama_server_exe() -> PathBuf {
    find_asset(PathBuf::from("runtime").join("llama").join("llama-server.exe"))
}

impl Config {
    pub fn path() -> PathBuf {
        data_dir().join("config.toml")
    }

    /// Load config from disk, creating a default file on first run.
    pub fn load_or_init() -> anyhow::Result<Self> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        if path.exists() {
            let raw = std::fs::read_to_string(&path)?;
            let cfg: Config = toml::from_str(&raw)?;
            Ok(cfg)
        } else {
            let cfg = Config::default();
            let raw = toml::to_string_pretty(&cfg)?;
            std::fs::write(&path, raw)?;
            tracing::info!(path = %path.display(), "wrote default config");
            Ok(cfg)
        }
    }

    /// Persist the current config to disk (called by the settings window on edit).
    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, toml::to_string_pretty(self)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod dir_tests {
    use super::*;

    /// The whole point of splitting data_dir from assets_dir: config.toml can't
    /// tell us where config.toml is, so only the big half is relocatable.
    #[test]
    fn config_lives_in_data_dir_not_assets_dir() {
        assert_eq!(Config::path(), data_dir().join("config.toml"));
    }

    #[test]
    fn default_does_not_pin_assets_to_a_drive() {
        assert!(Config::default().assets_dir.is_empty());
    }

    /// Guards the bug this replaced: a `D:\sotto` literal was baked into path
    /// resolution, so any machine that happened to have that folder got 2.8 GB
    /// written to it, and machines without a D: drive had no way to move the
    /// assets at all. Matches the construct, not prose — doc comments are free
    /// to name D:\sotto as the example it now is.
    #[test]
    fn no_drive_letter_is_hardcoded_in_path_resolution() {
        let src = include_str!("config.rs");
        let logic = &src[..src.find("mod dir_tests").unwrap()];
        let code: String = logic
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect();
        assert!(
            !code.contains(r#"PathBuf::from(r"D:"#),
            "path resolution must not construct a hardcoded drive path"
        );
    }

    #[test]
    fn assets_dir_is_configurable_and_trimmed() {
        let cfg: Config = toml::from_str(
            r#"
hotkey = "ControlRight"
activation_mode = "toggle"
injection_mode = "paste"
assets_dir = '  D:\sotto  '
"#,
        )
        .unwrap();
        // Trimmed at use, so a stray space in a hand-edited config still works.
        assert_eq!(cfg.assets_dir.trim(), r"D:\sotto");
    }
}
