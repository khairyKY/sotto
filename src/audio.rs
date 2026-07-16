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

    /// Stop capturing and return the audio as 16 kHz mono f32 in [-1.0, 1.0].
    pub fn stop(&mut self) -> anyhow::Result<Vec<f32>> {
        self.stream = None; // dropping the stream stops capture
        self.level.store(0.0f32.to_bits(), Ordering::Relaxed);
        let samples = std::mem::take(&mut *self.buffer.lock().unwrap());
        Ok(resample_to_16k(&samples, self.device_rate))
    }
}

/// Store the RMS of a callback's mono samples as the live capture level.
fn publish_level(level: &AtomicU32, sumsq: f32, n: usize) {
    if n > 0 {
        let rms = (sumsq / n as f32).sqrt();
        level.store(rms.to_bits(), Ordering::Relaxed);
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
