//! Speech-to-text via `transcribe-rs`, switchable between Parakeet TDT 0.6b v3
//! (int8, English-only, fast) and Whisper large-v3-turbo (multilingual)
//! depending on `config::AsrConfig::model`.
//!
//! The model (hundreds of MB) is preloaded at startup, dropped after
//! `asr.idle_unload_secs` without work, and loaded again at the next hotkey
//! press (#12). The transcribe thread owns the one `Asr` and calls `sync`
//! between takes, so an engine picked in Settings takes over on the next take
//! (#10), and is loaded as soon as it is picked and on disk (#118). ONNX
//! Runtime (used by Parakeet) is loaded dynamically at runtime from
//! `onnxruntime.dll` (see `main::init_ort`).

use crate::config::{self, VocabEntry};
use anyhow::Context;
use std::sync::Mutex;
use std::time::Duration;
use transcribe_rs::onnx::parakeet::ParakeetModel;
use transcribe_rs::onnx::Quantization;
use transcribe_rs::whisper_cpp::gpu::auto_select_gpu_device;
use transcribe_rs::{
    ModelCapabilities, SpeechModel, TranscribeError, TranscribeOptions, TranscriptionResult,
};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState};

pub struct Asr {
    model: Option<Box<dyn SpeechModel>>,
    /// The engine `model` is (or will next be) loaded as, e.g. "parakeet-v3"
    /// or "whisper-turbo". Follows `config::AsrConfig::model` via `sync`.
    engine: String,
    /// `None` = auto-detect (config `language = "auto"`).
    language: Option<String>,
    /// Whisper's `initial_prompt`, rebuilt from the trained vocabulary by
    /// every `sync`. A loaded model keeps the prompt it was loaded with, so a
    /// newly trained word joins at the next load (engine switch or idle
    /// reload). ponytail: not worth seconds of reload per trained word.
    prompt: Option<String>,
    /// `asr.idle_unload_secs`; 0 keeps the model loaded.
    idle_unload_secs: u64,
    /// `asr.whisper_beam`, taken at the next load like `prompt`.
    whisper_beam: u32,
}

/// The engine behind the latest transcript. Flags are labelled with it (#10):
/// after a hot switch, the configured engine is not always the one that ran.
static LAST_ENGINE: Mutex<String> = Mutex::new(String::new());

/// The engine that produced the latest transcript this session, if any.
pub fn last_engine() -> Option<String> {
    let engine = LAST_ENGINE.lock().unwrap().clone();
    (!engine.is_empty()).then_some(engine)
}

/// What an engine's model is doing, for its row in Settings (#118).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Load {
    /// Not in memory: not loaded yet, idle-unloaded (#12), or not on disk.
    Idle,
    Loading,
    Ready,
}

/// The engine the transcribe thread runs and what its model is doing: the
/// `asr-load` event's payload.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct LoadState {
    pub engine: String,
    pub state: Load,
}

/// The last `LoadState` told, for a Settings window built after the event (#13).
static LOAD: Mutex<Option<LoadState>> = Mutex::new(None);

pub fn load_state() -> Option<LoadState> {
    LOAD.lock().unwrap().clone()
}

/// Record what the model is doing. `Some` when that is news, for the caller
/// to emit.
pub fn set_load(engine: &str, state: Load) -> Option<LoadState> {
    let next = LoadState { engine: engine.to_string(), state };
    let mut last = LOAD.lock().unwrap();
    if last.as_ref() == Some(&next) {
        return None;
    }
    *last = Some(next.clone());
    Some(next)
}

/// Whether there is a model to load now (#118): not in memory, and its files
/// are on disk. A picked engine that is still downloading has nothing to
/// load yet, so its row says nothing about loading.
fn wants_load(loaded: bool, present: bool) -> bool {
    !loaded && present
}

