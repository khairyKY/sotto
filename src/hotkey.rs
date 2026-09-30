use crossbeam_channel::Sender;
use rdev::{listen, Button, EventType, Key};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::config::{ActivationMode, Transform};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DictationEvent {
    Start,
    Stop,
    /// Abort the current cycle — user pressed Escape. The worker stops any
    /// in-flight capture, stashes the take for a possible retry, and shows a
    /// "cancelled" state on the overlay.
    Cancel,
    /// Re-run the pipeline from the stashed take (tray "Retry last
    /// dictation", and later the overlay's ↺ button). Never emitted by the
    /// key listener itself.
    Retry,
    /// Re-run the current polish tier + dictionary over existing text (the
    /// ↻ on a history row) and put the result on the clipboard.
    Repolish(String),
    /// Drop the stashed take without retrying it — Home's "Dismiss". Only the
    /// worker owns the stash, so this has to travel the same channel.
    Dismiss,
    /// A Transform chord fired (#18): rewrite the focused app's selection.
    /// Travels the same channel so it queues behind any take being delivered.
    Transform(Transform),
    /// The Scratchpad (#19): its chord, or a row's "inject" from the page.
    Scratchpad(crate::scratchpad::Action),
    /// Lecture capture (#23) on or off, from the tray menu or Home. Never
    /// from a key: it has no hotkey, so it can't collide with dictation's.
    Lecture(bool),
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

/// Modifiers a Transform chord holds. Side-blind: Left and Right Ctrl are the
/// same Ctrl here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
}

/// A parsed Transform chord: exactly these modifiers, plus `key`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chord {
    pub mods: Mods,
    pub key: Key,
}

fn is_modifier(k: Key) -> bool {
    matches!(
        k,
        Key::ControlLeft | Key::ControlRight | Key::Alt | Key::AltGr | Key::ShiftLeft | Key::ShiftRight
            | Key::MetaLeft | Key::MetaRight | Key::CapsLock
    )
}

/// Parse a Transform chord, "Ctrl+Alt+Digit1": modifiers in any order, then
/// one keyboard key from [`SUPPORTED_HOTKEYS`]. `None` for anything the
/// listener can't match, and for a chord without Ctrl, Alt or Win, which would
/// fire while you type (Shift+P is just a capital P).
pub fn parse_chord(s: &str) -> Option<Chord> {
    let mut parts: Vec<&str> = s.split('+').map(str::trim).collect();
    let key_name = parts.pop()?;
    let mut mods = Mods::default();
    for p in parts {
        match p.to_ascii_lowercase().as_str() {
            "ctrl" => mods.ctrl = true,
            "alt" => mods.alt = true,
            "shift" => mods.shift = true,
            "win" => mods.win = true,
            _ => return None,
        }
    }
    let key = SUPPORTED_HOTKEYS.iter().find_map(|(_, name, input, _)| match input {
        Input::Key(k) if *name == key_name && !is_modifier(*k) => Some(*k),
        _ => None,
    })?;
    (mods.ctrl || mods.alt || mods.win).then_some(Chord { mods, key })
}

/// The chord modifiers among `held` keys. The dictation hotkey never counts:
/// with Right Ctrl bound to dictation, Right Ctrl+Alt+1 is a dictation with a
/// stray Alt+1, and only Left Ctrl+Alt+1 is the chord. That's what keeps the
/// two from firing off one key press.
fn mods_from(held: &[Key], bound: Input) -> Mods {
    let on = |a: Key, b: Key| held.iter().any(|&k| (k == a || k == b) && Input::Key(k) != bound);
    Mods {
        ctrl: on(Key::ControlLeft, Key::ControlRight),
        alt: on(Key::Alt, Key::AltGr),
        shift: on(Key::ShiftLeft, Key::ShiftRight),
        win: on(Key::MetaLeft, Key::MetaRight),
    }
}

