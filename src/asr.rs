//! Speech-to-text via `transcribe-rs`, switchable between Parakeet TDT 0.6b v3
//! (int8, English-only, fast) and Whisper large-v3-turbo (multilingual)
//! depending on `config::AsrConfig::model`.
//!
//! The model (hundreds of MB) is loaded lazily on the first dictation so
//! startup stays instant and idle memory stays near zero. ONNX Runtime (used
//! by Parakeet) is loaded dynamically at runtime from `onnxruntime.dll` (see
//! `main::init_ort`).

use crate::config::{self, VocabEntry};
use anyhow::Context;
use transcribe_rs::onnx::parakeet::ParakeetModel;
use transcribe_rs::onnx::Quantization;
use transcribe_rs::whisper_cpp::{WhisperEngine, WhisperInferenceParams, WhisperLoadParams};
use transcribe_rs::{
    ModelCapabilities, SpeechModel, TranscribeError, TranscribeOptions, TranscriptionResult,
};

pub struct Asr {
    model: Option<Box<dyn SpeechModel>>,
    /// `config::AsrConfig::model` at construction time, e.g. "parakeet-v3" or
    /// "whisper-turbo". Switching engines needs a fresh `Asr` (app restart),
    /// same as any other model-affecting setting.
    engine: String,
    /// `None` = auto-detect (config `language = "auto"`).
    language: Option<String>,
    /// Whisper's `initial_prompt`, built once from the trained vocabulary.
    /// ponytail: read at construction like `engine`, so a word trained after
    /// startup joins the prompt on the next restart. Pass the live
    /// `Controls::vocabulary` in if that lag ever matters.
    prompt: Option<String>,
}

/// Maps the config's language string to what `TranscribeOptions` expects.
fn to_language_option(lang: &str) -> Option<String> {
    if lang.eq_ignore_ascii_case("auto") {
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

/// transcribe-rs's `SpeechModel` impl for Whisper has no prompt, so wrap the
/// engine and go through `transcribe_with`, which has one. Everything else
/// is `WhisperInferenceParams::default()`, i.e. what the plain impl sends.
struct PromptedWhisper {
    engine: WhisperEngine,
    prompt: Option<String>,
}

impl SpeechModel for PromptedWhisper {
    fn capabilities(&self) -> ModelCapabilities {
        self.engine.capabilities()
    }

    fn transcribe_raw(
        &mut self,
        samples: &[f32],
        options: &TranscribeOptions,
    ) -> Result<TranscriptionResult, TranscribeError> {
        let params = WhisperInferenceParams {
            language: options.language.clone(),
            translate: options.translate,
            initial_prompt: self.prompt.clone(),
            ..Default::default()
        };
        self.engine.transcribe_with(samples, &params)
    }
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
        Self {
            model: None,
            prompt: cfg.asr.vocabulary_prompt.then(|| vocab_prompt(&cfg.polish.vocabulary)).flatten(),
            engine: cfg.asr.model,
            language: to_language_option(&cfg.asr.language),
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
                // NOT `WhisperEngine::load()` — that defaults flash_attn to
                // true, and on the AMD iGPU (Vulkan0) flash attention has no
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
                let params = WhisperLoadParams {
                    use_gpu: true,
                    flash_attn: false,
                    // -1 = GPU_DEVICE_AUTO: transcribe-rs picks a dedicated GPU
                    // first, then most VRAM (the RTX 3050 here, not whisper.cpp's
                    // own default of device 0, the iGPU).
                    gpu_device: -1,
                };
                Box::new(PromptedWhisper {
                    engine: WhisperEngine::load_with_params(&path, params)
                        .context("loading Whisper model")?,
                    prompt: self.prompt.clone(),
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
                    "ASR warmed up"
                );
            }
            self.model = Some(model);
        }
        Ok(self.model.as_deref_mut().unwrap())
    }

    /// Load the model now instead of on the first dictation. It costs ~5s, and
    /// paying that *after* the user has already spoken is the worst-feeling
    /// delay in the app. Called on the worker thread at startup; a failure here
    /// is fine and silent — the model may simply not be downloaded yet, and
    /// `transcribe` will retry lazily.
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
        Ok(collapse_loops(result.text.trim()))
    }
}

/// Two transcript words are the same once case and edge punctuation are
/// ignored ("The" / "the,"). Punctuation-only tokens never match.
pub fn same_word(a: &str, b: &str) -> bool {
    fn bare(w: &str) -> &str {
        w.trim_matches(|c: char| !c.is_alphanumeric())
    }
    !bare(a).is_empty() && bare(a).eq_ignore_ascii_case(bare(b))
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
        assert_eq!(to_language_option("auto"), None);
        assert_eq!(to_language_option("Auto"), None); // config value isn't case-sensitive
    }

    #[test]
    fn explicit_language_passes_through() {
        assert_eq!(to_language_option("en"), Some("en".to_string()));
        assert_eq!(to_language_option("ar"), Some("ar".to_string()));
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
}