/// Which engine `sync` should run (#10): the configured one, except while its
/// files are missing and the current engine's are there. A Download click
/// selects an engine before its files land, and dictation keeps working on
/// the old one until they do.
fn next_engine<'a>(current: &'a str, configured: &'a str, present: impl Fn(&str) -> bool) -> &'a str {
    if configured == current || present(configured) || !present(current) {
        configured
    } else {
        current
    }
}

/// How long the transcribe thread waits for work before `unload` (#12):
/// forever while nothing is loaded, or when idle unloading is off (0).
fn idle_wait(loaded: bool, idle_unload_secs: u64) -> Duration {
    if loaded && idle_unload_secs > 0 {
        Duration::from_secs(idle_unload_secs)
    } else {
        Duration::MAX // crossbeam's recv_timeout falls back to a plain recv
    }
}

/// Maps the config's language string to what `TranscribeOptions` expects,
/// for `engine`. egyptian-small always decodes as Arabic (#68): left to
/// detect, it writes English speech out as Arabic. `asr.language` itself is
/// kept, for when another engine is picked again.
fn to_language_option(engine: &str, lang: &str) -> Option<String> {
    if engine == "egyptian-small" {
        Some("ar".to_string())
    } else if lang.eq_ignore_ascii_case("auto") {
        None
    } else {
        Some(lang.to_string())
    }
}

/// Byte cap for the vocabulary prompt. Whisper's decoder keeps only the last
/// ~223 prompt tokens and drops the FRONT of anything longer, which is where
/// the most relevant words sit. 200 bytes stays well under that, even for
/// Arabic (2 bytes a letter, more tokens per word), and a short prompt is
/// less likely to be echoed back as text.
const PROMPT_MAX_BYTES: usize = 200;

/// The trained vocabulary as a Whisper prompt: "Zorvex, Sotto, Claude."
/// Most relevant first: the words the engine has been caught mishearing
/// (`heard_as` entries plus missed trainer attempts), list order breaking
/// ties. A word that would overflow the cap is skipped, not cut.
fn vocab_prompt(vocab: &[VocabEntry]) -> Option<String> {
    let misses = |e: &VocabEntry| e.heard_as.len() + e.recent.iter().filter(|ok| !**ok).count();
    let mut ranked: Vec<&VocabEntry> = vocab.iter().filter(|e| !e.word.trim().is_empty()).collect();
    ranked.sort_by_key(|e| std::cmp::Reverse(misses(e))); // stable: ties keep list order
    let mut out = String::new();
    for word in ranked.iter().map(|e| e.word.trim()) {
        // ", " before it, "." after the last.
        if out.len() + 2 + word.len() + 1 > PROMPT_MAX_BYTES {
            continue;
        }
        if !out.is_empty() {
            out.push_str(", ");
        }
        out.push_str(word);
    }
    (!out.is_empty()).then(|| out + ".")
}

/// Most decoders whisper.cpp runs at once (`WHISPER_MAX_DECODERS`); a bigger
/// beam fails the whole decode.
const MAX_BEAM: u32 = 8;

/// One Whisper decode's settings. `beam` is `asr.whisper_beam`: 0 is greedy.
/// The rest is what transcribe-rs's `WhisperInferenceParams::default()`
/// sends, plus our prompt.
fn full_params<'a>(beam: u32, language: Option<&'a str>, translate: bool, prompt: Option<&str>) -> FullParams<'a, 'a> {
    let mut p = FullParams::new(match beam {
        0 => SamplingStrategy::Greedy { best_of: 1 },
        n => SamplingStrategy::BeamSearch { beam_size: n.min(MAX_BEAM) as i32, patience: -1.0 },
    });
    p.set_language(language);
    p.set_translate(translate);
    p.set_print_special(false);
    p.set_print_progress(false);
    p.set_print_realtime(false);
    p.set_print_timestamps(false);
    p.set_suppress_blank(true);
    p.set_suppress_nst(true);
    p.set_no_speech_thold(0.2);
    p.set_no_context(true);
    if let Some(prompt) = prompt {
        p.set_initial_prompt(prompt);
    }
    p
}

