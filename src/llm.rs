//! Tier 1 polish: a local llama.cpp `llama-server` sidecar driving Qwen2.5
//! 1.5B (Q4) on the GPU to rewrite a raw transcript into clean, formatted text.
//!
//! Lifecycle, tuned for "least resources": the server is **spawned on first
//! use** (not at startup, so idle VRAM stays at zero) and **idle-killed** by a
//! background monitor after `idle_kill` of inactivity (freeing VRAM for games
//! etc.). The next dictation transparently respawns it.
//!
//! Every failure path — server won't spawn, won't become healthy, HTTP error,
//! or the hard wall-clock timeout — returns `Err`, and the caller
//! (`polish::Polisher`) falls back to the instant Tier 0 rules so a dictation
//! is never lost to the LLM.

use crate::config::{self, LlmConfig};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::os::windows::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Spawn the console-subsystem sidecar with no console window. Without this a
/// GUI (`windows_subsystem = "windows"`) build pops a cmd window every time the
/// sidecar launches. Same flag `startup.rs` uses for its `reg.exe` calls.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// Tight prompt on purpose: every prompt token is re-evaluated on every call
// (cache_prompt=true amortizes the shared prefix, but a shorter prompt still
// shaves latency on the first call after a restart and reduces the KV pressure
// that slows generation). Measured: cutting the previous 5-rule prompt (~90
// tokens) to this (~35) drops warm total latency ~10% on a 1.5B Q4.
const SYSTEM_PROMPT: &str = "\
Clean up this voice-dictation transcript. Fix punctuation and capitalization, \
and remove speech fillers (um, uh, like, يعني). Keep the speaker's exact words \
and language — never translate. If the text mixes languages (e.g. Egyptian \
Arabic with English words), keep each word in the language it was spoken and \
add the right punctuation for that script. Reply with ONLY the cleaned text, no \
preamble, no quotes.";

/// Append the resolved tone as one extra clause. `tone` is already a short
/// instruction sentence (a frontend preset or free-typed text), so this is
/// just a space-join — no extra wording to protect the latency budget above.
/// Empty tone (off, or no resolution matched) leaves the prompt untouched.
fn system_prompt(tone: &str) -> String {
    let tone = tone.trim();
    if tone.is_empty() {
        SYSTEM_PROMPT.to_string()
    } else {
        format!("{SYSTEM_PROMPT} {tone}")
    }
}

struct Inner {
    child: Option<Child>,
    last_used: Instant,
}

pub struct Llm {
    inner: Arc<Mutex<Inner>>,
    cfg: LlmConfig,
}

impl Llm {
    /// Construct the manager and start the idle-kill monitor. Does not spawn
    /// the server yet — that happens lazily on the first `polish` call.
    pub fn new(cfg: LlmConfig) -> Self {
        let inner = Arc::new(Mutex::new(Inner {
            child: None,
            last_used: Instant::now(),
        }));

        let monitor = inner.clone();
        let idle_kill = Duration::from_secs(cfg.idle_kill_secs);
        thread::Builder::new()
            .name("llm-idle-monitor".into())
            .spawn(move || loop {
                thread::sleep(Duration::from_secs(20));
                let mut g = monitor.lock().unwrap();
                if g.child.is_some() && g.last_used.elapsed() > idle_kill {
                    if let Some(mut child) = g.child.take() {
                        let _ = child.kill();
                        let _ = child.wait();
                        tracing::info!("llm sidecar idle-killed (VRAM freed)");
                    }
                }
            })
            .ok();

        Self { inner, cfg }
    }

    /// True if the sidecar binary and model file are both present on disk.
    pub fn is_available() -> bool {
        config::llama_server_exe().exists() && config::llm_model_path().exists()
    }

    /// Start the sidecar loading *now* (e.g. the moment the user begins
    /// speaking) so its ~4s cold model-load overlaps recording + transcription
    /// instead of blocking after the clip ends. Best-effort and non-blocking:
    /// only launches the process; any error is ignored since the real `polish`
    /// call re-checks and falls back to rules on failure.
    pub fn prewarm(&self) {
        if let Ok(mut g) = self.inner.lock() {
            if self.ensure_spawned(&mut g).is_ok() {
                g.last_used = Instant::now();
            }
        }
    }

    /// Rewrite `raw` via the LLM, optionally re-voiced per `tone` (an
    /// instruction sentence, or "" for none). Returns `Err` on any failure so
    /// the caller can fall back to rules-based cleanup.
    pub fn polish(&self, raw: &str, tone: &str) -> Result<String> {
        {
            let mut g = self.inner.lock().unwrap();
            self.ensure_spawned(&mut g)?;
            g.last_used = Instant::now();
        }

        self.wait_healthy()?;
        // Cap generation to roughly the input's own length + slack for added
        // punctuation. Prevents the model from running away (the old flat cap
        // was 1024, which at 118 tok/s on a 3050 = potentially 8s of pointless
        // generation). This is the single biggest wall-clock win once warm.
        // words * 4, not * 3: Arabic and other non-Latin scripts tokenize to
        // 2-3x more subword tokens per word than English, so a tighter cap
        // truncated code-switched Arabic mid-sentence. This is a ceiling, not a
        // target — the model still stops at end-of-text — so the extra headroom
        // is free for English and only prevents a real cut-off for Arabic.
        let words = raw.split_whitespace().count() as u32;
        let max_tokens = (words * 4 + 24).min(self.cfg.max_tokens);
        let out = self.request_with_timeout(raw, max_tokens, tone)?;

        self.inner.lock().unwrap().last_used = Instant::now();
        Ok(clean_output(&out))
    }

