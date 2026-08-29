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

/// Which page an entry belongs to in the Settings UI — and, since both kinds
/// also fire at a different point in the polish pipeline (see `polish.rs`'s
/// `polish_with_tone`), which pass replaces it: `Word` entries run BEFORE
/// polish (so the LLM sees the correct token instead of an ASR fragment it
/// might "correct" away), `Snippet` entries run AFTER (so grammar rewriting
/// never gets a chance to mangle a longer expansion like an email address).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    #[default]
    Word,
    Snippet,
}

/// A dictation dictionary / snippet entry: when the transcript contains
/// `spoken` — or any of `aliases` — (case-insensitive, whole phrase), it's
/// replaced with `replacement`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DictEntry {
    pub spoken: String,
    pub replacement: String,
    /// Other ways you say the same thing. "my main email", "my primary
    /// email", "my personal email" should all produce one address — without
    /// this you'd need three entries that drift apart the moment you edit one.
    /// `serde(default)` keeps configs written before aliases existed loading.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Per-entry off switch: keep the entry but stop it firing. Deleting to
    /// silence something temporarily loses the replacement text, which is the
    /// part that was actually hard to type.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// `None` only for entries loaded from a config written before this field
    /// existed — the UI used to *guess* Dictionary vs. Snippets from the
    /// replacement's shape (has a space/@/newline, or is long), which
    /// misfiled anything that broke the pattern (e.g. a 3-letter snippet).
    /// New entries always set this explicitly at creation; `load_or_init`
    /// backfills `None` once via `migrate_entry_kinds`, using that same old
    /// guess so upgrading never reshuffles an existing entry's page.
    #[serde(default)]
    pub kind: Option<EntryKind>,
}

/// Backfills `kind` on entries from a config written before it existed, using
/// the exact heuristic the UI used to apply live (see the removed `isSnippet`
/// in ui/index.js). Returns whether anything changed, so the caller knows to
/// persist — without this a config with legacy entries would re-derive (and
/// silently re-save) the same values every single launch.
fn migrate_entry_kinds(dictionary: &mut [DictEntry]) -> bool {
    let mut changed = false;
    for e in dictionary.iter_mut() {
        if e.kind.is_none() {
            let r = &e.replacement;
            let looks_like_snippet =
                r.contains(' ') || r.contains('\n') || r.contains('@') || r.chars().count() > 15;
            e.kind = Some(if looks_like_snippet { EntryKind::Snippet } else { EntryKind::Word });
            changed = true;
        }
    }
    changed
}

impl DictEntry {
    /// Every phrase that should trigger this entry, longest first.
    ///
    /// Order is load-bearing: with "my email" and "my primary email" both
    /// live, matching the shorter one first would consume "my" + "email" and
    /// strand "primary". Longest-first is the standard fix and the only reason
    /// overlapping aliases are safe to offer at all.
    pub fn phrases(&self) -> Vec<&str> {
        let mut all: Vec<&str> = std::iter::once(self.spoken.as_str())
            .chain(self.aliases.iter().map(|a| a.as_str()))
            .filter(|p| !p.trim().is_empty())
            .collect();
        all.sort_by_key(|p| std::cmp::Reverse(p.len()));
        all
    }
}

/// A per-app tone override: `app` is matched case-insensitively against
/// `stats::app_name()`'s output (the focused app when the take was recorded);
/// `tone` is the instruction text itself — same free-form string the default
/// `Config::tone` holds — appended to the AI-polish system prompt.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppTone {
    pub app: String,
    pub tone: String,
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
    /// Master switch for every replacement above. Off = transcripts pass
    /// through untouched, without you having to disable entries one by one or
    /// delete work you want to keep.
    #[serde(default = "default_true")]
    pub replacements_enabled: bool,
    /// "New line"/"new paragraph" spoken commands become real line breaks.
    /// Its own toggle, independent of `polish.mode` — see `polish.rs`'s
    /// `apply_formatting_commands` for why it applies even when polish is Off.
    #[serde(default = "default_true")]
    pub formatting_commands: bool,
    /// Transcribe long takes in pieces *while* you're still speaking, so the
    /// wait after you release the key is the last few seconds of audio instead
    /// of all of it. Short takes are never chunked (see `CHUNK_MIN_SAMPLES`).
    ///
    /// The knob exists because this is a real trade-off, not a free win: a
    /// chunk boundary is a point where the model loses context, so a long take
    /// can come back very slightly differently punctuated than it would have
    /// in one pass. Set false to go back to one-shot transcription.
    #[serde(default = "default_true")]
    pub chunked_transcription: bool,
    /// Default tone instruction appended to the AI-polish system prompt.
    /// Empty = off — today's prompt, byte-identical. AI tier only: Rules/Off
    /// strip and fix, they don't re-voice a sentence, so this has no effect
    /// there (see `Polisher::polish_with_tone`).
    #[serde(default)]
    pub tone: String,
    /// Per-app tone overrides. Falls back to `tone` when the focused app
    /// (matched case-insensitively) has no entry here.
    #[serde(default)]
    pub app_tones: Vec<AppTone>,
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
    /// Overlay pill placement + click-to-record (N1).
    #[serde(default)]
    pub overlay: OverlayConfig,
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
            replacements_enabled: true,
            formatting_commands: true,
            chunked_transcription: true,
            tone: String::new(),
            app_tones: Vec::new(),
            start_hidden: true,
            stats_enabled: true,
            theme: default_theme(),
            microphone: None,
            sound_enabled: true,
            overlay: OverlayConfig::default(),
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