/// Whisper through whisper-rs directly. transcribe-rs's `WhisperEngine` has
/// no prompt in its `SpeechModel` impl (#77) and hardcodes a beam of 3 (#101).
///
/// Greedy by default. On invented TTS clips over room tone (#101), turbo at
/// beam 3 made words out of room tone 47 times in 165 against greedy's 6,
/// took 25% longer, and kept a quiet clause no more often (81 in 132 both).
/// egyptian-small invents words from room tone either way (133 in 165);
/// greedy only gets there 3x sooner.
///
/// One `WhisperState` per load, as transcribe-rs had. A fresh one per take
/// would make every decode reproducible, but it measured ~320 ms slower a
/// take (it allocates ~450 MB of GPU buffers). whisper.cpp seeds decoder 0's
/// RNG once per state; beam search draws from it on every token (turbo: 3
/// repeats differed on 43 of 100 clips), greedy only after a temperature
/// fallback (turbo: 2 of 100, "." against "..." on room tone).
///
/// Don't reach for `no_speech_thold` when Whisper drops a quiet clause (#69):
/// this whisper.cpp reads the no-speech probability after the last prompt
/// token instead of at SOT, so it measured 0.000 on every window, digital
/// silence included, and 0.2 / 0.6 / 1.0 decode identically. The drop is the
/// decode itself; `suppress_blank` and `suppress_nst` don't move it.
struct PromptedWhisper {
    /// Keeps its context (the loaded model) alive.
    state: WhisperState,
    prompt: Option<String>,
    beam: u32,
}

impl SpeechModel for PromptedWhisper {
    fn capabilities(&self) -> ModelCapabilities {
        // Nothing in Sotto reads these.
        ModelCapabilities {
            name: "Whisper",
            engine_id: "whisper_cpp",
            sample_rate: 16_000,
            languages: &[], // any
            supports_timestamps: false,
            supports_translation: false,
            supports_streaming: false,
        }
    }

    fn transcribe_raw(
        &mut self,
        samples: &[f32],
        options: &TranscribeOptions,
    ) -> Result<TranscriptionResult, TranscribeError> {
        let err = |e: whisper_rs::WhisperError| TranscribeError::Inference(e.to_string());
        let params = full_params(self.beam, options.language.as_deref(), options.translate, self.prompt.as_deref());
        self.state.full(params, samples).map_err(err)?;
        let mut bytes = Vec::new();
        for segment in self.state.as_iter() {
            bytes.extend_from_slice(segment.to_bytes().map_err(err)?);
        }
        Ok(TranscriptionResult { text: segments_text(&bytes), segments: None })
    }
}

/// The take's text from its segments' raw bytes, decoded once. whisper.cpp
/// cuts segments at token boundaries, and a token can end mid-character, so a
/// multi-byte character (any Arabic letter) can straddle two segments.
/// Decoding each segment on its own failed the whole take on the first such
/// split (transcribe-rs's behaviour: 32 of 165 egyptian-small takes errored);
/// joined first, the character is whole again. Bytes that are invalid even
/// then become U+FFFD rather than losing the take.
fn segments_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_string()
}

impl Asr {
    pub fn new() -> Self {
        let cfg = config::Config::load_or_init().unwrap_or_default();
        // Parakeet ignores `language` entirely (see `transcribe`'s comment),
        // so a non-"auto" value here is invisible until the day this engine
        // switches to a Whisper model, which DOES respect it — a landmine
        // left over from testing a different engine, not a live problem
        // today. Flag it in the log rather than silently clearing it: it
        // might be deliberate prep for that future switch.
        if cfg.asr.model == "parakeet-v3" && cfg.asr.language != "auto" {
            tracing::warn!(
                language = %cfg.asr.language,
                "asr.language is set but the active engine (Parakeet) ignores it — \
                 it will take effect the moment the engine switches to Whisper"
            );
        }
        let mut asr = Self {
            model: None,
            engine: cfg.asr.model.clone(),
            language: None,
            prompt: None,
            idle_unload_secs: 0,
            whisper_beam: 0,
        };
        asr.sync(&cfg);
        asr
    }