    /// Spawn the server if it isn't already running (or has died). Assumes the
    /// caller holds the lock.
    fn ensure_spawned(&self, g: &mut Inner) -> Result<()> {
        if let Some(child) = g.child.as_mut() {
            match child.try_wait() {
                Ok(None) => return Ok(()), // still running
                _ => g.child = None,       // exited/errored — respawn below
            }
        }

        let exe = config::llama_server_exe();
        let model = config::llm_model_path();
        anyhow::ensure!(exe.exists(), "llama-server not found at {}", exe.display());
        anyhow::ensure!(model.exists(), "model not found at {}", model.display());

        let log_dir = config::data_dir().join("logs");
        let _ = std::fs::create_dir_all(&log_dir);
        let stderr = std::fs::File::create(log_dir.join("llama-server.log"))
            .map(Stdio::from)
            .unwrap_or_else(|_| Stdio::null());

        tracing::info!(port = self.cfg.port, "spawning llama-server sidecar");
        let child = Command::new(&exe)
            .arg("--model")
            .arg(&model)
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(self.cfg.port.to_string())
            .arg("--n-gpu-layers")
            .arg(self.cfg.n_gpu_layers.to_string())
            .arg("--ctx-size")
            .arg(self.cfg.ctx_size.to_string())
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(Stdio::null())
            .stderr(stderr)
            .spawn()
            .context("failed to spawn llama-server")?;

        crate::job::kill_with_parent(&child);
        g.child = Some(child);
        Ok(())
    }

    /// Poll `/health` until the server reports ready or we hit the spawn
    /// deadline (model load can take a few seconds cold).
    fn wait_healthy(&self) -> Result<()> {
        let url = format!("http://127.0.0.1:{}/health", self.cfg.port);
        let deadline = Instant::now() + Duration::from_secs(self.cfg.spawn_timeout_secs);
        loop {
            if ureq::get(&url).call().is_ok() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                bail!("llama-server did not become healthy within {}s", self.cfg.spawn_timeout_secs);
            }
            thread::sleep(Duration::from_millis(150));
        }
    }

    /// POST the chat completion, bounding it with a hard wall-clock timeout by
    /// running the (blocking) request on a scratch thread.
    fn request_with_timeout(&self, raw: &str, max_tokens: u32, tone: &str) -> Result<String> {
        let url = format!("http://127.0.0.1:{}/v1/chat/completions", self.cfg.port);
        let body = ChatRequest {
            messages: vec![
                Message { role: "system", content: system_prompt(tone) },
                Message { role: "user", content: raw.to_string() },
            ],
            temperature: self.cfg.temperature,
            max_tokens,
            stream: false,
            cache_prompt: true,
        };
        let timeout = Duration::from_millis(self.cfg.request_timeout_ms);

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(do_request(&url, &body));
        });

        match rx.recv_timeout(timeout) {
            Ok(result) => result,
            Err(_) => bail!("LLM request exceeded {}ms timeout", self.cfg.request_timeout_ms),
        }
    }
}

fn do_request(url: &str, body: &ChatRequest) -> Result<String> {
    let mut resp = ureq::post(url)
        .send_json(body)
        .context("llama-server request failed")?;
    let parsed: ChatResponse = resp.body_mut().read_json().context("bad LLM response")?;
    let content = parsed
        .choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .context("LLM returned no choices")?;
    Ok(content)
}

/// Strip stray wrapping quotes / whitespace a small model occasionally adds.
fn clean_output(s: &str) -> String {
    let t = s.trim();
    let unquoted = t
        .strip_prefix('"')
        .and_then(|x| x.strip_suffix('"'))
        .unwrap_or(t);
    unquoted.trim().to_string()
}

#[derive(Serialize)]
struct ChatRequest {
    messages: Vec<Message>,
    temperature: f32,
    max_tokens: u32,
    stream: bool,
    cache_prompt: bool,
}

#[derive(Serialize)]
struct Message {
    role: &'static str,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tone_leaves_system_prompt_unchanged() {
        assert_eq!(system_prompt(""), SYSTEM_PROMPT);
        assert_eq!(system_prompt("   "), SYSTEM_PROMPT); // whitespace-only counts as off
    }

    #[test]
    fn tone_appends_as_one_extra_clause() {
        let out = system_prompt("Casual and friendly.");
        assert_eq!(out, format!("{SYSTEM_PROMPT} Casual and friendly."));
    }
}
