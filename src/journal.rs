//! Crash journal for takes in flight (N4). Audio exists only in RAM until
//! it's injected, so a PC crash mid-take used to lose all of it. Now every
//! take's audio is appended to `data_dir()/pending/<launch>-<id>.pcm` while
//! it records, and the file is deleted when the take is dropped: delivered,
//! dismissed, or replaced in the retry stash (see `Take`'s `Drop` in main.rs).
//! Anything still here at the next launch belonged to a take that never got
//! delivered, and comes back through the existing retry stash.
//!
//! Always on, unlike history and recordings: this is audio you just spoke and
//! are still waiting on, not a log, and it deletes itself.
//!
//! Writes happen on this module's own thread, never the audio thread (which
//! owns the `!Send` cpal stream and must always be free to answer the hotkey):
//! `append` is an unbounded channel send. Raw 16 kHz mono i16 LE, no header
//! — nothing but `recover` ever reads it — at ~1.9 MB/min.

use crate::config;
use crossbeam_channel::Sender;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Stop growing a journal past 10 minutes of audio (~19 MB). The first ten
/// minutes of a longer take are still recovered.
const MAX_BYTES: u64 = 16_000 * 2 * 60 * 10;

enum Msg {
    Append(u64, Vec<f32>),
    Remove(PathBuf),
}

static TX: OnceLock<Sender<Msg>> = OnceLock::new();

fn dir() -> PathBuf {
    config::data_dir().join("pending")
}

/// This launch's journal file for take `id`. The launch stamp keeps a new
/// take from ever reusing a leftover's name; zero-padding keeps a plain name
/// sort chronological.
pub fn path_for(id: u64) -> PathBuf {
    static LAUNCH: OnceLock<u128> = OnceLock::new();
    let launch = LAUNCH.get_or_init(|| {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
    });
    dir().join(format!("{launch}-{id:08}.pcm"))
}

/// Start the writer thread. Call once, after `recover`.
pub fn spawn() {
    let (tx, rx) = crossbeam_channel::unbounded::<Msg>();
    if TX.set(tx).is_err() {
        return;
    }
    std::thread::spawn(move || {
        for msg in rx {
            match msg {
                Msg::Append(id, samples) => {
                    if let Err(err) = append_to(&path_for(id), &samples, MAX_BYTES) {
                        tracing::warn!(?err, id, "failed to journal take audio");
                    }
                }
                Msg::Remove(path) => {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
    });
}

/// Queue `samples` for take `id`'s journal. Never blocks.
pub fn append(id: u64, samples: &[f32]) {
    if let (Some(tx), false) = (TX.get(), samples.is_empty()) {
        let _ = tx.send(Msg::Append(id, samples.to_vec()));
    }
}

/// Queue a journal's deletion. Same queue as `append`, so it always lands
/// after every append already sent for that take.
pub fn remove(path: PathBuf) {
    if let Some(tx) = TX.get() {
        let _ = tx.send(Msg::Remove(path));
    }
}

/// Leftovers from the last run: `(files, audio)`, oldest first and joined
/// into one take (several only when takes overlapped). `None` when there's
/// nothing, or less than `min_samples` in total, which is deleted.
pub fn recover(min_samples: usize) -> Option<(Vec<PathBuf>, Vec<f32>)> {
    recover_in(&dir(), min_samples)
}

fn recover_in(dir: &Path, min_samples: usize) -> Option<(Vec<PathBuf>, Vec<f32>)> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "pcm"))
        .collect();
    files.sort();
    let mut samples = Vec::new();
    for f in &files {
        let Ok(bytes) = std::fs::read(f) else { continue };
        samples.extend(bytes.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / i16::MAX as f32));
    }
    if samples.len() < min_samples {
        for f in &files {
            let _ = std::fs::remove_file(f);
        }
        return None;
    }
    Some((files, samples))
}

/// Append as i16 and flush to disk: a PC crash is the case this exists for,
/// so the audio can't sit in the OS write cache.
fn append_to(path: &Path, samples: &[f32], max_bytes: u64) -> std::io::Result<()> {
    if std::fs::metadata(path).map(|m| m.len()).unwrap_or(0) >= max_bytes {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes: Vec<u8> = samples
        .iter()
        .flat_map(|&s| ((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes())
        .collect();
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    f.write_all(&bytes)?;
    f.sync_data()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A private temp dir per test — never `data_dir()`.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sotto-journal-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn nothing_left_over_means_nothing_to_offer() {
        let dir = temp_dir("empty");
        assert!(recover_in(&dir, 10).is_none(), "no pending dir at all");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(recover_in(&dir, 10).is_none(), "empty pending dir");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn appended_audio_comes_back_on_recovery() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("1-00000001.pcm");
        let a: Vec<f32> = (0..800).map(|i| (i as f32 / 50.0).sin() * 0.5).collect();
        append_to(&path, &a[..300], MAX_BYTES).unwrap();
        append_to(&path, &a[300..], MAX_BYTES).unwrap();
        let (files, back) = recover_in(&dir, 100).expect("a real leftover is offered");
        assert_eq!(files, [path]);
        assert_eq!(back.len(), a.len());
        assert!(back.iter().zip(&a).all(|(x, y)| (x - y).abs() < 1e-3), "i16 round trip");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn overlapping_takes_are_joined_oldest_first() {
        let dir = temp_dir("overlap");
        // id 10 sorts after id 9 only because of the zero-padding.
        append_to(&dir.join("5-00000010.pcm"), &[0.5; 100], MAX_BYTES).unwrap();
        append_to(&dir.join("5-00000009.pcm"), &[-0.5; 100], MAX_BYTES).unwrap();
        let (files, back) = recover_in(&dir, 150).unwrap();
        assert_eq!(files.len(), 2);
        assert!(back[0] < 0.0 && back[199] > 0.0, "take 9's audio comes first");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_too_short_leftover_is_deleted_not_offered() {
        let dir = temp_dir("short");
        let path = dir.join("1-00000001.pcm");
        append_to(&path, &[0.1; 50], MAX_BYTES).unwrap();
        assert!(recover_in(&dir, 100).is_none());
        assert!(!path.exists(), "a tap's leftover must not come back every launch");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_stops_growing_at_the_cap() {
        let dir = temp_dir("cap");
        let path = dir.join("1-00000001.pcm");
        for _ in 0..5 {
            append_to(&path, &[0.2; 100], 400).unwrap();
        }
        // 200 bytes per append: two fit, the third finds the cap reached.
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 400);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
