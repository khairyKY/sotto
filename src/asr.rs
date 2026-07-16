//! Speech-to-text via Parakeet TDT 0.6b v3 (int8) through `transcribe-rs`.
//!
//! The model (~670 MB) is loaded lazily on the first dictation so startup
//! stays instant and idle memory stays near zero. ONNX Runtime is loaded
//! dynamically at runtime from `onnxruntime.dll` (see `main::init_ort`).

use crate::config;
use anyhow::Context;
use std::path::PathBuf;
use transcribe_rs::onnx::parakeet::{ParakeetModel, ParakeetParams};
use transcribe_rs::onnx::Quantization;

pub struct Asr {
    model: Option<ParakeetModel>,
    model_dir: PathBuf,
}

impl Asr {
    pub fn new() -> Self {
        Self {
            model: None,
            model_dir: config::model_dir(),
        }
    }

    fn ensure_loaded(&mut self) -> anyhow::Result<&mut ParakeetModel> {
        if self.model.is_none() {
            let encoder = self.model_dir.join("encoder-model.int8.onnx");
            anyhow::ensure!(
                encoder.exists(),
                "Parakeet model not found at {} — download it first",
                self.model_dir.display()
            );

            let t = std::time::Instant::now();
            let model = ParakeetModel::load(&self.model_dir, &Quantization::Int8)
                .context("loading Parakeet model")?;
            tracing::info!(load_ms = t.elapsed().as_millis(), "Parakeet model loaded");
            self.model = Some(model);
        }
        Ok(self.model.as_mut().unwrap())
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
        let model = self.ensure_loaded()?;
        let result = model
            .transcribe_with(samples, &ParakeetParams::default())
            .context("transcription failed")?;
        Ok(result.text.trim().to_string())
    }
}