/// The GGML filename for a whisper-family engine, or `None` for Parakeet.
/// Single source of truth for "which .bin does this engine use" — every other
/// function that needs the whisper path routes through here, so a new engine is
/// one line and can't drift between the loader, the downloader, and the UI.
pub fn whisper_model_file(model: &str) -> Option<&'static str> {
    match model {
        "whisper-turbo" => Some("ggml-large-v3-turbo-q5_0.bin"),
        "egyptian-small" => Some("ggml-egyptian-codeswitch-small.bin"),
        _ => None,
    }
}

/// Overlay pill placement + click-to-record (N1). `position` is one of the 9
/// anchor strings `main.rs`'s `OVERLAY_ANCHORS` lists — validated at the
/// `set_overlay_position` command, not here, so a hand-edited garbage value
/// still loads fine (the anchor arithmetic falls back to bottom-center at
/// render time rather than panicking).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlayConfig {
    pub position: String,
    /// Opt-in: keep the idle pill on screen and let its body start a
    /// dictation. Off by default — today's hide-when-idle behavior,
    /// byte-identical.
    pub always_visible: bool,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            position: "bottom-center".to_string(),
            always_visible: false,
        }
    }
}

/// Path to a whisper-family engine's GGML file. A single file, unlike Parakeet's
/// directory of ONNX parts. Non-whisper ids fall back to the turbo file, but
/// only an id that slipped past validation would ever reach that.
pub fn whisper_model_path(model: &str) -> PathBuf {
    let file = whisper_model_file(model).unwrap_or("ggml-large-v3-turbo-q5_0.bin");
    find_asset(PathBuf::from("models").join(file))
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
    let model = asr_model();
    if whisper_model_file(&model).is_some() {
        whisper_model_path(&model).exists()
    } else {
        model_dir().join("encoder-model.int8.onnx").exists()
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
            let mut cfg: Config = toml::from_str(&raw)?;
            if migrate_entry_kinds(&mut cfg.dictionary) {
                tracing::info!(
                    entries = cfg.dictionary.len(),
                    "migrated dictionary entries to explicit kind"
                );
                cfg.save()?;
            }
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

#[cfg(test)]
mod entry_kind_tests {
    use super::*;

    fn entry(replacement: &str) -> DictEntry {
        DictEntry { spoken: "x".into(), replacement: replacement.into(), kind: None, ..Default::default() }
    }

    #[test]
    fn migration_matches_the_old_ui_heuristic_word_vs_snippet() {
        // Short, no space/@/newline -> word. This is the exact shape check
        // ui/index.js's removed `isSnippet` used to run client-side; the
        // point of this test is that upgrading never reclassifies a
        // pre-existing entry, so it has to agree with that old logic exactly.
        let mut dict = vec![entry("GPT"), entry("khairyshhn1@gmail.com"), entry("Hey there, great to meet you")];
        assert!(migrate_entry_kinds(&mut dict));
        assert_eq!(dict[0].kind, Some(EntryKind::Word)); // short, plain
        assert_eq!(dict[1].kind, Some(EntryKind::Snippet)); // has '@'
        assert_eq!(dict[2].kind, Some(EntryKind::Snippet)); // has a space
    }

    #[test]
    fn migration_is_a_noop_once_kind_is_already_set() {
        // Real installs re-run load_or_init on every launch; if this weren't
        // idempotent, a manually-recategorized entry would silently flip back
        // to the guess on the next restart.
        let mut dict = vec![DictEntry { kind: Some(EntryKind::Snippet), ..entry("short") }];
        assert!(!migrate_entry_kinds(&mut dict));
        assert_eq!(dict[0].kind, Some(EntryKind::Snippet));
    }

    #[test]
    fn short_snippet_is_the_bug_this_fixes() {
        // The whole reason this field exists: "KIS" is short, no space/@, so
        // the old heuristic would have guessed Word even for an entry the
        // user explicitly created as a Snippet. Explicit `kind` from the UI
        // must never be overridden by the shape-guess.
        let mut dict = vec![DictEntry { kind: Some(EntryKind::Snippet), ..entry("KIS") }];
        migrate_entry_kinds(&mut dict);
        assert_eq!(dict[0].kind, Some(EntryKind::Snippet));
    }
}