/// The modifiers physically down right now, asked of the OS rather than
/// tracked from hook events: the listener skips every event while
/// `suppressed` is set, so a release that lands then would leave a tracked
/// modifier stuck down (and a later plain key would match a chord).
fn held_mods(bound: Input) -> Mods {
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    const VKS: [(Key, VIRTUAL_KEY); 8] = [
        (Key::ControlLeft, VK_LCONTROL), (Key::ControlRight, VK_RCONTROL),
        (Key::Alt, VK_LMENU), (Key::AltGr, VK_RMENU),
        (Key::ShiftLeft, VK_LSHIFT), (Key::ShiftRight, VK_RSHIFT),
        (Key::MetaLeft, VK_LWIN), (Key::MetaRight, VK_RWIN),
    ];
    let held: Vec<Key> = VKS
        .iter()
        .filter(|(_, vk)| unsafe { GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000 != 0 })
        .map(|&(k, _)| k)
        .collect();
    mods_from(&held, bound)
}

/// Transform chords' press/release state. A chord fires on the RELEASE of
/// its key: by then the user has let go of most of the chord (a Ctrl+C sent
/// under a still-held Alt is Ctrl+Alt+C, not a copy), and OS auto-repeat of
/// the held key can't fire it twice. The press arms it.
#[derive(Default)]
struct ChordKey {
    armed: Option<(usize, Key)>,
}

impl ChordKey {
    /// Returns `(consumed, fire)`. A consumed event belongs to a chord and
    /// must not also reach the dictation hotkey. `mods` is only asked for
    /// when `key` is some chord's key. `blocked` (a take recording, or
    /// paused) still consumes the chord, it just never fires.
    fn on_key(
        &mut self,
        key: Key,
        pressed: bool,
        mods: impl FnOnce() -> Mods,
        chords: &[Option<Chord>],
        blocked: bool,
    ) -> (bool, Option<usize>) {
        if !pressed {
            return match self.armed {
                Some((i, k)) if k == key => {
                    self.armed = None;
                    (true, (!blocked).then_some(i))
                }
                _ => (false, None),
            };
        }
        if self.armed.is_some_and(|(_, k)| k == key) {
            return (true, None); // OS auto-repeat while held
        }
        if !chords.iter().flatten().any(|c| c.key == key) {
            return (false, None);
        }
        let mods = mods();
        match chords.iter().position(|c| *c == Some(Chord { mods, key })) {
            Some(i) => {
                self.armed = Some((i, key));
                (true, None)
            }
            None => (false, None),
        }
    }
}

/// Toggle-mode's press/release state machine, split out from `run_listener`
/// so the repeat-guard is unit-testable without a real OS hook (same reason
/// `main.rs`'s `anchor_xy`/`chunk_cut` are split from their callers).
///
/// Only a genuine down-then-up transition flips `is_active` — not every
/// `pressed` event. Windows repeats `KeyPress` for every OS auto-repeat tick
/// while a key stays down (confirmed: rdev's low-level hook maps
/// `WM_KEYDOWN` straight to `KeyPress`, no repeat filtering), so without this
/// guard, holding the bound key even slightly past the repeat delay flipped
/// `is_active` several times before the matching release — whether Stop
/// actually landed then depended on the parity of however many repeats
/// fired, which is exactly the "sometimes I need to press it twice" bug this
/// fixes. Mirrors Hold mode's existing `!is_held` guard, which was already
/// immune since it only reacts to `pressed && !is_held`.
///
/// Returns the new `(is_down, is_active)` state and the event to send, if
/// any.
fn toggle_step(
    pressed: bool,
    is_down: bool,
    is_active: bool,
    paused: bool,
) -> (bool, bool, Option<DictationEvent>) {
    if !pressed {
        return (false, is_active, None); // release: just clear is_down
    }
    if is_down {
        return (is_down, is_active, None); // OS auto-repeat while held — not a new press
    }
    if !is_active && paused {
        // A genuine press, but pausing blocks *starting* a new dictation —
        // still record that the key is down, or the eventual release would
        // leave is_down stuck true and swallow the next real press.
        return (true, is_active, None);
    }
    let new_active = !is_active;
    let ev = Some(if new_active { DictationEvent::Start } else { DictationEvent::Stop });
    (true, new_active, ev)
}

