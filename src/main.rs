mod asr;
mod audio;
mod config;
mod hotkey;
mod inject;
mod history;
mod job;
mod llm;
mod overlay;
mod polish;
mod settings;
mod single_instance;
mod startup;
mod tray;

use config::Config;
use hotkey::DictationEvent;
use single_instance::SingleInstanceGuard;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Runtime controls shared between the tray (and later the settings window),
/// the hotkey listener, and the polisher — the live, mutable knobs that aren't
/// captured at startup. Cheap to clone (just `Arc`s).
#[derive(Clone)]
pub struct Controls {
    /// When set, the hotkey listener ignores requests to *start* a dictation.
    pub paused: Arc<AtomicBool>,
    /// Live polish tier (`PolishMode` as u8), read per-dictation by the polisher.
    pub polish_mode: Arc<AtomicU8>,
    /// Live activation mode (`ActivationMode` as u8), read by the hotkey listener.
    pub activation: Arc<AtomicU8>,
    /// Live index into `hotkey::SUPPORTED_HOTKEYS`, read by the hotkey listener.
    pub hotkey_idx: Arc<AtomicUsize>,
    /// Live AI-tier word threshold, read by the polisher.
    pub ai_min_words: Arc<AtomicUsize>,
    /// Live dictionary/snippet replacements, applied by the polisher.
    pub dictionary: Arc<Mutex<Vec<(String, String)>>>,
    /// Recent dictations, appended by the worker, read by the settings window.
    pub history: history::History,
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
                cfg.dictionary
                    .iter()
                    .map(|e| (e.spoken.clone(), e.replacement.clone()))
                    .collect(),
            )),
            history: history::History::new(),
        }
    }
}

