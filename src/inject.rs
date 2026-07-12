use crate::config::InjectionMode;
use std::mem::size_of;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, VIRTUAL_KEY, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT, VK_V,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

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
    wait_for_modifiers_released();

    match mode {
        InjectionMode::Unicode => inject_unicode(text),
        InjectionMode::Paste => inject_via_paste(text),
    }
}

fn wait_for_modifiers_released() {
    const MODIFIERS: [VIRTUAL_KEY; 5] = [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN];
    let deadline = Instant::now() + Duration::from_millis(150);

    loop {
        let any_down = MODIFIERS
            .iter()
            .any(|&vk| unsafe { (GetAsyncKeyState(vk.0 as i32) as u16) & 0x8000 != 0 });

        if !any_down {
            return;
        }
        if Instant::now() >= deadline {
            tracing::warn!("modifier key still down after settle timeout — injecting anyway");
            return;
        }
        std::thread::sleep(Duration::from_millis(4));
    }
}

#[derive(Debug)]
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

    send_ctrl_v()?;
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

fn send_ctrl_v() -> anyhow::Result<()> {
    let events = [
        vk_input(VK_CONTROL, false),
        vk_input(VK_V, false),
        vk_input(VK_V, true),
        vk_input(VK_CONTROL, true),
    ];
    let sent = unsafe { SendInput(&events, size_of::<INPUT>() as i32) };
    if sent as usize != events.len() {
        anyhow::bail!("SendInput (ctrl+v) only accepted {sent}/{}", events.len());
    }
    Ok(())
}

fn vk_input(vk: VIRTUAL_KEY, key_up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if key_up { KEYEVENTF_KEYUP } else { Default::default() },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}