    /// Pick up the ASR settings from `cfg`. Call it only between takes: an
    /// engine switch drops the loaded model (#10), and the next `preload` or
    /// `transcribe` loads the new one. Language applies from the next call.
    pub fn sync(&mut self, cfg: &config::Config) {
        let next = next_engine(&self.engine, &cfg.asr.model, config::asr_model_present);
        if next != self.engine {
            tracing::info!(from = %self.engine, to = %next, "ASR engine switched");
            self.engine = next.to_string();
            self.model = None;
        }
        self.language = to_language_option(&self.engine, &cfg.asr.language);
        self.prompt = cfg.asr.vocabulary_prompt.then(|| vocab_prompt(&cfg.polish.vocabulary)).flatten();
        self.idle_unload_secs = cfg.asr.idle_unload_secs;
        self.whisper_beam = cfg.asr.whisper_beam;
    }

    /// The engine the next transcript comes from.
    pub fn engine(&self) -> &str {
        &self.engine
    }

    /// How long the owner may wait for work before calling `unload`.
    pub fn idle_wait(&self) -> Duration {
        idle_wait(self.model.is_some(), self.idle_unload_secs)
    }

    /// `Ready` with the model in memory, else `Idle`.
    pub fn load(&self) -> Load {
        if self.model.is_some() { Load::Ready } else { Load::Idle }
    }

    /// Whether `preload` has a model to load.
    pub fn wants_load(&self) -> bool {
        wants_load(self.model.is_some(), config::asr_model_present(&self.engine))
    }

    /// Free the model's memory (#12). The next `preload` or `transcribe`
    /// loads it again.
    pub fn unload(&mut self) {
        if self.model.take().is_some() {
            tracing::info!(engine = %self.engine, "ASR model unloaded (idle)");
        }
    }

    fn ensure_loaded(&mut self) -> anyhow::Result<&mut dyn SpeechModel> {
        if self.model.is_none() {
            let t = std::time::Instant::now();
            // Every whisper-family engine (turbo, egyptian-small, …) loads the
            // same way and differs only in which .bin — so branch on "is this a
            // whisper model" rather than on a specific id.
            let whisper = config::whisper_model_file(&self.engine).is_some();
            let mut model: Box<dyn SpeechModel> = if whisper {
                let path = config::whisper_model_path(&self.engine);
                anyhow::ensure!(
                    path.exists(),
                    "Whisper model not found at {} — download it first",
                    path.display()
                );
                // flash_attn stays off (transcribe-rs's `WhisperEngine::load()`
                // defaults it on): on the AMD iGPU (Vulkan0) flash attention has no
                // fast kernel and silently falls back to a slow path: measured
                // 6.5x slower on identical audio (23.1s vs 3.5s for 7.5s of
                // speech). It is not a correctness issue — the transcript is
                // identical — which is exactly why it would never have been
                // noticed except by timing it. Caveat (#14): that was measured
                // on the iGPU, but the auto-selected device below is the RTX
                // 3050, where flash_attn measured ~1.6x *faster* (445 vs 711 ms
                // warm on 7.7s). Turning it on needs a per-device rule, not a
                // flip.
                //
                // use_gpu stays true: when no Vulkan device is usable,
                // whisper.cpp reports "no devices found" and falls back to CPU
                // on its own, so this degrades rather than fails.
                let mut params = WhisperContextParameters::default();
                // transcribe-rs's auto-select picks a dedicated GPU first, then
                // most VRAM (the RTX 3050 here, not whisper.cpp's own default
                // of device 0, the iGPU).
                params.use_gpu(true).flash_attn(false).gpu_device(auto_select_gpu_device());
                let ctx = WhisperContext::new_with_params(&path, params).context("loading Whisper model")?;
                Box::new(PromptedWhisper {
                    state: ctx.create_state().context("creating Whisper state")?,
                    prompt: self.prompt.clone(),
                    beam: self.whisper_beam,
                })
            } else {
                let dir = config::model_dir();
                let encoder = dir.join("encoder-model.int8.onnx");
                anyhow::ensure!(
                    encoder.exists(),
                    "Parakeet model not found at {} — download it first",
                    dir.display()
                );
                Box::new(
                    ParakeetModel::load(&dir, &Quantization::Int8)
                        .context("loading Parakeet model")?,
                )
            };
            tracing::info!(
                load_ms = t.elapsed().as_millis(),
                engine = %self.engine,
                "ASR model loaded"
            );
            // Pay Whisper's first-inference cost here, not on the first
            // dictation. ggml-vulkan compiles its GPU pipelines lazily on first
            // use; the GPU driver caches them on disk, but when that cache is
            // cold (e.g. the first run under a new exe name) the first
            // transcription took 44-80 s on the RTX 3050. With it warm, the
            // first call in a process is still ~0.2 s slower than the rest
            // (#14). 2 s of silence runs the same encoder + decoder kernels
            // (whisper.cpp skips input under 1 s); the text is discarded,
            // never delivered.
            if whisper {
                let t = std::time::Instant::now();
                let _ = model.transcribe(&vec![0.0; 32_000], &self.options());
                tracing::info!(
                    warmup_ms = t.elapsed().as_millis() as u64,
                    // Length only: the words are the user's.
                    prompt_bytes = self.prompt.as_ref().map_or(0, |p| p.len()),
                    beam = self.whisper_beam,
                    "ASR warmed up"
                );
            }
            self.model = Some(model);
        }
        Ok(self.model.as_deref_mut().unwrap())
    }

