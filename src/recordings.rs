//! Opt-in recording retention: audio + raw + polished text for each
//! delivered take, so raw ASR output <-> polished output <-> what actually
//! got typed can be compared later. OFF BY DEFAULT -- every other part of
//! Sotto is careful never to persist dictated content (see `stats.rs`'s own
//! "counts and timings only" rule); this is the one deliberate exception,
//! gated behind its own toggle, capped by total size with oldest evicted
//! first.
//!
//! Stored under `assets_dir()/recordings/`, not `data_dir()` -- so pointing
//! `assets_dir` at a roomier drive (as the models already do) keeps this off
//! the system drive too. One 8-bit mu-law WAV per take (~0.96 MB/min, half
//! of 16-bit PCM, zero extra dependencies, opens in any player unmodified)
//! plus one JSONL line per take in `index.jsonl`, mirroring `stats.rs`'s
//! append-only pattern.

use crate::config;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RecordingEntry {
    /// Unix seconds (UTC) -- also the wav filename's stem, and what oldest-
    /// first eviction sorts on.
    pub t: u64,
    /// Filename only, relative to the recordings directory.
    pub wav: String,
    pub app: String,
    pub engine: String,
    pub tier: String,
    pub audio_ms: u64,
    pub raw: String,
    pub polished: String,
}

pub fn recordings_dir() -> PathBuf {
    config::assets_dir().join("recordings")
}

fn index_path(dir: &Path) -> PathBuf {
    dir.join("index.jsonl")
}

/// Save one take's audio + transcripts under `assets_dir()/recordings/`,
/// then enforce `max_mb`. Failures are logged, never fatal -- losing a kept
/// recording must not affect dictation, same discipline as `stats::record`.
pub fn record(
    samples: &[f32],
    raw: &str,
    polished: &str,
    app: &str,
    engine: &str,
    tier: &str,
    audio_ms: u64,
    max_mb: u64,
) {
    let dir = recordings_dir();
    if let Err(err) = record_in(&dir, samples, raw, polished, app, engine, tier, audio_ms) {
        tracing::warn!(?err, "failed to save recording");
        return;
    }
    enforce_cap(&dir, max_mb.saturating_mul(1_000_000));
}

fn record_in(
    dir: &Path,
    samples: &[f32],
    raw: &str,
    polished: &str,
    app: &str,
    engine: &str,
    tier: &str,
    audio_ms: u64,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let wav_name = format!("{t}.wav");
    write_wav_mulaw(&dir.join(&wav_name), samples)?;
    let entry = RecordingEntry {
        t,
        wav: wav_name,
        app: app.to_string(),
        engine: engine.to_string(),
        tier: tier.to_string(),
        audio_ms,
        raw: raw.to_string(),
        polished: polished.to_string(),
    };
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(index_path(dir))?;
    writeln!(f, "{}", serde_json::to_string(&entry)?)?;
    Ok(())
}

// Not called yet — the read side for Wave 3b's review panel / 3e's snippet
// suggestions, which consume this index. Not speculative: both are already
// planned, just not built this pass.
#[allow(dead_code)]
pub fn load_index() -> Vec<RecordingEntry> {
    load_index_in(&recordings_dir())
}

