//! Microphone capture for hold-to-talk dictation.
//!
//! `Recorder::start` opens an input stream on the default device and appends
//! mono f32 samples (at the device's native rate) into a shared buffer;
//! `Recorder::stop` tears the stream down and returns the captured audio
//! resampled to the 16 kHz mono the ASR engine requires.
//!
//! A `Recorder` owns a `cpal::Stream`, which is `!Send` on Windows, so it must
//! be created and used entirely on one thread (the dictation worker).

use anyhow::Context;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// Sample rate the ASR engine expects.
const TARGET_RATE: u32 = 16_000;

/// Names of every available input device, for the settings microphone picker.
/// cpal 0.18 dropped `Device::name()` in favor of `Display` — `to_string()`
/// is the name.
pub fn list_input_devices() -> Vec<String> {
    let host = cpal::default_host();
    let Ok(devices) = host.input_devices() else { return Vec::new() };
    devices.map(|d| d.to_string()).collect()
}

pub struct Recorder {
    stream: Option<cpal::Stream>,
    buffer: Arc<Mutex<Vec<f32>>>,
    /// Live capture level (RMS of the most recent callback, f32 bits) — drives
    /// the overlay's Listening waveform. Zeroed when capture stops.
    level: Arc<AtomicU32>,
    device_rate: u32,
    /// User-selected input device name, or `None` for the OS default. Read
    /// live from settings so a change applies on the next dictation, no restart.
    device_name: Arc<Mutex<Option<String>>>,
    /// Device-rate samples left over from the last `drain` — the tail that
    /// didn't fill a whole 16 kHz output sample. Carried instead of dropped:
    /// at 48 kHz that's up to 2 samples per drain, which over a few hundred
    /// drains in a long take would silently eat audible slivers of speech.
    carry: Vec<f32>,
}

impl Recorder {
    /// `level` receives the live capture RMS (f32 bits) each audio callback.
    /// `device_name` is shared with the settings command that changes it.
    pub fn new(level: Arc<AtomicU32>, device_name: Arc<Mutex<Option<String>>>) -> Self {
        Self {
            stream: None,
            buffer: Arc::new(Mutex::new(Vec::new())),
            level,
            device_rate: 0,
            device_name,
            carry: Vec::new(),
        }
    }

    /// Begin capturing. Any previous stream is dropped and the buffer cleared.
    pub fn start(&mut self) -> anyhow::Result<()> {
        self.stream = None;
        self.buffer.lock().unwrap().clear();

        let host = cpal::default_host();
        let wanted = self.device_name.lock().unwrap().clone();
        let device = match wanted {
            Some(name) => host
                .input_devices()
                .ok()
                .and_then(|mut ds| ds.find(|d| d.to_string() == name))
                // Falls back to default if the named device vanished (e.g.
                // unplugged) rather than failing dictation outright.
                .or_else(|| host.default_input_device())
                .context("no input device found")?,
            None => host
                .default_input_device()
                .context("no default input device found")?,
        };
        let supported = device
            .default_input_config()
            .context("no default input config")?;

        let sample_format = supported.sample_format();
        let channels = supported.channels() as usize;
        self.device_rate = supported.sample_rate();
        let config: cpal::StreamConfig = supported.config();

        tracing::info!(
            device = %device,
            rate = self.device_rate,
            channels,
            ?sample_format,
            "opening input stream"
        );

        let buf = self.buffer.clone();
        let err_fn = |e| tracing::error!(error = %e, "audio input stream error");

        // Downmix interleaved frames to mono by averaging channels, and publish
        // the RMS of each callback as the live capture level.
        let level_f32 = self.level.clone();
        let level_i16 = self.level.clone();
        let stream = match sample_format {
            SampleFormat::F32 => device.build_input_stream(
                config.clone(),
                move |data: &[f32], _: &_| {
                    let mut b = buf.lock().unwrap();
                    let mut sumsq = 0.0f32;
                    let mut n = 0usize;
                    for frame in data.chunks(channels) {
                        let m = frame.iter().sum::<f32>() / channels as f32;
                        b.push(m);
                        sumsq += m * m;
                        n += 1;
                    }
                    publish_level(&level_f32, sumsq, n);
                },
                err_fn,
                None,
            )?,
            SampleFormat::I16 => device.build_input_stream(
                config.clone(),
                move |data: &[i16], _: &_| {
                    let mut b = buf.lock().unwrap();
                    let mut sumsq = 0.0f32;
                    let mut n = 0usize;
                    for frame in data.chunks(channels) {
                        let m = frame.iter().map(|&s| s as f32 / 32768.0).sum::<f32>() / channels as f32;
                        b.push(m);
                        sumsq += m * m;
                        n += 1;
                    }
                    publish_level(&level_i16, sumsq, n);
                },
                err_fn,
                None,
            )?,
            other => anyhow::bail!("unsupported input sample format: {other:?}"),
        };

        stream.play().context("failed to start input stream")?;
        self.stream = Some(stream);
        Ok(())
    }

