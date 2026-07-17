//! Throwaway probe: is transcribe-rs's hardcoded BeamSearch{patience:-1.0} the
//! reason Whisper never finishes? Times greedy vs that exact beam config on the
//! same model + audio, driving whisper-rs directly.
//!
//!   cargo run --release --example whisper_probe -- <model.bin> <audio.wav>

use std::time::Instant;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

fn main() {
    let mut a = std::env::args().skip(1);
    let model = a.next().expect("model path");
    let wav = a.next().expect("wav path");

    let samples = transcribe_rs::audio::read_wav_samples(std::path::Path::new(&wav)).unwrap();
    println!("audio: {} samples ({:.1}s)", samples.len(), samples.len() as f32 / 16000.0);

    let t = Instant::now();
    let ctx = WhisperContext::new_with_params(&model, WhisperContextParameters::default()).unwrap();
    println!("model loaded in {} ms\n", t.elapsed().as_millis());

    let cases: Vec<(&str, SamplingStrategy)> = vec![
        ("greedy(best_of=1)", SamplingStrategy::Greedy { best_of: 1 }),
        // Exactly what transcribe-rs hardcodes:
        ("beam(size=3, patience=-1.0)", SamplingStrategy::BeamSearch { beam_size: 3, patience: -1.0 }),
        // Same beam, legal patience — isolates patience as the culprit:
        ("beam(size=3, patience=1.0)", SamplingStrategy::BeamSearch { beam_size: 3, patience: 1.0 }),
    ];

    for (name, strat) in cases {
        let mut state = ctx.create_state().unwrap();
        let mut p = FullParams::new(strat);
        p.set_language(Some("en"));
        p.set_n_threads(8);
        p.set_print_special(false);
        p.set_print_progress(false);
        p.set_print_realtime(false);
        p.set_print_timestamps(false);

        println!("-- {name}: running (60s budget) ...");
        let t = Instant::now();
        match state.full(p, &samples) {
            Ok(_) => {
                let ms = t.elapsed().as_millis();
                let n = state.full_n_segments();
                let mut text = String::new();
                for i in 0..n {
                    if let Some(s) = state.get_segment(i) {
                        text.push_str(s.to_str().unwrap_or(""));
                    }
                }
                println!("   {name}: {ms} ms => {:?}\n", text.trim());
            }
            Err(e) => println!("   {name}: ERROR {e:?}\n"),
        }
    }
}