    /// Load the model now instead of on the first dictation. It costs ~5s, and
    /// paying that *after* the user has already spoken is the worst-feeling
    /// delay in the app. Called on the worker thread at startup, when an
    /// engine is picked or its download lands (#118), and at every hotkey
    /// press (a no-op while loaded); a failure here is fine and
    /// silent — the model may simply not be downloaded yet, and `transcribe`
    /// will retry lazily.
    pub fn preload(&mut self) {
        if let Err(err) = self.ensure_loaded() {
            tracing::info!(%err, "ASR preload skipped — will load on first use");
        }
    }

    fn options(&self) -> TranscribeOptions {
        TranscribeOptions {
            language: self.language.clone(),
            ..Default::default()
        }
    }

    /// Transcribe 16 kHz mono f32 samples into trimmed text.
    pub fn transcribe(&mut self, samples: &[f32]) -> anyhow::Result<String> {
        let options = self.options();
        let model = self.ensure_loaded()?;
        // `SpeechModel::transcribe` (not `transcribe_raw`) so Parakeet still
        // gets its 250 ms leading-silence padding via `default_leading_silence_ms`;
        // Whisper defaults to none. Parakeet's `transcribe_raw` ignores
        // `options.language` entirely (English-only), so passing it through
        // unconditionally is safe for both engines.
        let t = std::time::Instant::now();
        let result = model
            .transcribe(samples, &options)
            .context("transcription failed")?;
        // Inference time ONLY (model load is timed separately in ensure_loaded).
        // audio_ms lets us read it as a real-time factor — the number that
        // actually answers "why does this feel slow" per engine and per model.
        let audio_ms = samples.len() as u64 * 1000 / 16_000;
        tracing::info!(
            transcribe_ms = t.elapsed().as_millis() as u64,
            audio_ms,
            engine = %self.engine,
            "transcribed"
        );
        *LAST_ENGINE.lock().unwrap() = self.engine.clone();
        Ok(collapse_loops(result.text.trim()))
    }
}

/// Two transcript words are the same once case, edge punctuation and Arabic
/// spelling (`polish::fold`, #115) are ignored ("The" / "the,", "أيه" /
/// "ايه"). Punctuation-only tokens never match.
pub fn same_word(a: &str, b: &str) -> bool {
    fn bare(w: &str) -> String {
        let w = w.trim_matches(|c: char| !c.is_alphanumeric());
        w.chars().filter_map(crate::polish::fold).map(|c| c.to_ascii_lowercase()).collect()
    }
    let a = bare(a);
    !a.is_empty() && a == bare(b)
}

