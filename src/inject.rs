use crate::config::InjectionMode;
use std::mem::size_of;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, VIRTUAL_KEY, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VK_BACK, VK_C, VK_CONTROL, VK_INSERT, VK_LEFT,
    VK_LWIN, VK_MENU, VK_RETURN, VK_RIGHT, VK_RWIN, VK_SHIFT, VK_V,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId, IsWindow, SetForegroundWindow,
};

/// Read the currently-focused window's HWND so it can be restored later
/// (before injection) even if the user Alt-Tabbed while speaking. Returns 0
/// if capture fails, which downstream treats as "just inject wherever the
/// user is now" — the pre-fix behavior.
pub fn capture_focus() -> isize {
    unsafe { GetForegroundWindow().0 as isize }
}

/// Bring `target_hwnd` back to the foreground before injection. Windows
/// refuses `SetForegroundWindow` from a non-foreground process by default —
/// the reliable workaround is to attach our input queue to the target's
/// thread, do the call, then detach. Failure is silent: injection then
/// lands in whatever window is currently focused (still functional, just
/// not the intended target).
pub fn restore_focus(target_hwnd: isize) {
    if target_hwnd == 0 {
        return;
    }
    let target = HWND(target_hwnd as *mut _);
    unsafe {
        if !IsWindow(Some(target)).as_bool() {
            // Target window closed between Start and now; nothing to do.
            return;
        }
        let target_thread = GetWindowThreadProcessId(target, None);
        let current_thread = GetCurrentThreadId();
        let attach = target_thread != 0
            && target_thread != current_thread
            && AttachThreadInput(current_thread, target_thread, true).as_bool();
        let _ = SetForegroundWindow(target);
        if attach {
            let _ = AttachThreadInput(current_thread, target_thread, false);
        }
    }
    // Small settle so the target's message pump processes the activation
    // before we start feeding it keystrokes.
    std::thread::sleep(Duration::from_millis(25));
}

/// Injects `text` into whatever window currently has keyboard focus, using the
/// configured strategy.
pub fn inject_text(text: &str, mode: InjectionMode) -> anyhow::Result<()> {
    let target = ForegroundTarget::capture();
    tracing::debug!(?target, chars = text.chars().count(), ?mode, "injecting text");

    // The hotkey's release event fires slightly before Windows' own modifier
    // state (GetAsyncKeyState) settles. If a modifier is still logically down
    // when we start feeding KEYEVENTF_UNICODE events, the receiving app can
    // misinterpret some of them as Ctrl/Alt/Shift-chords instead of literal
    // characters, corrupting the output. Wait briefly for the coast to clear.
    if !wait_for_modifiers_released(Duration::from_millis(150)) {
        tracing::warn!("modifier key still down after settle timeout — injecting anyway");
    }

    match mode {
        InjectionMode::Unicode => inject_unicode(text),
        InjectionMode::Paste => inject_via_paste(text),
    }
}

/// Wait up to `timeout` for Ctrl/Shift/Alt/Win to be let go. False if one is
/// still down when it runs out.
pub fn wait_for_modifiers_released(timeout: Duration) -> bool {
    const MODIFIERS: [VIRTUAL_KEY; 5] = [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN];
    let deadline = Instant::now() + timeout;

    loop {
        let any_down = MODIFIERS
            .iter()
            .any(|&vk| unsafe { (GetAsyncKeyState(vk.0 as i32) as u16) & 0x8000 != 0 });

        if !any_down {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(4));
    }
}

/// Whether the focused window is one of Sotto's own (the settings window,
/// say), where a Transform chord has nothing to rewrite.
pub fn foreground_is_ours() -> bool {
    ForegroundTarget::capture().process_id == std::process::id()
}

// Fields are only read through the derived `Debug` (in the tracing call), which
// the dead-code lint doesn't count — silence it rather than drop the diagnostics.
#[derive(Debug)]
#[allow(dead_code)]
struct ForegroundTarget {
    hwnd: isize,
    process_id: u32,
}

impl ForegroundTarget {
    fn capture() -> Self {
        unsafe {
            let hwnd: HWND = GetForegroundWindow();
            let mut process_id = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut process_id));
            Self {
                hwnd: hwnd.0 as isize,
                process_id,
            }
        }
    }
}

