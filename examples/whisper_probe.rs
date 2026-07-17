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

    // transcribe-rs's WhisperEngine::load() defaults flash_attn to TRUE; the
    // plain WhisperContextParameters::default() used here originally has it
    // FALSE. That one flag is the whole difference between the app taking 51s
    // and this probe taking 3.6s on the same file — so test both.
    for flash in [false, true] {
        let t = Instant::now();
        let mut cp = WhisperContextParameters::default();
        cp.flash_attn(flash);
        let ctx = WhisperContext::new_with_params(&model, cp).unwrap();
        println!("\n===== flash_attn = {flash} (model loaded in {} ms) =====", t.elapsed().as_millis());

        let cases: Vec<(&str, SamplingStrategy)> = vec![
            ("greedy(best_of=1)", SamplingStrategy::Greedy { best_of: 1 }),
            // Exactly what transcribe-rs hardcodes:
            ("beam(size=3, patience=-1.0)", SamplingStrategy::BeamSearch { beam_size: 3, patience: -1.0 }),
        ];

        for (name, strat) in cases {
            let mut state = ctx.create_state().unwrap();
            let mut p = FullParams::new(strat);
            p.set_language(Some("en"));
            // transcribe-rs leaves this at whisper's default (min(4, cores));
            // pass 0 to match it exactly rather than flatter ourselves with 8.
            p.set_n_threads(4);
            p.set_print_special(false);
            p.set_print_progress(false);
            p.set_print_realtime(false);
            p.set_print_timestamps(false);

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
                    println!("   flash={flash} {name}: {ms} ms => {:?}", text.trim());
                }
                Err(e) => println!("   flash={flash} {name}: ERROR {e:?}"),
            }
        }
    }
}
