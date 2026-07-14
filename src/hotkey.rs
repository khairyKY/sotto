use crossbeam_channel::Sender;
use rdev::{listen, Button, EventType, Key};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;

use crate::config::ActivationMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DictationEvent {
    Start,
    Stop,
}

/// A hotkey binding source — either a keyboard key or a mouse button. The
/// event listener matches on both event families accordingly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Key(Key),
    Button(Button),
}

/// Every hotkey the settings window offers, in order.
///
/// Fields: display label, `config.toml` name (stable across renames), the
/// `Input` the listener matches, and `risky` — a hint the UI uses to ask
/// "are you sure?" before saving. Risky ≠ blocked; a determined user can
/// still bind Left Click. The warning fires because that button is in
/// constant use elsewhere so mis-triggering dictation would be constant.
pub const SUPPORTED_HOTKEYS: &[(&str, &str, Input, bool)] = &[
    // Modifiers — the sensible defaults, safe to hold/toggle.
    ("Right Ctrl",           "ControlRight", Input::Key(Key::ControlRight),      false),
    ("Left Ctrl",            "ControlLeft",  Input::Key(Key::ControlLeft),       false),
    ("Right Alt",            "AltGr",        Input::Key(Key::AltGr),             false),
    ("Left Alt",             "Alt",          Input::Key(Key::Alt),               false),
    ("Right Shift",          "ShiftRight",   Input::Key(Key::ShiftRight),        false),
    ("Left Shift",           "ShiftLeft",    Input::Key(Key::ShiftLeft),         false),
    ("Caps Lock",            "CapsLock",     Input::Key(Key::CapsLock),          false),
    // Function keys F1-F12 — warn on the handful that many apps already claim.
    ("F1",  "F1",  Input::Key(Key::F1),  true),   // Help
    ("F2",  "F2",  Input::Key(Key::F2),  false),
    ("F3",  "F3",  Input::Key(Key::F3),  false),
    ("F4",  "F4",  Input::Key(Key::F4),  false),
    ("F5",  "F5",  Input::Key(Key::F5),  true),   // Browser refresh
    ("F6",  "F6",  Input::Key(Key::F6),  false),
    ("F7",  "F7",  Input::Key(Key::F7),  false),
    ("F8",  "F8",  Input::Key(Key::F8),  false),
    ("F9",  "F9",  Input::Key(Key::F9),  false),
    ("F10", "F10", Input::Key(Key::F10), false),
    ("F11", "F11", Input::Key(Key::F11), true),   // Full-screen
    ("F12", "F12", Input::Key(Key::F12), true),   // DevTools
    // Numpad — often unused as text but nice for a dedicated hotkey.
    ("Numpad Enter",    "NumpadEnter",    Input::Key(Key::KpReturn),   false),
    ("Numpad +",        "NumpadAdd",      Input::Key(Key::KpPlus),     false),
    ("Numpad −",        "NumpadSubtract", Input::Key(Key::KpMinus),    false),
    ("Numpad ×",        "NumpadMultiply", Input::Key(Key::KpMultiply), false),
    ("Numpad ÷",        "NumpadDivide",   Input::Key(Key::KpDivide),   false),
    ("Num Lock",        "NumLock",        Input::Key(Key::NumLock),    false),
    ("Function key",    "Function",       Input::Key(Key::Function),   false),
    // Note: rdev 0.5 has no named F13-F24 variants — they'd arrive as
    // Unknown(scancode). Streamdeck / macro-pad users should configure the
    // device to emit a standard key (F1-F12, a letter, a numpad key, etc.).
    // Space / whitespace / edit keys — commonly typed, so risky.
    ("Space",     "Space",     Input::Key(Key::Space),     true),
    ("Tab",       "Tab",       Input::Key(Key::Tab),       true),
    ("Enter",     "Return",    Input::Key(Key::Return),    true),
    ("Backspace", "Backspace", Input::Key(Key::Backspace), true),
    ("Escape",    "Escape",    Input::Key(Key::Escape),    true),
    // Navigation & editing.
    ("Insert",       "Insert",       Input::Key(Key::Insert),       false),
    ("Delete",       "Delete",       Input::Key(Key::Delete),       false),
    ("Home",         "Home",         Input::Key(Key::Home),         false),
    ("End",          "End",          Input::Key(Key::End),          false),
    ("Page Up",      "PageUp",       Input::Key(Key::PageUp),       false),
    ("Page Down",    "PageDown",     Input::Key(Key::PageDown),     false),
    ("Print Screen", "PrintScreen",  Input::Key(Key::PrintScreen),  false),
    ("Scroll Lock",  "ScrollLock",   Input::Key(Key::ScrollLock),   false),
    ("Pause / Break","Pause",        Input::Key(Key::Pause),        false),
    // Arrows.
    ("Arrow Up",    "ArrowUp",    Input::Key(Key::UpArrow),    true),
    ("Arrow Down",  "ArrowDown",  Input::Key(Key::DownArrow),  true),
    ("Arrow Left",  "ArrowLeft",  Input::Key(Key::LeftArrow),  true),
    ("Arrow Right", "ArrowRight", Input::Key(Key::RightArrow), true),
    // Letters A-Z — heavy risk since users type these constantly. Included so
    // press-to-bind can resolve them; the UI still asks "are you sure?".
    ("A", "KeyA", Input::Key(Key::KeyA), true), ("B", "KeyB", Input::Key(Key::KeyB), true),
    ("C", "KeyC", Input::Key(Key::KeyC), true), ("D", "KeyD", Input::Key(Key::KeyD), true),
    ("E", "KeyE", Input::Key(Key::KeyE), true), ("F", "KeyF", Input::Key(Key::KeyF), true),
    ("G", "KeyG", Input::Key(Key::KeyG), true), ("H", "KeyH", Input::Key(Key::KeyH), true),
    ("I", "KeyI", Input::Key(Key::KeyI), true), ("J", "KeyJ", Input::Key(Key::KeyJ), true),
    ("K", "KeyK", Input::Key(Key::KeyK), true), ("L", "KeyL", Input::Key(Key::KeyL), true),
    ("M", "KeyM", Input::Key(Key::KeyM), true), ("N", "KeyN", Input::Key(Key::KeyN), true),
    ("O", "KeyO", Input::Key(Key::KeyO), true), ("P", "KeyP", Input::Key(Key::KeyP), true),
    ("Q", "KeyQ", Input::Key(Key::KeyQ), true), ("R", "KeyR", Input::Key(Key::KeyR), true),
    ("S", "KeyS", Input::Key(Key::KeyS), true), ("T", "KeyT", Input::Key(Key::KeyT), true),
    ("U", "KeyU", Input::Key(Key::KeyU), true), ("V", "KeyV", Input::Key(Key::KeyV), true),
    ("W", "KeyW", Input::Key(Key::KeyW), true), ("X", "KeyX", Input::Key(Key::KeyX), true),
    ("Y", "KeyY", Input::Key(Key::KeyY), true), ("Z", "KeyZ", Input::Key(Key::KeyZ), true),
    // Digits — same reasoning as letters.
    ("0", "Digit0", Input::Key(Key::Num0), true), ("1", "Digit1", Input::Key(Key::Num1), true),
    ("2", "Digit2", Input::Key(Key::Num2), true), ("3", "Digit3", Input::Key(Key::Num3), true),
    ("4", "Digit4", Input::Key(Key::Num4), true), ("5", "Digit5", Input::Key(Key::Num5), true),
    ("6", "Digit6", Input::Key(Key::Num6), true), ("7", "Digit7", Input::Key(Key::Num7), true),
    ("8", "Digit8", Input::Key(Key::Num8), true), ("9", "Digit9", Input::Key(Key::Num9), true),
    // Mouse buttons — Middle/Back/Forward sensible; Left/Right the "you sure?"
    // tier. rdev exposes XButtons as Unknown(4)/Unknown(5).
    ("Mouse: Middle click",   "MouseMiddle", Input::Button(Button::Middle),     false),
    ("Mouse: Back button",    "MouseX1",     Input::Button(Button::Unknown(4)), false),
    ("Mouse: Forward button", "MouseX2",     Input::Button(Button::Unknown(5)), false),
    ("Mouse: Left click",     "MouseLeft",   Input::Button(Button::Left),       true),
    ("Mouse: Right click",    "MouseRight",  Input::Button(Button::Right),      true),
];

/// Index into [`SUPPORTED_HOTKEYS`] for a config.toml key name (defaults to
/// Right Ctrl for an unknown/legacy name).
pub fn index_of(name: &str) -> usize {
    SUPPORTED_HOTKEYS
        .iter()
        .position(|(_, cfg_name, _, _)| *cfg_name == name)
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

        // Read the bound input live so a rebind takes effect without a restart.
        let idx = hotkey_idx.load(Ordering::Relaxed).min(SUPPORTED_HOTKEYS.len() - 1);
        let bound = SUPPORTED_HOTKEYS[idx].2;

        // Match either KeyPress/KeyRelease or ButtonPress/ButtonRelease
        // depending on the bound input family. Wrong-family events early-out.
        let pressed = match (bound, event.event_type) {
            (Input::Key(k),    EventType::KeyPress(pk))     if k == pk => true,
            (Input::Key(k),    EventType::KeyRelease(pk))   if k == pk => false,
            (Input::Button(b), EventType::ButtonPress(pb))  if b == pb => true,
            (Input::Button(b), EventType::ButtonRelease(pb)) if b == pb => false,
            _ => return,
        };

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