/// A group of 1–3 words repeated this many times in a row is a decoding loop
/// ("to to to to …", seen on a chunk in #64), not speech. People do say "no,
/// no, no"; nobody dictates the same word five times running.
const LOOP_MIN: usize = 5;

/// Collapse each decoding loop to one copy, the last, so a full stop that
/// ends the loop survives. Text without a loop comes back untouched, and so
/// do digits: "5 5 5 5 5" is a code someone read out.
fn collapse_loops(text: &str) -> String {
    let w: Vec<&str> = text.split_whitespace().collect();
    let mut out = Vec::with_capacity(w.len());
    let mut i = 0;
    while i < w.len() {
        let mut n = 1;
        let digits = w[i].chars().any(|c| c.is_ascii_digit());
        for len in (1..=3).filter(|_| !digits) {
            let reps = 1 + (1..)
                .take_while(|&r| i + (r + 1) * len <= w.len() && (0..len).all(|k| same_word(w[i + k], w[i + r * len + k])))
                .count();
            if reps >= LOOP_MIN {
                tracing::info!(words = len, reps, "collapsed a repetition loop");
                i += (reps - 1) * len;
                n = len;
                break;
            }
        }
        out.extend_from_slice(&w[i..i + n]);
        i += n;
    }
    if out.len() == w.len() { text.to_string() } else { out.join(" ") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_language_maps_to_none() {
        assert_eq!(to_language_option("whisper-turbo", "auto"), None);
        assert_eq!(to_language_option("whisper-turbo", "Auto"), None); // config value isn't case-sensitive
    }

    #[test]
    fn explicit_language_passes_through() {
        assert_eq!(to_language_option("whisper-turbo", "en"), Some("en".to_string()));
        assert_eq!(to_language_option("whisper-turbo", "ar"), Some("ar".to_string()));
    }

    #[test]
    fn egyptian_small_always_listens_for_arabic() {
        for lang in ["auto", "en", "ar", "fr"] {
            assert_eq!(to_language_option("egyptian-small", lang).as_deref(), Some("ar"), "{lang}");
        }
        // Only while it is the engine that runs: the stored choice is kept.
        let mut asr = Asr {
            model: None,
            engine: "egyptian-small".into(),
            language: None,
            prompt: None,
            idle_unload_secs: 0,
            whisper_beam: 0,
        };
        let mut cfg = config::Config::default();
        cfg.asr.model = "egyptian-small".into();
        cfg.asr.language = "auto".into();
        asr.sync(&cfg);
        assert_eq!(asr.language.as_deref(), Some("ar"));
        assert_eq!(cfg.asr.language, "auto");
        // While its download is still landing, the engine that runs keeps the choice.
        assert_eq!(to_language_option(next_engine("whisper-turbo", "egyptian-small", |e| e == "whisper-turbo"), "auto"), None);
    }

    fn entry(word: &str, heard_as: usize, misses: usize) -> VocabEntry {
        VocabEntry {
            word: word.into(),
            heard_as: vec!["x".into(); heard_as],
            recent: vec![false; misses],
        }
    }

    #[test]
    fn no_words_means_no_prompt() {
        assert_eq!(vocab_prompt(&[]), None);
        assert_eq!(vocab_prompt(&[entry("  ", 3, 0)]), None);
    }

    #[test]
    fn misheard_words_lead_and_ties_keep_list_order() {
        let vocab = [entry("Sotto", 0, 0), entry("Zorvex", 1, 2), entry("Kestrel", 0, 0), entry("Qwen", 2, 0)];
        assert_eq!(vocab_prompt(&vocab).unwrap(), "Zorvex, Qwen, Sotto, Kestrel.");
    }

    #[test]
    fn caps_the_prompt_keeping_the_most_relevant_words() {
        // 60 filler words of 9 bytes each, far over the cap, with the one
        // misheard word last in the list.
        let mut vocab: Vec<VocabEntry> = (0..60).map(|i| entry(&format!("Filler{i:03}"), 0, 0)).collect();
        vocab.push(entry("Zorvex", 0, 1));
        let prompt = vocab_prompt(&vocab).unwrap();
        assert!(prompt.len() <= PROMPT_MAX_BYTES, "{} bytes", prompt.len());
        assert!(prompt.starts_with("Zorvex, Filler000, "));
        assert!(prompt.ends_with('.'));
        assert!(!prompt.contains("Filler059")); // the least relevant fell off
    }

    #[test]
    fn an_overlong_word_is_skipped_not_cut() {
        let long = "L".repeat(PROMPT_MAX_BYTES);
        let vocab = [entry(&long, 5, 0), entry("Zorvex", 0, 0)];
        assert_eq!(vocab_prompt(&vocab).unwrap(), "Zorvex.");
    }

    #[test]
    fn repetition_loops_collapse_to_one_copy() {
        assert_eq!(collapse_loops("I want to to to to to to to to go."), "I want to go.");
        assert_eq!(collapse_loops("thank you thank you thank you thank you thank you."), "thank you.");
        assert_eq!(collapse_loops("so so so so so so"), "so");
    }

    #[test]
    fn ordinary_repeats_are_speech_not_loops() {
        // Emphasis and stutters stay exactly as heard.
        for s in ["No, no, no, not that one.", "very very very very good", "It is what it is.", "PIN 5 5 5 5 5 1", ""] {
            assert_eq!(collapse_loops(s), s);
        }
    }

    #[test]
    fn same_word_ignores_case_and_edge_punctuation() {
        assert!(same_word("The", "the,"));
        assert!(same_word("again.", "again"));
        assert!(!same_word("—", "—"));
        assert!(!same_word("their", "there"));
    }

    #[test]
    fn a_picked_engine_takes_over_once_its_files_are_on_disk() {
        let both = |_: &str| true;
        assert_eq!(next_engine("parakeet-v3", "whisper-turbo", both), "whisper-turbo");
        assert_eq!(next_engine("parakeet-v3", "parakeet-v3", both), "parakeet-v3");
        // Download click: the new engine is selected before it lands, so the
        // old one keeps transcribing.
        let old_only = |e: &str| e == "parakeet-v3";
        assert_eq!(next_engine("parakeet-v3", "whisper-turbo", old_only), "parakeet-v3");
        // First run, nothing on disk: follow the config, so the "still
        // downloading" check asks about the engine being downloaded.
        let none = |_: &str| false;
        assert_eq!(next_engine("parakeet-v3", "whisper-turbo", none), "whisper-turbo");
    }

    #[test]
    fn whisper_decodes_greedy_at_beam_0_and_keeps_the_other_settings_either_way() {
        // FullParams has no getters; its Debug prints whisper.cpp's struct
        // (strategy 0 = greedy, 1 = beam search).
        let p = |beam| format!("{:?}", full_params(beam, Some("en"), false, None));
        let (greedy, beam3) = (p(0), p(3));
        assert!(greedy.contains("strategy: 0") && greedy.contains("best_of: 1"), "{greedy}");
        assert!(beam3.contains("strategy: 1") && beam3.contains("beam_size: 3"), "{beam3}");
        assert!(p(20).contains("beam_size: 8"), "past 8 whisper.cpp fails the decode");
        for s in [&greedy, &beam3] {
            for kept in ["no_context: true", "suppress_blank: true", "suppress_nst: true", "no_speech_thold: 0.2", "translate: false", "print_progress: false", "print_timestamps: false"] {
                assert!(s.contains(kept), "{kept} missing: {s}");
            }
        }
    }

    #[test]
    fn idle_unload_waits_only_while_a_model_is_loaded() {
        assert_eq!(idle_wait(true, 300), Duration::from_secs(300));
        assert_eq!(idle_wait(false, 300), Duration::MAX); // nothing to free
        assert_eq!(idle_wait(true, 0), Duration::MAX); // 0 = off
    }

    /// Stands in for a loaded model; `sync` and `unload` never call it.
    struct Loaded;

    impl SpeechModel for Loaded {
        fn capabilities(&self) -> ModelCapabilities {
            unimplemented!()
        }
        fn transcribe_raw(&mut self, _: &[f32], _: &TranscribeOptions) -> Result<TranscriptionResult, TranscribeError> {
            unimplemented!()
        }
    }

    #[test]
    fn a_character_split_across_segments_survives() {
        // One segment ends mid-character and the next begins with the rest
        // of it: each half alone is invalid UTF-8.
        let whole = " كشري مع صلصة ".as_bytes();
        let cut = 4; // inside the second Arabic letter (2 bytes each, after the space)
        assert!(std::str::from_utf8(&whole[..cut]).is_err() && std::str::from_utf8(&whole[cut..]).is_err());
        let joined: Vec<u8> = [&whole[..cut], &whole[cut..]].concat();
        assert_eq!(segments_text(&joined), "كشري مع صلصة");
        // Truly broken bytes don't fail the take either.
        assert_eq!(segments_text(b"ok \xFF done"), "ok \u{FFFD} done");
    }

    #[test]
    fn a_settings_change_without_an_engine_switch_keeps_the_model_then_idle_frees_it() {
        let mut asr = Asr {
            model: Some(Box::new(Loaded)),
            engine: "whisper-turbo".into(),
            language: None,
            prompt: None,
            idle_unload_secs: 0,
            whisper_beam: 0,
        };
        let mut cfg = config::Config::default();
        cfg.asr.model = "whisper-turbo".into();
        cfg.asr.language = "ar".into();
        cfg.asr.idle_unload_secs = 60;
        cfg.asr.whisper_beam = 3;
        cfg.polish.vocabulary = vec![entry("Zorvex", 0, 0)];
        asr.sync(&cfg);
        assert!(asr.model.is_some(), "no reload for language, words or beam");
        assert_eq!(asr.language.as_deref(), Some("ar"));
        assert_eq!(asr.prompt.as_deref(), Some("Zorvex.")); // used at the next load
        assert_eq!(asr.whisper_beam, 3); // likewise
        assert_eq!(asr.idle_wait(), Duration::from_secs(60));
        assert_eq!(asr.load(), Load::Ready);
        assert!(!asr.wants_load(), "in memory: a pick or a hotkey press reloads nothing");
        asr.unload();
        assert!(asr.model.is_none());
        assert_eq!(asr.idle_wait(), Duration::MAX);
        assert_eq!(asr.load(), Load::Idle, "Settings stops saying Ready (#118)");
    }

    #[test]
    fn a_picked_engine_loads_once_its_files_are_on_disk() {
        assert!(wants_load(false, true), "picked and on disk: load it now");
        assert!(!wants_load(false, false), "still downloading: nothing to load, nothing to say");
        assert!(!wants_load(true, true), "already in memory");
    }

    #[test]
    fn settings_hears_each_change_of_load_state_once() {
        let told = |state| set_load("whisper-turbo", state).map(|s| serde_json::to_string(&s).unwrap());
        assert_eq!(told(Load::Loading).as_deref(), Some(r#"{"engine":"whisper-turbo","state":"loading"}"#));
        assert_eq!(told(Load::Loading), None, "no news");
        assert_eq!(told(Load::Ready).as_deref(), Some(r#"{"engine":"whisper-turbo","state":"ready"}"#));
        // A window built now still learns it (#13).
        assert_eq!(load_state(), Some(LoadState { engine: "whisper-turbo".into(), state: Load::Ready }));
        // Another engine's model is news even in the same state.
        assert!(set_load("parakeet-v3", Load::Ready).is_some());
        assert_eq!(told(Load::Idle).as_deref(), Some(r#"{"engine":"whisper-turbo","state":"idle"}"#));
    }
}