fn load_index_in(dir: &Path) -> Vec<RecordingEntry> {
    let Ok(raw) = std::fs::read_to_string(index_path(dir)) else { return Vec::new() };
    raw.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// Total size of kept recordings on disk, in MB -- the wav files only, the
/// index itself is negligible. Settings' "Clear recordings" row reads this.
pub fn total_size_mb() -> u64 {
    let dir = recordings_dir();
    load_index_in(&dir)
        .iter()
        .map(|e| std::fs::metadata(dir.join(&e.wav)).map(|m| m.len()).unwrap_or(0))
        .sum::<u64>()
        / 1_000_000
}

/// Delete oldest entries (by `t`) until at or under `max_bytes`. Re-stats
/// every file on every call rather than tracking a running total -- simplest
/// correct thing at the scale this ever reaches (a cap in the hundreds of MB
/// at ~1 MB/min is at most a few thousand short takes), and it can't drift
/// out of sync with files someone deleted by hand.
fn enforce_cap(dir: &Path, max_bytes: u64) {
    let mut entries = load_index_in(dir);
    entries.sort_by_key(|e| e.t);
    let sizes: Vec<u64> =
        entries.iter().map(|e| std::fs::metadata(dir.join(&e.wav)).map(|m| m.len()).unwrap_or(0)).collect();
    let mut total: u64 = sizes.iter().sum();
    let mut evicted = 0;
    while total > max_bytes && evicted < entries.len() {
        let _ = std::fs::remove_file(dir.join(&entries[evicted].wav));
        total = total.saturating_sub(sizes[evicted]);
        evicted += 1;
    }
    if evicted > 0 {
        if let Ok(mut f) = std::fs::File::create(index_path(dir)) {
            for e in &entries[evicted..] {
                let _ = writeln!(f, "{}", serde_json::to_string(e).unwrap_or_default());
            }
        }
    }
}

/// Delete every kept recording and the index -- Settings' "Clear recordings".
pub fn clear() {
    let _ = std::fs::remove_dir_all(recordings_dir());
}

// ── mu-law WAV encoding ──────────────────────────────────────────────────

/// mu-law segment boundaries: `SEG_END[i] == 2^(i+8) - 1`.
const SEG_END: [i32; 8] = [0xFF, 0x1FF, 0x3FF, 0x7FF, 0xFFF, 0x1FFF, 0x3FFF, 0x7FFF];

/// ITU-T G.711 mu-law encode of one 16-bit linear PCM sample -- the standard
/// segment-table algorithm (the same form used by SoX, Asterisk's
/// codec_ulaw, etc.). Correctness leans on the round-trip test below against
/// `mulaw_to_linear`, not on this being transcribed perfectly from memory.
fn linear_to_mulaw(pcm_val: i16) -> u8 {
    const BIAS: i32 = 0x84;
    const CLIP: i32 = 32635;
    let (mut magnitude, mask): (i32, u8) =
        if pcm_val < 0 { (BIAS - pcm_val as i32, 0x7F) } else { (pcm_val as i32 + BIAS, 0xFF) };
    if magnitude > CLIP {
        magnitude = CLIP;
    }
    match SEG_END.iter().position(|&end| magnitude <= end) {
        Some(seg) => {
            let uval = ((seg as i32) << 4) | ((magnitude >> (seg + 3)) & 0x0F);
            (uval as u8) ^ mask
        }
        // Unreachable given the CLIP above (SEG_END[7] > CLIP), kept as a
        // defensive fallback matching the reference algorithm's own shape.
        None => 0x7F ^ mask,
    }
}

/// Inverse of `linear_to_mulaw`. Only used by the round-trip test -- WAV
/// playback of a mu-law file is the OS's/player's job, not ours.
#[cfg(test)]
fn mulaw_to_linear(encoded: u8) -> i16 {
    let u = !encoded; // mu-law stores the whole byte bit-inverted
    let mantissa = (u & 0x0F) as i32;
    let exponent = ((u & 0x70) >> 4) as i32;
    let t = ((mantissa << 3) + 0x84) << exponent;
    (if (u & 0x80) != 0 { 0x84 - t } else { t - 0x84 }) as i16
}

/// Write `samples` (16 kHz mono f32, Sotto's native format) as an 8-bit
/// mu-law WAV. `WAVE_FORMAT_MULAW` (tag 7) is non-PCM, which the WAV spec
/// requires a `fact` chunk for -- Windows Media Player, VLC and Audacity all
/// decode it unmodified.
fn write_wav_mulaw(path: &Path, samples: &[f32]) -> std::io::Result<()> {
    const RATE: u32 = 16_000;
    let pcm: Vec<u8> =
        samples.iter().map(|&s| linear_to_mulaw((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)).collect();
    let data_len = pcm.len() as u32;
    let mut wav = Vec::with_capacity(58 + pcm.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(50 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&18u32.to_le_bytes()); // non-PCM fmt body needs the extra cbSize field
    wav.extend_from_slice(&7u16.to_le_bytes()); // WAVE_FORMAT_MULAW
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&RATE.to_le_bytes()); // byte rate: 1 byte/sample
    wav.extend_from_slice(&1u16.to_le_bytes()); // block align
    wav.extend_from_slice(&8u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(&0u16.to_le_bytes()); // cbSize = 0
    wav.extend_from_slice(b"fact");
    wav.extend_from_slice(&4u32.to_le_bytes());
    wav.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(&pcm);
    std::fs::write(path, wav)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mulaw_round_trip_stays_within_companding_tolerance() {
        // mu-law is lossy by design -- this proves encode/decode are a real
        // inverse pair (catches a sign flip, a wrong bias, a shifted segment
        // table) rather than claiming bit-exact ITU-T conformance.
        for pcm in [-32000i16, -10000, -1000, -100, -1, 1, 100, 1000, 10000, 32000, i16::MAX, i16::MIN + 1] {
            let round = mulaw_to_linear(linear_to_mulaw(pcm));
            let err = (round as i32 - pcm as i32).abs();
            // mu-law's step size roughly doubles each segment, so the
            // worst-case quantization error scales with amplitude too --
            // ~1024 wide steps (up to ~512 error) right at full scale is the
            // documented, correct behavior of the companding curve, not a
            // bug. 2.5% of the sample's own magnitude (floor 50) tracks that
            // curve instead of one flat number that's either too loose for
            // quiet samples or (as measured: 643 at i16::MAX) too tight here.
            let tolerance = ((pcm as i32).unsigned_abs() as f64 * 0.025).max(50.0) as i32;
            assert!(err <= tolerance, "pcm={pcm} round-tripped to {round}, error {err} > tolerance {tolerance}");
        }
        // Silence matters most for a voice dictation app.
        assert!(mulaw_to_linear(linear_to_mulaw(0)).abs() <= 8);
    }

    #[test]
    fn wav_header_is_mulaw_and_correctly_sized() {
        let dir = std::env::temp_dir().join(format!("sotto-mulaw-hdr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.wav");
        let samples: Vec<f32> = (0..1600).map(|i| (i as f32 / 100.0).sin() * 0.5).collect(); // 100ms @ 16kHz
        write_wav_mulaw(&path, &samples).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[12..16], b"fmt ");
        let fmt_tag = u16::from_le_bytes(bytes[20..22].try_into().unwrap());
        assert_eq!(fmt_tag, 7, "must be WAVE_FORMAT_MULAW");
        assert_eq!(bytes.len(), 58 + samples.len());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Exercises `record_in`/`enforce_cap` against a private temp directory
    /// -- never `recordings_dir()`/`assets_dir()`, which read process-global
    /// config/env state and would race with other tests running in
    /// parallel (see `config::dir_tests` for the same discipline: pure,
    /// explicitly-parameterized logic, no shared global touched).
    #[test]
    fn enforce_cap_evicts_oldest_first_and_rewrites_the_index() {
        let dir = std::env::temp_dir().join(format!("sotto-cap-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let samples = vec![0.0f32; 800]; // ~858 bytes on disk (58-byte header + 800-byte mu-law body) each
        record_in(&dir, &samples, "one", "one", "app", "engine", "off", 50).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100)); // `t` is whole seconds — must differ to sort
        record_in(&dir, &samples, "two", "two", "app", "engine", "off", 50).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        record_in(&dir, &samples, "three", "three", "app", "engine", "off", 50).unwrap();
        let before = load_index_in(&dir);
        let oldest_wav = dir.join(&before.iter().find(|e| e.raw == "one").unwrap().wav);
        assert!(oldest_wav.exists(), "setup sanity check");

        // Cap sits between one file (~858B) and all three (~2574B): the
        // oldest ("one") must go, "two" and "three" must survive.
        enforce_cap(&dir, 2000);
        let kept = load_index_in(&dir);
        assert_eq!(kept.len(), 2, "one eviction expected, got {kept:?} entries remaining");
        assert_eq!(kept.iter().map(|e| e.raw.as_str()).collect::<Vec<_>>(), vec!["two", "three"]);
        assert!(!oldest_wav.exists(), "evicted entry's wav file must be deleted");
        for e in &kept {
            assert!(dir.join(&e.wav).exists(), "kept entry's wav file must still exist: {}", e.wav);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