    /// Take everything captured since the last call, as 16 kHz mono, leaving
    /// the stream running. This is what makes transcribing *during* recording
    /// possible: the worker pulls audio out as the user speaks instead of
    /// waiting for the whole take.
    ///
    /// Whatever doesn't fill a complete output sample stays in `carry` and is
    /// picked up by the next call, so draining a take in N pieces yields the
    /// same samples as draining it in one.
    pub fn drain(&mut self) -> Vec<f32> {
        let fresh = std::mem::take(&mut *self.buffer.lock().unwrap());
        if self.carry.is_empty() {
            self.carry = fresh;
        } else {
            self.carry.extend_from_slice(&fresh);
        }
        let (out, consumed) = resample_prefix(&self.carry, self.device_rate);
        self.carry.drain(..consumed);
        out
    }

    /// Stop capturing and return whatever audio is left as 16 kHz mono f32 in
    /// [-1.0, 1.0]. Any carried remainder is flushed here, since there's no
    /// next drain to pick it up.
    pub fn stop(&mut self) -> anyhow::Result<Vec<f32>> {
        self.stream = None; // dropping the stream stops capture
        self.level.store(0.0f32.to_bits(), Ordering::Relaxed);
        let fresh = std::mem::take(&mut *self.buffer.lock().unwrap());
        let mut rest = std::mem::take(&mut self.carry);
        rest.extend_from_slice(&fresh);
        Ok(resample_to_16k(&rest, self.device_rate))
    }
}

/// Resample as much of `input` as forms whole output samples, returning the
/// output and how much input it consumed. The unconsumed tail is the caller's
/// to carry forward.
///
/// Same box-filter/linear scheme as `resample_to_16k` — the difference is only
/// that this one refuses to emit a final output sample whose input span runs
/// off the end of the slice, which is exactly the sample that would be wrong
/// if more audio is still coming.
fn resample_prefix(input: &[f32], in_rate: u32) -> (Vec<f32>, usize) {
    if input.is_empty() || in_rate == 0 || in_rate == TARGET_RATE {
        return (input.to_vec(), input.len());
    }
    let ratio = in_rate as f64 / TARGET_RATE as f64;
    let out_len = (input.len() as f64 / ratio) as usize;
    if out_len == 0 {
        return (Vec::new(), 0);
    }
    let mut out = Vec::with_capacity(out_len);
    if ratio > 1.0 {
        for j in 0..out_len {
            let start = (j as f64 * ratio) as usize;
            let end = (((j + 1) as f64 * ratio) as usize).max(start + 1).min(input.len());
            let slice = &input[start..end];
            out.push(slice.iter().sum::<f32>() / slice.len() as f32);
        }
    } else {
        for j in 0..out_len {
            let pos = j as f64 * ratio;
            let i = pos as usize;
            let frac = (pos - i as f64) as f32;
            let a = input[i];
            let b = *input.get(i + 1).unwrap_or(&a);
            out.push(a + (b - a) * frac);
        }
    }
    let consumed = ((out_len as f64 * ratio) as usize).min(input.len());
    (out, consumed)
}