/// Ignore captured clips shorter than this — almost always an accidental tap
/// of the hotkey rather than real speech.
const MIN_CLIP_SAMPLES: usize = 16_000 / 2; // 0.5s at 16 kHz

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("SOTTO_LOG").unwrap_or_else(|_| "info".into()))
        .init();

    init_ort();

    // Dev/debug one-shot: `sotto --transcribe <file.wav>` runs the real ASR
    // path on a 16 kHz mono WAV and exits. Placed before the single-instance
    // guard so it works even while the tray app is already running.
    if let Some(path) = arg_value("--transcribe") {
        return run_transcribe_once(&path);
    }

    // Dev/debug one-shot: `sotto --polish "<raw text>"` runs the AI polish
    // path (spawns the llama.cpp sidecar, cleans the text) and exits.
    if let Some(text) = arg_value("--polish") {
        return run_polish_once(&text);
    }

    // Dev/debug: `sotto --overlay-demo` cycles the overlay pill through every
    // state with a synthetic mic level, so the visuals + animation can be
    // eyeballed without a mic, model, or hotkey. Runs the real overlay path.
    if std::env::args().any(|a| a == "--overlay-demo") {
        return run_overlay_demo();
    }

    // Dev/debug: `sotto --settings` opens the settings window immediately so it
    // can be eyeballed without a mic or model.
    if std::env::args().any(|a| a == "--settings") {
        let cfg = Config::load_or_init().unwrap_or_default();
        return overlay::Overlay::new(Controls::from_config(&cfg))
            .open_settings_on_start()
            .run()
            .map_err(|e| anyhow::anyhow!("settings preview failed: {e}"));
    }

    let Some(_guard) = SingleInstanceGuard::acquire()? else {
        tracing::warn!("another Sotto instance is already running — exiting");
        return Ok(());
    };

    let cfg = Config::load_or_init()?;
    tracing::info!(?cfg, path = %Config::path().display(), "loaded config");

    let (tx, rx) = crossbeam_channel::unbounded::<DictationEvent>();

    // Set for the duration of our own text injection. The paste fallback
    // synthesizes a real Ctrl+V via SendInput, which our own global hook
    // would otherwise see as "the user pressed Ctrl" whenever the hotkey is
    // itself a modifier key — silently re-triggering another dictation cycle.
    let suppressed = Arc::new(AtomicBool::new(false));

    // Live, mutable controls shared with the tray + settings window.
    let controls = Controls::from_config(&cfg);

    let listener_suppressed = suppressed.clone();
    let listener_hotkey_idx = controls.hotkey_idx.clone();
    let listener_activation = controls.activation.clone();
    let listener_paused = controls.paused.clone();
    std::thread::spawn(move || {
        hotkey::run_listener(
            listener_hotkey_idx,
            listener_activation,
            tx,
            listener_suppressed,
            listener_paused,
        );
    });

    // Dictation worker. Owns the microphone recorder and the ASR engine
    // (both `!Send` / thread-affine), so it creates them here and never lets
    // them cross threads.
    let injection_mode = cfg.injection_mode;
    let polisher = polish::Polisher::new(controls.clone(), cfg.llm.clone());

    // The overlay drives the on-screen pill; the worker publishes state to it
    // and the audio recorder feeds it live mic level. `overlay` itself runs the
    // event loop on the main thread at the end of `main`.
    let overlay = overlay::Overlay::new(controls.clone());
    let worker_ov = overlay.clone();
    let level_slot = overlay.level_slot();
    let worker_history = controls.history.clone();

    std::thread::spawn(move || {
        let mut recorder = audio::Recorder::new(level_slot);
        let mut asr = asr::Asr::new();

        for event in rx {
            match event {
                DictationEvent::Start => match recorder.start() {
                    Ok(()) => {
                        worker_ov.set(overlay::OverlayState::Listening);
                        tracing::info!("listening");
                    }
                    Err(err) => {
                        worker_ov.set(overlay::OverlayState::Error);
                        tracing::error!(?err, "failed to start capture");
                    }
                },
                DictationEvent::Stop => {
                    let samples = match recorder.stop() {
                        Ok(s) => s,
                        Err(err) => {
                            worker_ov.set(overlay::OverlayState::Error);
                            tracing::error!(?err, "failed to stop capture");
                            continue;
                        }
                    };
                    if samples.len() < MIN_CLIP_SAMPLES {
                        // Accidental tap — just hide the pill, no error treatment.
                        worker_ov.set(overlay::OverlayState::Idle);
                        tracing::info!(
                            secs = samples.len() as f32 / 16_000.0,
                            "clip too short — ignoring"
                        );
                        continue;
                    }

                    let audio_s = samples.len() as f32 / 16_000.0;
                    worker_ov.set(overlay::OverlayState::Transcribing);
                    let t_asr = Instant::now();
                    let text = match asr.transcribe(&samples) {
                        Ok(t) => t,
                        Err(err) => {
                            worker_ov.set(overlay::OverlayState::Error);
                            tracing::error!(?err, "transcription failed");
                            continue;
                        }
                    };
                    let asr_ms = t_asr.elapsed().as_millis();

                    if text.is_empty() {
                        worker_ov.set(overlay::OverlayState::Error);
                        tracing::info!(audio_s, asr_ms, "empty transcript");
                        continue;
                    }

                    if polisher.uses_ai_tier(&text) {
                        worker_ov.set(overlay::OverlayState::Polishing);
                    }
                    let t_polish = Instant::now();
                    let polished = polisher.polish(&text);
                    let polish_ms = t_polish.elapsed().as_millis();
                    if polished.is_empty() {
                        worker_ov.set(overlay::OverlayState::Error);
                        tracing::info!(audio_s, asr_ms, raw = %text, "nothing left after polish");
                        continue;
                    }

                    let t_inject = Instant::now();
                    suppressed.store(true, Ordering::SeqCst);
                    let result = inject::inject_text(&polished, injection_mode);
                    suppressed.store(false, Ordering::SeqCst);
                    match result {
                        Ok(()) => {
                            worker_ov.set(overlay::OverlayState::Done);
                            worker_history.push(polished.clone());
                            tracing::info!(
                                audio_s,
                                asr_ms,
                                polish_ms,
                                inject_ms = t_inject.elapsed().as_millis(),
                                raw = %text,
                                "injected: {polished:?}"
                            );
                        }
                        Err(err) => {
                            worker_ov.set(overlay::OverlayState::Error);
                            tracing::error!(?err, "injection failed");
                        }
                    }
                }
            }
        }
    });

    tracing::info!(hotkey = %cfg.hotkey, mode = ?cfg.activation_mode, "Sotto ready — hold hotkey and speak");
    overlay
        .run()
        .map_err(|e| anyhow::anyhow!("overlay/event loop failed: {e}"))
}

