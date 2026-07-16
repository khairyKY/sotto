//! Soft dictation ticks — "recording started" / "recording stopped".
//!
//! The two blips are synthesized once at first use (a short sine with an
//! exponential decay, 22.05 kHz mono 16-bit WAV in memory) and played through
//! `PlaySoundW(SND_MEMORY | SND_ASYNC)`. No audio assets to bundle, no mixer
//! dependency — the OS handles playback. ponytail: PlaySound allows one async
//! sound at a time per process; start/stop ticks never overlap in practice
//! (recording is at least MIN_CLIP long), so that's fine.

use std::sync::OnceLock;
use windows::core::PCWSTR;
use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};

const RATE: u32 = 22_050;

/// Build a WAV byte buffer containing a `freq` Hz sine, `ms` long, with a fast
/// attack and exponential decay so it reads as a soft "tick", not a beep.
fn synth_wav(freq: f32, ms: u32, gain: f32) -> Vec<u8> {
    let n = (RATE * ms / 1000) as usize;
    let mut pcm = Vec::with_capacity(n * 2);
    for i in 0..n {
        let t = i as f32 / RATE as f32;
        // 2 ms linear attack, then exponential decay over the remainder.
        let attack = (t / 0.002).min(1.0);
        let decay = (-t * 55.0).exp();
        let s = (t * freq * std::f32::consts::TAU).sin() * attack * decay * gain;
        pcm.extend_from_slice(&((s * i16::MAX as f32) as i16).to_le_bytes());
    }
    let data_len = pcm.len() as u32;
    let mut wav = Vec::with_capacity(44 + pcm.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes()); // byte rate
    wav.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(&pcm);
    wav
}

fn play(wav: &'static [u8]) {
    // SND_MEMORY: the "sound name" pointer is actually the WAV bytes. The
    // buffer is 'static (OnceLock), so it outlives the async playback.
    unsafe {
        let _ = PlaySoundW(
            PCWSTR(wav.as_ptr() as *const u16),
            None,
            SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
        );
    }
}

/// Recording started — a bright, quiet tick.
pub fn tick() {
    static WAV: OnceLock<Vec<u8>> = OnceLock::new();
    play(WAV.get_or_init(|| synth_wav(880.0, 90, 0.22)));
}

/// Recording stopped — same tick a third lower, reads as "done".
pub fn tock() {
    static WAV: OnceLock<Vec<u8>> = OnceLock::new();
    play(WAV.get_or_init(|| synth_wav(620.0, 110, 0.22)));
}

#[cfg(test)]
mod tests {
    use super::synth_wav;

    #[test]
    fn wav_header_is_valid_and_sized() {
        let w = synth_wav(880.0, 90, 0.22);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(&w[8..12], b"WAVE");
        let data_len = u32::from_le_bytes(w[40..44].try_into().unwrap()) as usize;
        assert_eq!(w.len(), 44 + data_len);
        // 90 ms at 22050 Hz mono 16-bit PCM.
        assert_eq!(data_len, (22_050 * 90 / 1000) * 2);
    }
}