/// Store the RMS of a callback's mono samples as the live capture level.
fn publish_level(level: &AtomicU32, sumsq: f32, n: usize) {
    if n > 0 {
        let rms = (sumsq / n as f32).sqrt();
        level.store(rms.to_bits(), Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The seam test for chunked transcription: a take drained in many small
    /// pieces has to yield the same audio as one drained whole. If this drifts,
    /// long dictations quietly lose slivers of speech at every drain boundary —
    /// the kind of bug that shows up as "it dropped a word" and nothing else.
    #[test]
    fn piecewise_drain_matches_one_shot() {
        let rate = 48_000;
        let input: Vec<f32> = vec![0.5; rate as usize * 3]; // 3s of steady tone
        let one_shot = resample_to_16k(&input, rate);

        // Mirrors Recorder::drain + the final flush in stop().
        let mut carry: Vec<f32> = Vec::new();
        let mut pieces: Vec<f32> = Vec::new();
        for block in input.chunks(1024) {
            carry.extend_from_slice(block);
            let (out, consumed) = resample_prefix(&carry, rate);
            carry.drain(..consumed);
            pieces.extend(out);
        }
        pieces.extend(resample_to_16k(&carry, rate));

        assert!(
            (pieces.len() as i64 - one_shot.len() as i64).abs() <= 1,
            "piecewise produced {} samples, one-shot {}",
            pieces.len(),
            one_shot.len()
        );
        assert!(pieces.iter().all(|&s| (s - 0.5).abs() < 1e-6), "seam artifact in output");
    }

    #[test]
    fn resample_prefix_never_over_consumes() {
        let input = vec![0.1f32; 1000];
        for rate in [44_100, 48_000, 16_000, 8_000] {
            let (_, consumed) = resample_prefix(&input, rate);
            assert!(consumed <= input.len(), "rate {rate} consumed {consumed} of {}", input.len());
        }
    }
}

/// Resample mono f32 audio to 16 kHz.
///
/// For downsampling (the common case — mics run at 44.1/48 kHz) this uses a
/// box filter: each output sample is the average of the input samples it
/// spans, which is cheap and provides basic anti-aliasing. For the rare
/// upsampling case it falls back to linear interpolation. Good enough to prove
/// the pipeline; a higher-quality sinc resampler (rubato) is a later upgrade.
fn resample_to_16k(input: &[f32], in_rate: u32) -> Vec<f32> {
    if input.is_empty() || in_rate == 0 || in_rate == TARGET_RATE {
        return input.to_vec();
    }

    let out_len = (input.len() as u64 * TARGET_RATE as u64 / in_rate as u64) as usize;
    if out_len == 0 {
        return Vec::new();
    }
    let ratio = in_rate as f64 / TARGET_RATE as f64;
    let mut out = Vec::with_capacity(out_len);

    if ratio > 1.0 {
        // Downsample: average the span [j*ratio, (j+1)*ratio).
        for j in 0..out_len {
            let start = (j as f64 * ratio) as usize;
            let end = (((j + 1) as f64 * ratio) as usize).max(start + 1).min(input.len());
            let slice = &input[start..end];
            out.push(slice.iter().sum::<f32>() / slice.len() as f32);
        }
    } else {
        // Upsample: linear interpolation.
        for j in 0..out_len {
            let pos = j as f64 * ratio;
            let i = pos as usize;
            let frac = (pos - i as f64) as f32;
            let a = input[i];
            let b = *input.get(i + 1).unwrap_or(&a);
            out.push(a + (b - a) * frac);
        }
    }

    out
}