/// Simulates raw Unicode keystrokes via `SendInput`, bypassing the active
/// keyboard layout / IME entirely. Handles characters outside the Basic
/// Multilingual Plane (e.g. emoji) by sending their UTF-16 surrogate pairs as
/// two consecutive events, which Windows recombines on the receiving end.
///
/// Sent one UTF-16 unit per `SendInput` call, not batched into a single giant
/// array. Submitting the whole string as one call was measured to corrupt
/// output in real apps (Notepad): the receiving window's message loop can't
/// keep up, key-up events get coalesced, and a character gets "stuck" and
/// auto-repeats for the rest of the string. A 1ms gap between units avoids it
/// with negligible cost (≤ ~200ms for a 200-character dictation).
fn inject_unicode(text: &str) -> anyhow::Result<()> {
    let mut buf = [0u16; 2];

    for ch in text.chars() {
        for &unit in ch.encode_utf16(&mut buf).iter() {
            let events = [unicode_input(unit, false), unicode_input(unit, true)];
            let sent = unsafe { SendInput(&events, size_of::<INPUT>() as i32) };
            if sent as usize != events.len() {
                anyhow::bail!(
                    "SendInput only accepted {sent}/{} events for unit {unit:#06x} (last_error={:?})",
                    events.len(),
                    unsafe { windows::Win32::Foundation::GetLastError() }
                );
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    Ok(())
}

fn unicode_input(utf16_unit: u16, key_up: bool) -> INPUT {
    let mut flags = KEYEVENTF_UNICODE;
    if key_up {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: utf16_unit,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Fallback for apps that mangle raw `KEYEVENTF_UNICODE` input (rare, but seen
/// in some terminal emulators / games). Swaps the clipboard, sends a real
/// Ctrl+V, then restores whatever was on the clipboard before.
fn inject_via_paste(text: &str) -> anyhow::Result<()> {
    let mut clipboard = arboard::Clipboard::new()?;
    let previous = clipboard.get_text().ok();

    clipboard.set_text(text.to_string())?;
    std::thread::sleep(Duration::from_millis(30));

    send_ctrl(VK_V)?;
    std::thread::sleep(Duration::from_millis(80));

    match previous {
        Some(prev) => {
            let _ = clipboard.set_text(prev);
        }
        None => {
            let _ = clipboard.clear();
        }
    }
    Ok(())
}

/// Ctrl+C into the focused window: a Transform's selection grab (#18). Same
/// VK-only key path as the paste; the spike found it copies in Notepad, VS Code
/// and an Edge text field alike, so no per-app fallback.
pub fn send_ctrl_c() -> anyhow::Result<()> {
    send_ctrl(VK_C)
}

/// Ctrl+Insert: a copy that is never an interrupt. A voice correction's (#20)
/// check of what `select_back` grabbed, where Ctrl+C would stop whatever a
/// terminal is running.
pub fn send_ctrl_insert() -> anyhow::Result<()> {
    send_ctrl(VK_INSERT)
}

/// Shift+Left × `n`: select the `n` characters before the caret, a voice
/// correction's re-grab of the last injection. One key per `SendInput` with
/// a 1ms gap, for the same reason as `inject_unicode`.
pub fn select_back(n: usize) -> anyhow::Result<()> {
    send(&[vk_input(VK_SHIFT, false)])?;
    let walked = press(VK_LEFT, n);
    send(&[vk_input(VK_SHIFT, true)])?;
    walked
}

/// Right × `n`: 1 collapses a selection to its end.
pub fn press_right(n: usize) -> anyhow::Result<()> {
    press(VK_RIGHT, n)
}

/// Enter: an auto-send (#21) after a dictation lands.
pub fn press_enter() -> anyhow::Result<()> {
    press(VK_RETURN, 1)
}

/// Backspace: a backtrack (#26) deleting the re-selected dictation.
pub fn press_backspace() -> anyhow::Result<()> {
    press(VK_BACK, 1)
}

fn press(key: VIRTUAL_KEY, n: usize) -> anyhow::Result<()> {
    for _ in 0..n {
        send(&[vk_input(key, false), vk_input(key, true)])?;
        std::thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}

fn send_ctrl(key: VIRTUAL_KEY) -> anyhow::Result<()> {
    send(&[
        vk_input(VK_CONTROL, false),
        vk_input(key, false),
        vk_input(key, true),
        vk_input(VK_CONTROL, true),
    ])
}

fn send(events: &[INPUT]) -> anyhow::Result<()> {
    let sent = unsafe { SendInput(events, size_of::<INPUT>() as i32) };
    if sent as usize != events.len() {
        anyhow::bail!("SendInput only accepted {sent}/{}", events.len());
    }
    Ok(())
}

fn vk_input(vk: VIRTUAL_KEY, key_up: bool) -> INPUT {
    let mut flags = if key_up { KEYEVENTF_KEYUP } else { Default::default() };
    // Arrows and Insert live on the extended keypad. Without the flag they
    // arrive as their number-pad twins, which NumLock can turn into digits
    // and a held Shift into plain arrows.
    if matches!(vk, VK_LEFT | VK_RIGHT | VK_INSERT) {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}
