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
    /// Dictation dictionary / snippet replacements, edited in the settings window.
    #[serde(default)]
    pub dictionary: Vec<DictEntry>,
    /// Start minimized to the tray (no window shown on launch).
    #[serde(default = "default_true")]
    pub start_hidden: bool,
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
            dictionary: Vec::new(),
            start_hidden: true,
        }
    }
}

/// Root directory for all Sotto data (config, models, the ONNX runtime dll).
///
/// Defaults to `D:\sotto` rather than `%APPDATA%`: this machine's C: drive is
/// space-constrained and the Parakeet model alone is ~670 MB. Override with
/// the `SOTTO_DATA_DIR` environment variable.
pub fn data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SOTTO_DATA_DIR") {
        return PathBuf::from(dir);
    }
    PathBuf::from(r"D:\sotto")
}

/// Directory the Parakeet v3 int8 model files live in.
pub fn model_dir() -> PathBuf {
    data_dir().join("models").join("parakeet-tdt-0.6b-v3-int8")
}

/// Path to the ONNX Runtime shared library (`ort` load-dynamic target).
pub fn onnxruntime_dll() -> PathBuf {
    data_dir().join("onnxruntime.dll")
}

/// GGUF model file for the Tier 1 LLM polish sidecar.
pub fn llm_model_path() -> PathBuf {
    data_dir()
        .join("models")
        .join("qwen2.5-1.5b-instruct-q4_k_m.gguf")
}

/// The bundled llama.cpp server executable.
pub fn llama_server_exe() -> PathBuf {
    data_dir()
        .join("runtime")
        .join("llama")
        .join("llama-server.exe")
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
