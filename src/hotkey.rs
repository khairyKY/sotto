use crossbeam_channel::Sender;
use rdev::{listen, EventType, Key};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;

use crate::config::ActivationMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DictationEvent {
    Start,
    Stop,
}

/// The hotkeys the settings window offers, in order: display label, config.toml
/// name, and the rdev `Key`. The listener reads the selected index live, so a
/// rebind takes effect without a restart. (rdev calls the right-hand Alt key
/// "AltGr" — its behavior on most layouts.)
pub const SUPPORTED_HOTKEYS: &[(&str, &str, Key)] = &[
    ("Right Ctrl", "ControlRight", Key::ControlRight),
    ("Left Ctrl", "ControlLeft", Key::ControlLeft),
    ("Right Alt", "AltGr", Key::AltGr),
    ("Left Alt", "Alt", Key::Alt),
    ("Right Shift", "ShiftRight", Key::ShiftRight),
    ("Left Shift", "ShiftLeft", Key::ShiftLeft),
    ("Caps Lock", "CapsLock", Key::CapsLock),
];

/// Index into [`SUPPORTED_HOTKEYS`] for a config.toml key name (defaults to
/// Right Ctrl for an unknown/legacy name).
pub fn index_of(name: &str) -> usize {
    SUPPORTED_HOTKEYS
        .iter()
        .position(|(_, cfg_name, _)| *cfg_name == name)
        .unwrap_or_else(|| {
            tracing::warn!(hotkey = name, "unknown hotkey in config — defaulting to Right Ctrl");
            0
        })
}

/// Blocks the calling thread forever, listening system-wide for the configured
/// hotkey and emitting `DictationEvent`s on `tx`. Must run on its own
/// dedicated OS thread — rdev owns the thread it's called from on Windows.
///
/// `suppressed` must be set to `true` for the duration of any text injection
/// we perform ourselves. Without it, injecting a real Ctrl+V (the
/// clipboard-paste fallback) is indistinguishable — to this same global
/// hook — from the user pressing Ctrl, which self-triggers another
/// dictation cycle whenever the hotkey involves a modifier key. Measured
/// live: a single paste fired 8 recursive cycles before it settled.
pub fn run_listener(
    hotkey_idx: Arc<AtomicUsize>,
    activation: Arc<AtomicU8>,
    tx: Sender<DictationEvent>,
    suppressed: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
) {
    let mut is_held = false;
    let mut is_active = false; // toggle-mode recording state

    let callback = move |event: rdev::Event| {
        if suppressed.load(Ordering::SeqCst) {
            return;
        }

        let key = match event.event_type {
            EventType::KeyPress(k) => Some((k, true)),
            EventType::KeyRelease(k) => Some((k, false)),
            _ => None,
        };

        let Some((key, pressed)) = key else { return };
        // Read the bound hotkey + activation mode live, so settings changes
        // apply without a restart.
        let idx = hotkey_idx.load(Ordering::Relaxed).min(SUPPORTED_HOTKEYS.len() - 1);
        if key != SUPPORTED_HOTKEYS[idx].2 {
            return;
        }
        let mode = ActivationMode::from_u8(activation.load(Ordering::Relaxed));

        // Pausing only blocks *starting* a new dictation — a stop/release
        // already in flight always goes through, so we never strand the
        // recorder mid-capture.
        match mode {
            ActivationMode::Hold => {
                if pressed && !is_held {
                    if paused.load(Ordering::Relaxed) {
                        return;
                    }
                    is_held = true;
                    let _ = tx.send(DictationEvent::Start);
                } else if !pressed && is_held {
                    is_held = false;
                    let _ = tx.send(DictationEvent::Stop);
                }
            }
            ActivationMode::Toggle => {
                // Only react on the press edge; ignore the matching release.
                if pressed {
                    if !is_active && paused.load(Ordering::Relaxed) {
                        return;
                    }
                    is_active = !is_active;
                    let _ = tx.send(if is_active {
                        DictationEvent::Start
                    } else {
                        DictationEvent::Stop
                    });
                }
            }
        }
    };

    if let Err(err) = listen(callback) {
        tracing::error!(?err, "rdev global hotkey listener stopped unexpectedly");
    }
}