/// Returns the value following `flag` in the process args, if present.
fn arg_value(flag: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1).cloned())
}

/// Dev one-shot for the AI polish path: forces AI mode (any length) and prints
/// raw vs. polished with timing. Exercises the real `llm.rs` + `polish.rs`.
fn run_polish_once(raw: &str) -> anyhow::Result<()> {
    let cfg = Config::load_or_init().unwrap_or_default();
    let controls = Controls::from_config(&cfg);
    controls
        .polish_mode
        .store(config::PolishMode::Ai.as_u8(), Ordering::Relaxed);
    controls.ai_min_words.store(0, Ordering::Relaxed); // force the LLM regardless of length
    let polisher = polish::Polisher::new(controls, cfg.llm.clone());

    let t = Instant::now();
    let out = polisher.polish(raw);
    println!("raw      => {raw:?}");
    println!("polished => {out:?}  ({} ms)", t.elapsed().as_millis());
    Ok(())
}

/// Dev one-shot: cycle the overlay through every state with a synthetic mic
/// level so the pill + animations can be verified without a mic or model.
fn run_overlay_demo() -> anyhow::Result<()> {
    use overlay::OverlayState::*;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    let overlay = overlay::Overlay::new(Controls::from_config(&Config::default()));
    let ov = overlay.clone();
    let level = overlay.level_slot();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        loop {
            // Listening — feed a wobbling level for ~3.2s so the wave looks alive.
            ov.set(Listening);
            let start = Instant::now();
            while start.elapsed() < Duration::from_millis(3200) {
                let t = start.elapsed().as_secs_f32();
                let rms = 0.05 + 0.06 * (t * 6.0).sin().abs();
                level.store(rms.to_bits(), Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(60));
            }
            level.store(0f32.to_bits(), Ordering::Relaxed);
            ov.set(Transcribing);
            std::thread::sleep(Duration::from_millis(2600));
            ov.set(Polishing);
            std::thread::sleep(Duration::from_millis(3200));
            ov.set(Done); // auto-hides after ~0.8s
            std::thread::sleep(Duration::from_millis(1600));
            ov.set(Error); // auto-hides after ~2.7s
            std::thread::sleep(Duration::from_millis(3400));
        }
    });
    overlay
        .run()
        .map_err(|e| anyhow::anyhow!("overlay demo failed: {e}"))
}

fn run_transcribe_once(path: &str) -> anyhow::Result<()> {
    let samples = transcribe_rs::audio::read_wav_samples(std::path::Path::new(path))
        .map_err(|e| anyhow::anyhow!("failed to read {path}: {e}"))?;
    let mut asr = asr::Asr::new();
    let t = Instant::now();
    let text = asr.transcribe(&samples)?;
    println!(
        "({} samples, {} ms) => {text:?}",
        samples.len(),
        t.elapsed().as_millis()
    );
    Ok(())
}

/// Point `ort` (load-dynamic) at our bundled ONNX Runtime dll. Loading a
/// C-ABI dll at runtime works fine from a MinGW-built binary, which is why we
/// can stay on the GNU toolchain despite `ort` shipping no gnu prebuilts.
fn init_ort() {
    let dll = config::onnxruntime_dll();
    if dll.exists() {
        // SAFETY: single-threaded startup, before any worker thread is spawned.
        unsafe { std::env::set_var("ORT_DYLIB_PATH", &dll) };
        tracing::info!(path = %dll.display(), "ORT_DYLIB_PATH set");
    } else {
        tracing::warn!(
            path = %dll.display(),
            "onnxruntime.dll not found — dictation will fail until it and the model are installed"
        );
    }
}
