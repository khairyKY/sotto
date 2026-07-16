//! First-run provisioning of the heavy, rarely-changing assets — the ASR + LLM
//! models and the llama.cpp/CUDA runtime — that are deliberately **not** bundled
//! in the installer, so app updates stay tiny (a few MB instead of ~1.8 GB).
//!
//! On first launch anything missing is downloaded once into `data_dir()` from
//! the project's stable GitHub "assets" release; thereafter `config::*` finds
//! the files locally and nothing is re-downloaded. These assets are versioned
//! independently of the app (see `ASSET_BASE`), so shipping a new app version
//! never touches them.
//!
//! Correctness note: each file streams to a `<dest>.part` and is atomically
//! renamed only after a complete download, so an interrupted run never leaves a
//! half-written model that would later be mistaken for "installed".

use crate::config;
use anyhow::{Context, Result};
use serde::Serialize;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter};

/// Stable release tag holding the asset files. Decoupled from the app version:
/// bump this only when the models/runtime themselves change.
const ASSET_BASE: &str = "https://github.com/khairyKY/sotto/releases/download/assets-v1";

/// Prevents the startup auto-provision and a manual `download_assets` click from
/// running at the same time. ponytail: a single global flag — fine, there is
/// only ever one provisioning run for the whole app.
static IN_PROGRESS: AtomicBool = AtomicBool::new(false);

enum Kind {
    /// A plain file downloaded straight to `dest` (which is also its marker).
    File { dest: fn() -> PathBuf },
    /// A zip extracted into `dir`; `marker` proves the extraction completed.
    Zip { dir: fn() -> PathBuf, marker: fn() -> PathBuf },
}

struct Asset {
    /// Human label shown in the UI and used to key progress events.
    name: &'static str,
    /// File name of the asset within the GitHub release.
    file: &'static str,
    kind: Kind,
}

fn runtime_dir() -> PathBuf {
    config::data_dir().join("runtime").join("llama")
}

/// The assets the app needs to function, and where each lands. Markers are the
/// exact paths `config::*` reads, so "provisioned" here means "found" there.
fn manifest() -> Vec<Asset> {
    vec![
        Asset {
            name: "ONNX Runtime",
            file: "onnxruntime.dll",
            kind: Kind::File { dest: config::onnxruntime_dll },
        },
        Asset {
            name: "Parakeet v3 (speech-to-text)",
            file: "parakeet-tdt-0.6b-v3-int8.zip",
            kind: Kind::Zip {
                dir: config::model_dir,
                marker: || config::model_dir().join("encoder-model.int8.onnx"),
            },
        },
        Asset {
            name: "Qwen2.5 1.5B (AI polish)",
            file: "qwen2.5-1.5b-instruct-q4_k_m.gguf",
            kind: Kind::File { dest: config::llm_model_path },
        },
        Asset {
            name: "llama.cpp runtime",
            file: "llama-runtime.zip",
            kind: Kind::Zip { dir: runtime_dir, marker: config::llama_server_exe },
        },
    ]
}

fn is_present(a: &Asset) -> bool {
    match &a.kind {
        Kind::File { dest } => dest().exists(),
        Kind::Zip { marker, .. } => marker().exists(),
    }
}

#[derive(Serialize, Clone)]
pub struct AssetsStatus {
    /// True when every asset is present and dictation is fully functional.
    pub ready: bool,
    /// Names of assets still missing (for the settings "Models" section).
    pub missing: Vec<String>,
}

#[derive(Serialize, Clone)]
struct Progress {
    name: String,
    received: u64,
    total: u64,
}

/// Whether all assets are already on disk (so the settings UI can show a
/// download prompt / progress vs. "installed").
#[tauri::command]
pub fn assets_status() -> AssetsStatus {
    let missing: Vec<String> = manifest()
        .iter()
        .filter(|a| !is_present(a))
        .map(|a| a.name.to_string())
        .collect();
    AssetsStatus { ready: missing.is_empty(), missing }
}

/// Manual trigger from the settings "Download" button. Same path as the
/// first-run auto-provision; the in-progress guard keeps them from overlapping.
#[tauri::command]
pub fn download_assets(app: AppHandle) {
    spawn_provision_if_missing(app);
}