/// Toggle-mode state the listener keeps between hook events.
///
/// Whether a press means Start or Stop comes from `listening` (the pipeline's
/// own flag), never from a private copy here: a take can also be started by
/// a click on the overlay pill or the trainer button, which this listener
/// never sees. With a private copy the first press after a click-started
/// take sent a second Start, and a Start mid-take wiped the take (#37).
///
/// ponytail: `listening` flips when the audio thread actually opens the mic,
/// so a double-tap faster than that (~100 ms) reads the second press as
/// another Start (ignored mid-take, see the Start guard in main.rs); one
/// more press then stops it. Nothing is lost. Track a pending Start here if
/// that ever matters.
#[derive(Default)]
struct ToggleKey {
    is_down: bool,
}

impl ToggleKey {
    fn on_key(&mut self, pressed: bool, listening: bool, paused: bool) -> Option<DictationEvent> {
        let (down, _, ev) = toggle_step(pressed, self.is_down, listening, paused);
        self.is_down = down;
        ev
    }
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
    cancelled: Arc<AtomicBool>,
    listening: Arc<AtomicBool>,
    transforms: Arc<Mutex<Vec<Transform>>>,
    transforms_enabled: Arc<AtomicBool>,
) {
    let mut is_held = false;
    let mut toggle = ToggleKey::default();
    let mut chord_key = ChordKey::default();
    let mut pad_key = ChordKey::default();

    let callback = move |event: rdev::Event| {
        if suppressed.load(Ordering::SeqCst) {
            return;
        }

        // Escape aborts an in-flight dictation regardless of what the bound
        // hotkey is — same key on every keyboard, and the pipeline handler
        // ignores the event if nothing is happening. The flag is set HERE,
        // from the listener thread, because the worker is single-threaded:
        // while it's blocked inside ASR or the LLM it can't process a Cancel
        // event, but it does check this flag at every stage boundary — so a
        // mid-transcription Escape still stops the polish + injection. The
        // channel event additionally handles the "cancel while recording"
        // path (stop the recorder, stash the take).
        if let EventType::KeyPress(rdev::Key::Escape) = event.event_type {
            cancelled.store(true, Ordering::SeqCst);
            let _ = tx.send(DictationEvent::Cancel);
            // Fall through — Escape could ALSO match a bound hotkey. Almost
            // never the case, but the logic below handles it cleanly.
        }

        // Read the bound input live so a rebind takes effect without a restart.
        let idx = hotkey_idx.load(Ordering::Relaxed).min(SUPPORTED_HOTKEYS.len() - 1);
        let bound = SUPPORTED_HOTKEYS[idx].2;

        let key_event = match event.event_type {
            EventType::KeyPress(k) => Some((k, true)),
            EventType::KeyRelease(k) => Some((k, false)),
            _ => None,
        };

        // The Scratchpad chord (#19), by a Transform chord's rules. Not
        // blocked mid-take: a take lands where it was spoken, pad or not.
        if let Some((key, pressed)) = key_event {
            let chord = [crate::scratchpad::chord()];
            let (consumed, fire) = pad_key.on_key(key, pressed, || held_mods(bound), &chord, false);
            if fire.is_some() {
                let _ = tx.send(DictationEvent::Scratchpad(crate::scratchpad::Action::Toggle));
            }
            if consumed {
                return;
            }
        }

        // Then Transform chords (#18): a chord's own key press/release is
        // theirs, never also the dictation hotkey's. Never fires mid-take.
        // ponytail: chords re-parsed per key event; a handful of short strings.
        if transforms_enabled.load(Ordering::Relaxed) {
            if let Some((key, pressed)) = key_event {
                let list = transforms.lock().unwrap();
                let chords: Vec<Option<Chord>> = list.iter().map(|t| parse_chord(&t.chord)).collect();
                let blocked = listening.load(Ordering::Relaxed) || paused.load(Ordering::Relaxed);
                let (consumed, fire) = chord_key.on_key(key, pressed, || held_mods(bound), &chords, blocked);
                // `get`: the list can be edited between a chord's press and release.
                if let Some(t) = fire.and_then(|i| list.get(i)) {
                    let _ = tx.send(DictationEvent::Transform(t.clone()));
                }
                if consumed {
                    return;
                }
            }
        }

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
                let ev = toggle.on_key(
                    pressed,
                    listening.load(Ordering::Relaxed),
                    paused.load(Ordering::Relaxed),
                );
                if let Some(ev) = ev {
                    let _ = tx.send(ev);
                }
            }
        }
    };

    if let Err(err) = listen(callback) {
        tracing::error!(?err, "rdev global hotkey listener stopped unexpectedly");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_press_release_toggles_once() {
        let (down, active, ev) = toggle_step(true, false, false, false);
        assert!(down);
        assert!(active);
        assert_eq!(ev, Some(DictationEvent::Start));

        let (down, active, ev) = toggle_step(false, down, active, false);
        assert!(!down);
        assert!(active); // release never flips is_active, only clears is_down
        assert_eq!(ev, None);

        let (down, active, ev) = toggle_step(true, down, active, false);
        assert!(down);
        assert!(!active);
        assert_eq!(ev, Some(DictationEvent::Stop));
    }

    #[test]
    fn os_auto_repeat_while_held_does_not_re_toggle() {
        // The actual bug: a held key firing 3 raw KeyPress events (one real
        // + two OS repeats) before the matching release must still toggle
        // exactly once, not three times.
        let (down, active, ev) = toggle_step(true, false, false, false);
        assert_eq!((down, active, ev), (true, true, Some(DictationEvent::Start)));

        let (down, active, ev) = toggle_step(true, down, active, false); // repeat #1
        assert_eq!((down, active, ev), (true, true, None));
        let (down, active, ev) = toggle_step(true, down, active, false); // repeat #2
        assert_eq!((down, active, ev), (true, true, None));

        let (down, active, ev) = toggle_step(false, down, active, false); // real release
        assert_eq!((down, active, ev), (false, true, None));
    }

    #[test]
    fn release_with_no_prior_press_is_a_no_op() {
        assert_eq!(toggle_step(false, false, false, false), (false, false, None));
    }

    #[test]
    fn paused_blocks_starting_but_still_tracks_the_key_as_down() {
        // Regression guard for the stuck-is_down trap: if the paused branch
        // didn't record is_down, the matching release would land on a
        // "was never down" state, and a later un-paused press would be
        // treated as a repeat of a press that never happened.
        let (down, active, ev) = toggle_step(true, false, false, true);
        assert_eq!((down, active, ev), (true, false, None));
        let (down, active, ev) = toggle_step(false, down, active, true);
        assert_eq!((down, active, ev), (false, false, None));
    }

    #[test]
    fn a_take_started_by_click_is_stopped_by_the_first_toggle_press() {
        // #37: the overlay pill and the trainer button start a take without
        // the hotkey. The first press must stop it, not send a second Start
        // (which restarts the recorder and wipes the take).
        let mut key = ToggleKey::default();
        assert_eq!(key.on_key(true, /* pipeline listening */ true, false), Some(DictationEvent::Stop));
        // ...and after it has stopped, the next press starts a new take.
        assert_eq!(key.on_key(false, true, false), None);
        assert_eq!(key.on_key(true, /* stopped */ false, false), Some(DictationEvent::Start));
    }

    const CTRL_ALT: Mods = Mods { ctrl: true, alt: true, shift: false, win: false };

    #[test]
    fn parses_chords_and_refuses_ones_that_would_fire_while_typing() {
        assert_eq!(parse_chord("Ctrl+Alt+Digit1"), Some(Chord { mods: CTRL_ALT, key: Key::Num1 }));
        assert_eq!(parse_chord(" alt + ctrl + KeyP "), Some(Chord { mods: CTRL_ALT, key: Key::KeyP }));
        assert_eq!(parse_chord("Win+Shift+F9").map(|c| (c.mods.win, c.mods.shift, c.key)), Some((true, true, Key::F9)));
        for bad in ["", "KeyP", "Shift+KeyP", "Ctrl+Alt+ControlLeft", "Ctrl+Alt+MouseMiddle", "Ctrl+Alt+Nope", "Hyper+KeyP"] {
            assert_eq!(parse_chord(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_dictation_hotkey_never_counts_as_a_chord_modifier() {
        let held = [Key::ControlRight, Key::Alt];
        // Right Ctrl bound to dictation: Right Ctrl+Alt is just Alt to chords.
        assert_eq!(mods_from(&held, Input::Key(Key::ControlRight)), Mods { alt: true, ..Mods::default() });
        // Anything else bound: it's Ctrl+Alt, whichever side.
        assert_eq!(mods_from(&held, Input::Key(Key::F9)), CTRL_ALT);
        assert_eq!(mods_from(&[Key::ControlLeft, Key::AltGr], Input::Key(Key::ControlRight)), CTRL_ALT);
    }

    #[test]
    fn a_chord_arms_on_press_and_fires_once_on_release() {
        let chords = [parse_chord("Ctrl+Alt+Digit1"), parse_chord("Ctrl+Alt+Digit2")];
        let mut ck = ChordKey::default();
        assert_eq!(ck.on_key(Key::Num2, true, || CTRL_ALT, &chords, false), (true, None));
        assert_eq!(ck.on_key(Key::Num2, true, || CTRL_ALT, &chords, false), (true, None)); // auto-repeat
        assert_eq!(ck.on_key(Key::Num2, false, || CTRL_ALT, &chords, false), (true, Some(1)));
        assert_eq!(ck.on_key(Key::Num2, false, || CTRL_ALT, &chords, false), (false, None)); // stray release
    }

    #[test]
    fn plain_typing_and_wrong_modifiers_pass_through_untouched() {
        let chords = [parse_chord("Ctrl+Alt+Digit1")];
        let mut ck = ChordKey::default();
        // A key no chord uses never even asks for the modifier state.
        assert_eq!(ck.on_key(Key::KeyA, true, || unreachable!(), &chords, false), (false, None));
        // Ctrl+1 alone is someone else's shortcut.
        let ctrl = Mods { ctrl: true, ..Mods::default() };
        assert_eq!(ck.on_key(Key::Num1, true, || ctrl, &chords, false), (false, None));
        assert_eq!(ck.on_key(Key::Num1, false, || ctrl, &chords, false), (false, None));
        // Ctrl+Alt+Shift+1 isn't Ctrl+Alt+1 either: modifiers match exactly.
        let more = Mods { shift: true, ..CTRL_ALT };
        assert_eq!(ck.on_key(Key::Num1, true, || more, &chords, false), (false, None));
    }

    #[test]
    fn a_chord_never_fires_while_a_take_is_recording() {
        let chords = [parse_chord("Ctrl+Alt+Digit1")];
        let mut ck = ChordKey::default();
        ck.on_key(Key::Num1, true, || CTRL_ALT, &chords, true);
        // Still consumed, so the key can't leak to the dictation hotkey, but no fire.
        assert_eq!(ck.on_key(Key::Num1, false, || CTRL_ALT, &chords, true), (true, None));
    }

    #[test]
    fn a_chord_on_the_dictation_key_keeps_both_press_and_release_from_it() {
        // Dictation on F9, a chord on Ctrl+Alt+F9: the chord's press and
        // release are consumed, so the dictation hotkey never sees either.
        let chords = [parse_chord("Ctrl+Alt+F9")];
        let mut ck = ChordKey::default();
        let mods = || mods_from(&[Key::ControlLeft, Key::Alt], Input::Key(Key::F9));
        assert_eq!(ck.on_key(Key::F9, true, mods, &chords, false), (true, None));
        assert_eq!(ck.on_key(Key::F9, false, mods, &chords, false), (true, Some(0)));
        // ...and a bare F9 still goes to dictation.
        assert_eq!(ck.on_key(Key::F9, true, Mods::default, &chords, false), (false, None));
    }

    #[test]
    fn paused_does_not_block_stopping_an_already_active_dictation() {
        // "Pausing only blocks *starting* a new dictation" — an in-flight
        // recording (is_active already true) must still be stoppable.
        let (down, active, ev) = toggle_step(true, false, true, true);
        assert_eq!((down, active, ev), (true, false, Some(DictationEvent::Stop)));
    }
}
