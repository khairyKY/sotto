//! Speech-to-text via `transcribe-rs`, switchable between Parakeet TDT 0.6b v3
//! (int8, English-only, fast) and Whisper large-v3-turbo (multilingual)
//! depending on `config::AsrConfig::model`.
//!
//! The model (hundreds of MB) is loaded lazily on the first dictation so
//! startup stays instant and idle memory stays near zero. ONNX Runtime (used
//! by Parakeet) is loaded dynamically at runtime from `onnxruntime.dll` (see
//! `main::init_ort`).

use crate::config;
use anyhow::Context;
use transcribe_rs::onnx::parakeet::ParakeetModel;
use transcribe_rs::onnx::Quantization;
use transcribe_rs::whisper_cpp::{WhisperEngine, WhisperLoadParams};
use transcribe_rs::{SpeechModel, TranscribeOptions};

pub struct Asr {
    model: Option<Box<dyn SpeechModel>>,
    /// `config::AsrConfig::model` at construction time, e.g. "parakeet-v3" or
    /// "whisper-turbo". Switching engines needs a fresh `Asr` (app restart),
    /// same as any other model-affecting setting.
    engine: String,
    /// `None` = auto-detect (config `language = "auto"`).
    language: Option<String>,
}

/// Maps the config's language string to what `TranscribeOptions` expects.
fn to_language_option(lang: &str) -> Option<String> {
    if lang.eq_ignore_ascii_case("auto") {
        None
    } else {
        Some(lang.to_string())
    }
}

impl Asr {
    pub fn new() -> Self {
        let cfg = config::Config::load_or_init().unwrap_or_default();
        Self {
            model: None,
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
            let model: Box<dyn SpeechModel> = if config::whisper_model_file(&self.engine).is_some() {
                let path = config::whisper_model_path(&self.engine);
                anyhow::ensure!(
                    path.exists(),
                    "Whisper model not found at {} — download it first",
                    path.display()
                );
                // NOT `WhisperEngine::load()` — that defaults flash_attn to
                // true, and on this Vulkan backend flash attention has no fast
                // kernel and silently falls back to a slow path: measured 6.5x
                // slower on identical audio (23.1s vs 3.5s for 7.5s of speech).
                // It is not a correctness issue — the transcript is identical —
                // which is exactly why it would never have been noticed except
                // by timing it.
                //
                // use_gpu stays true: when no Vulkan device is usable,
                // whisper.cpp reports "no devices found" and falls back to CPU
                // on its own, so this degrades rather than fails.
                let params = WhisperLoadParams {
                    use_gpu: true,
                    flash_attn: false,
                    // -1 = let whisper.cpp pick the device (GPU_DEVICE_AUTO).
                    gpu_device: -1,
                };
                Box::new(
                    WhisperEngine::load_with_params(&path, params)
                        .context("loading Whisper model")?,
                )
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

    /// Transcribe 16 kHz mono f32 samples into trimmed text.
    pub fn transcribe(&mut self, samples: &[f32]) -> anyhow::Result<String> {
        let options = TranscribeOptions {
            language: self.language.clone(),
            ..Default::default()
        };
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
        Ok(result.text.trim().to_string())
    }
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
}