/// On launch, download anything missing in the background. No-op (emits
/// `assets-ready`) when everything is already present, e.g. an existing
/// `D:\sotto` install or after the first run.
pub fn spawn_provision_if_missing(app: AppHandle) {
    std::thread::spawn(move || {
        let all = manifest();
        // Log every marker path + presence — makes "why is it still
        // downloading?" debuggable from the log without a debugger attached.
        for a in &all {
            let (marker, ok) = match &a.kind {
                Kind::File { dest }      => { let p = dest();   let ok = p.exists(); (p, ok) }
                Kind::Zip  { marker, .. }=> { let p = marker(); let ok = p.exists(); (p, ok) }
            };
            tracing::info!(asset = a.name, marker = %marker.display(), present = ok, "asset check");
        }
        let missing: Vec<Asset> = all.into_iter().filter(|a| !is_present(a)).collect();
        if missing.is_empty() {
            tracing::info!("all assets present — no download needed");
            let _ = app.emit("assets-ready", true);
            return;
        }
        if IN_PROGRESS.swap(true, Ordering::SeqCst) {
            tracing::info!("asset provisioning already running");
            return;
        }
        let names: Vec<&str> = missing.iter().map(|a| a.name).collect();
        tracing::info!(?names, "provisioning missing assets from GitHub");
        // Clear leftovers from interrupted runs (downloads restart from zero,
        // so a stale .part / zip is pure disk garbage).
        for a in &missing {
            let dir = match &a.kind {
                Kind::File { dest } => dest().parent().map(|p| p.to_path_buf()),
                Kind::Zip { dir, .. } => Some(dir()),
            };
            let Some(dir) = dir else { continue };
            let Ok(entries) = fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.ends_with(".part") || name == "_download.zip" {
                    let _ = fs::remove_file(e.path());
                }
            }
        }
        let result = provision(&app, &missing);
        IN_PROGRESS.store(false, Ordering::SeqCst);
        match result {
            Ok(()) => {
                tracing::info!("all assets provisioned");
                let _ = app.emit("assets-ready", true);
            }
            Err(err) => {
                tracing::error!(?err, "asset provisioning failed");
                let _ = app.emit("asset-error", err.to_string());
            }
        }
    });
}

fn provision(app: &AppHandle, assets: &[Asset]) -> Result<()> {
    for a in assets {
        let url = format!("{ASSET_BASE}/{}", a.file);
        match &a.kind {
            Kind::File { dest } => {
                let dest = dest();
                download_to(app, a.name, &url, &dest)
                    .with_context(|| format!("downloading {}", a.name))?;
            }
            Kind::Zip { dir, .. } => {
                let dir = dir();
                download_and_extract(app, a.name, &url, &dir)
                    .with_context(|| format!("downloading {}", a.name))?;
            }
        }
    }
    Ok(())
}

/// Stream `url` to `dest` via a `.part` sibling, renaming only on success.
/// ponytail: no HTTP-range resume — a dropped download restarts that file from
/// zero. Add `Range` resume if large-file retries over flaky links become a
/// real complaint; correctness (never a half-written final file) holds either way.
fn download_to(app: &AppHandle, name: &str, url: &str, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let part = PathBuf::from(format!("{}.part", dest.display()));

    let resp = ureq::get(url).call().with_context(|| format!("GET {url}"))?;
    let total: u64 = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let mut reader = resp.into_body().into_reader();
    let mut file = fs::File::create(&part)
        .with_context(|| format!("creating {}", part.display()))?;
    let mut buf = vec![0u8; 1 << 16];
    let mut received: u64 = 0;
    let mut last_emit = std::time::Instant::now();

    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        received += n as u64;
        if last_emit.elapsed().as_millis() >= 200 {
            let _ = app.emit("asset-progress", Progress { name: name.into(), received, total });
            last_emit = std::time::Instant::now();
        }
    }
    file.flush()?;
    drop(file);
    fs::rename(&part, dest)
        .with_context(|| format!("finalizing {}", dest.display()))?;
    let _ = app.emit("asset-progress", Progress { name: name.into(), received, total });
    Ok(())
}

fn download_and_extract(app: &AppHandle, name: &str, url: &str, dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)?;
    let tmp = dir.join("_download.zip");
    download_to(app, name, url, &tmp)?;

    let f = fs::File::open(&tmp)?;
    let mut zip = zip::ZipArchive::new(f).context("opening downloaded zip")?;
    zip.extract(dir).context("extracting zip")?;
    let _ = fs::remove_file(&tmp);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_well_formed() {
        let m = manifest();
        // Every asset URL resolves under the pinned release base.
        for a in &m {
            let url = format!("{ASSET_BASE}/{}", a.file);
            assert!(url.starts_with("https://github.com/"), "bad url: {url}");
        }
        // Names are unique (they key progress events in the UI).
        let mut names: Vec<_> = m.iter().map(|a| a.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), m.len(), "asset names must be unique");
    }

    #[test]
    fn markers_match_config_consumers() {
        // The downloader's "present" markers must be the very paths the rest of
        // the app reads, or a completed download would still look missing.
        let m = manifest();
        let onnx = m.iter().find(|a| a.name.starts_with("ONNX")).unwrap();
        match &onnx.kind {
            Kind::File { dest } => assert_eq!(dest(), config::onnxruntime_dll()),
            _ => panic!("onnx should be a File asset"),
        }
        let qwen = m.iter().find(|a| a.name.starts_with("Qwen")).unwrap();
        match &qwen.kind {
            Kind::File { dest } => assert_eq!(dest(), config::llm_model_path()),
            _ => panic!("qwen should be a File asset"),
        }
        let llama = m.iter().find(|a| a.name.starts_with("llama")).unwrap();
        match &llama.kind {
            Kind::Zip { marker, .. } => assert_eq!(marker(), config::llama_server_exe()),
            _ => panic!("llama should be a Zip asset"),
        }
    }
}
