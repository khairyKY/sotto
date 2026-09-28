//! Whisper timing probe: one model + one WAV through whisper-rs directly, with
//! every knob transcribe-rs sets for the app exposed as an env var, so an
//! app-vs-probe gap can be closed one variable at a time (#14).
//!
//!   cargo run --release --example whisper_probe -- <model.bin> <audio.wav> [runs]
//!
//! Defaults reproduce what `sotto.exe --transcribe` does (flash_attn off, see
//! `asr.rs`). Knobs:
//!   PROBE_DEVICE   gpu_device; -1 (default) = transcribe-rs auto-select, as the
//!                  app does. The old probe used whisper-rs's default 0, the AMD
//!                  iGPU here, while the app ran on the RTX 3050 (#14).
//!   PROBE_BEAM     beam size, default 3; 0 = greedy
//!   PROBE_THREADS  0 (default) = whisper.cpp's min(4, cores)
//!   PROBE_LANG     default "en"; "auto" = detect
//!   PROBE_TR_PARAMS 1 (default) = transcribe-rs's suppress_nst=true,
//!                  no_speech_thold=0.2; 0 = whisper.cpp's own defaults
//!   PROBE_FLASH    0 (default, as the app) or 1 = flash_attn
//!
//! Every run reuses one state, like the app: run 1 is the cold one.

use std::time::Instant;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

fn knob(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn main() {
    let mut a = std::env::args().skip(1);
    let model = a.next().expect("model path");
    let wav = a.next().expect("wav path");
    let runs: usize = a.next().map_or(3, |r| r.parse().expect("runs"));

    let device: i32 = knob("PROBE_DEVICE", "-1").parse().unwrap();
    let beam: i32 = knob("PROBE_BEAM", "3").parse().unwrap();
    let threads: i32 = knob("PROBE_THREADS", "0").parse().unwrap();
    let lang = knob("PROBE_LANG", "en");
    let tr_params = knob("PROBE_TR_PARAMS", "1") == "1";
    let flash = knob("PROBE_FLASH", "0") == "1";

    let samples = transcribe_rs::audio::read_wav_samples(std::path::Path::new(&wav)).unwrap();
    let device = if device < 0 { transcribe_rs::whisper_cpp::gpu::auto_select_gpu_device() } else { device };
    println!(
        "audio {:.1}s | device={device} beam={beam} threads={threads} lang={lang} tr_params={tr_params} flash={flash}",
        samples.len() as f32 / 16000.0
    );

    let t = Instant::now();
    let mut cp = WhisperContextParameters::default();
    cp.flash_attn(flash).gpu_device(device);
    let ctx = WhisperContext::new_with_params(&model, cp).unwrap();
    let mut state = ctx.create_state().unwrap();
    println!("load_ms={}", t.elapsed().as_millis());

    for run in 1..=runs {
        let strat = if beam > 0 {
            SamplingStrategy::BeamSearch { beam_size: beam, patience: -1.0 }
        } else {
            SamplingStrategy::Greedy { best_of: 1 }
        };
        let mut p = FullParams::new(strat);
        p.set_language(if lang == "auto" { None } else { Some(&lang) });
        if threads > 0 {
            p.set_n_threads(threads);
        }
        if tr_params {
            p.set_suppress_nst(true);
            p.set_no_speech_thold(0.2);
        }
        p.set_print_special(false);
        p.set_print_progress(false);
        p.set_print_realtime(false);
        p.set_print_timestamps(false);

        let t = Instant::now();
        state.full(p, &samples).unwrap();
        let ms = t.elapsed().as_millis();
        let text: String = (0..state.full_n_segments())
            .filter_map(|i| state.get_segment(i))
            .map(|s| s.to_str().unwrap_or("").to_string())
            .collect();
        println!("run {run}: transcribe_ms={ms} => {:?}", text.trim());
    }
}
