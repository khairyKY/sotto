//! Auto tone (#28): a short look at what's already in the focused text field,
//! so AI polish can match its register (a chat, an email, a code comment).
//! Read through UI Automation when a take starts, the way `focus_target` is
//! captured. No screenshots, and nothing leaves the machine: the text only
//! goes into the local sidecar's prompt. It's never logged (#48) or written
//! anywhere; the take carries it in memory and drops it with the take.

use std::sync::mpsc;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomation2, IUIAutomationElement, IUIAutomationTextPattern,
    IUIAutomationValuePattern, TextPatternRangeEndpoint_End, TextPatternRangeEndpoint_Start, TextUnit_Character,
    UIA_TextPatternId, UIA_ValuePatternId,
};
use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
use windows::core::Interface;

/// How much of the field before the caret, in characters.
const MAX_CHARS: usize = 300;
/// A read slower than this is dropped: the context is a nicety, never worth
/// waiting on a busy app for.
const BUDGET: Duration = Duration::from_millis(50);

/// Read the field in `hwnd` on a worker thread; the capture path never waits
/// on it. The receiver yields the tidied context only if the read finished
/// inside `BUDGET`, otherwise nothing and the take polishes as it always did.
/// A late or failed read is silent.
pub fn spawn_read(hwnd: isize) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    let started = Instant::now();
    let _ = std::thread::Builder::new().name("uia-context".into()).spawn(move || {
        let app = crate::stats::app_name(hwnd);
        let text = read_focused(hwnd, |is_password| may_read(&app, is_password));
        if let Some(text) = text.as_deref().and_then(tidy) {
            if started.elapsed() <= BUDGET {
                let _ = tx.send(text);
            }
        }
    });
    rx
}

/// Never a password field, and never a terminal (the `is_terminal` rule):
/// a console's "field" is its scrollback, commands and output and all.
fn may_read(app: &str, is_password: bool) -> bool {
    !is_password && !crate::correction::is_terminal(app)
}

/// The tail of `text` as prompt-ready context: its last `MAX_CHARS`, a word
/// cut in half at the front dropped, control and embedded-object characters
/// gone, and every whitespace run (line breaks too) one space, so the field
/// can't reshape the prompt around it. `None` when nothing is left.
fn tidy(text: &str) -> Option<String> {
    let start = text.char_indices().rev().nth(MAX_CHARS - 1).map_or(0, |(i, _)| i);
    let mid_word = text[..start].ends_with(|c: char| !c.is_whitespace())
        && text[start..].starts_with(|c: char| !c.is_whitespace());
    let clean: String =
        text[start..].chars().map(|c| if c.is_control() || c == '\u{FFFC}' { ' ' } else { c }).collect();
    let words: Vec<&str> = clean.split_whitespace().skip(mid_word as usize).collect();
    (!words.is_empty()).then(|| words.join(" "))
}

/// The focused element's text before the caret, if it lives in `hwnd`'s
/// process and `allowed(is_password)` says yes. The process check keeps the
/// read on the app `allowed` judged, should focus move in the meantime.
fn read_focused(hwnd: isize, allowed: impl FnOnce(bool) -> bool) -> Option<String> {
    with_uia(|uia| unsafe {
        let el = uia.GetFocusedElement()?;
        let mut pid = 0;
        GetWindowThreadProcessId(HWND(hwnd as *mut _), Some(&mut pid));
        if pid == 0 || el.CurrentProcessId()? as u32 != pid || !allowed(el.CurrentIsPassword()?.as_bool()) {
            return Ok(None);
        }
        Ok(before_caret(&el))
    })
}

/// Run `f` with a UI Automation client, COM set up and torn down around it.
/// Every cross-process call is bounded by `BUDGET` too, so a hung app can't
/// hold the thread for UIA's default seconds.
fn with_uia<T>(f: impl FnOnce(&IUIAutomation) -> windows::core::Result<Option<T>>) -> Option<T> {
    unsafe {
        let init = CoInitializeEx(None, COINIT_MULTITHREADED);
        let out = CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok().and_then(|uia| {
            if let Ok(uia2) = uia.cast::<IUIAutomation2>() {
                let ms = BUDGET.as_millis() as u32;
                let _ = uia2.SetConnectionTimeout(ms);
                let _ = uia2.SetTransactionTimeout(ms);
            }
            f(&uia).ok().flatten()
        });
        if init.is_ok() {
            CoUninitialize();
        }
        out
    }
}

/// Up to `MAX_CHARS` before the caret through the Text pattern (collapse the
/// selection to its start, walk back). A field with only the Value pattern
/// gives its whole value; `tidy` keeps the tail, where typing usually is.
fn before_caret(el: &IUIAutomationElement) -> Option<String> {
    unsafe {
        let via_text = || -> windows::core::Result<String> {
            let text = el.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)?;
            let range = text.GetSelection()?.GetElement(0)?;
            range.MoveEndpointByRange(TextPatternRangeEndpoint_End, &range, TextPatternRangeEndpoint_Start)?;
            range.MoveEndpointByUnit(TextPatternRangeEndpoint_Start, TextUnit_Character, -(MAX_CHARS as i32))?;
            Ok(range.GetText(-1)?.to_string())
        };
        let via_value = || -> windows::core::Result<String> {
            Ok(el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)?.CurrentValue()?.to_string())
        };
        via_text().or_else(|_| via_value()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tidy_keeps_the_tail_whole_words_and_one_line() {
        assert_eq!(tidy("Hi Sam,\r\n\r\n\tThanks for the notes."), Some("Hi Sam, Thanks for the notes.".into()));
        // Longer than MAX_CHARS: the tail, minus the word the cut went through.
        let long = format!("{} ok see you soon", "abc".repeat(200));
        assert_eq!(tidy(&long).as_deref(), Some("ok see you soon"));
        let long = format!("{} {}", "x".repeat(50), "word ".repeat(80));
        let got = tidy(&long).unwrap();
        assert!(got.chars().count() <= MAX_CHARS && got.split(' ').all(|w| w == "word"), "{got}");
        // A cut that lands on a word boundary keeps the first word.
        assert_eq!(tidy(&format!("{}{}", "a ".repeat(10), "b".repeat(MAX_CHARS))), Some("b".repeat(MAX_CHARS)));
        // Control and embedded-object characters never reach the prompt.
        assert_eq!(tidy("see\u{0}you\u{FFFC}there\u{1b}[0m"), Some("see you there [0m".into()));
        // Arabic is counted in characters, not bytes, and kept intact.
        assert_eq!(tidy("  أهلا يا سام  "), Some("أهلا يا سام".into()));
        assert_eq!(tidy(" \r\n\u{FFFC} "), None);
        assert_eq!(tidy(""), None);
    }

    #[test]
    fn never_reads_a_password_field_or_a_terminal() {
        assert!(may_read("Notepad", false));
        assert!(may_read("Edge", false));
        assert!(!may_read("Edge", true));
        for app in ["Terminal", "cmd", "powershell", "pwsh", "OpenConsole", "WEZTERM-GUI"] {
            assert!(!may_read(app, false), "{app}");
        }
    }

    /// The real Text-pattern path without touching focus: our own RichEdit
    /// (the control family Notepad is built on) in an off-screen,
    /// never-activated window, caret set mid-text, read from another thread
    /// (UIA calls must not come from the thread that owns the window). A plain
    /// Win32 Edit only offers the Value pattern here. Run by hand, it prints
    /// the cold and warm read times against `BUDGET`:
    /// `cargo test --bin sotto uia_reads -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn uia_reads_the_text_before_a_rich_edits_caret() {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::System::Threading::GetCurrentThreadId;
        use windows::Win32::UI::WindowsAndMessaging::*;
        use windows::core::w;
        const EM_SETSEL: u32 = 0x00B1;
        // Longer than MAX_CHARS before the caret, so the walk-back is bounded too.
        let before = format!("{}Dear Ms. Rivera, thank you for the quick reply.", "An earlier line. ".repeat(20));
        let caret = before.len();
        let text = windows::core::HSTRING::from(format!("{before} AFTER THE CARET"));
        let (tx, rx) = mpsc::channel();
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn LoadLibraryW(name: windows::core::PCWSTR) -> isize;
        }
        let owner = std::thread::spawn(move || unsafe {
            LoadLibraryW(w!("Msftedit.dll"));
            let (ex, style) = (WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW, WS_POPUP | WS_VISIBLE);
            let host = CreateWindowExW(ex, w!("STATIC"), w!(""), style, -10000, -10000, 400, 200, None, None, None, None).unwrap();
            let style = WS_CHILD | WS_VISIBLE | WINDOW_STYLE(ES_MULTILINE as u32);
            let edit = CreateWindowExW(WINDOW_EX_STYLE(0), w!("RICHEDIT50W"), &text, style, 0, 0, 400, 200, Some(host), None, None, None).unwrap();
            SendMessageW(edit, EM_SETSEL, Some(WPARAM(caret)), Some(LPARAM(caret as isize)));
            tx.send((edit.0 as isize, GetCurrentThreadId())).unwrap();
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                DispatchMessageW(&msg);
            }
            let _ = DestroyWindow(host);
        });
        let (edit, owner_thread) = rx.recv().unwrap();
        let t = Instant::now();
        let got = with_uia(|uia| unsafe { Ok(before_caret(&uia.ElementFromHandle(HWND(edit as *mut _))?)) });
        println!("cold read in {} ms: {got:?}", t.elapsed().as_millis());
        let t = Instant::now();
        let again = with_uia(|uia| unsafe { Ok(before_caret(&uia.ElementFromHandle(HWND(edit as *mut _))?)) });
        println!("warm read in {} ms", t.elapsed().as_millis());
        assert_eq!(again, got);
        unsafe { PostThreadMessageW(owner_thread, WM_QUIT, WPARAM(0), LPARAM(0)).unwrap() };
        owner.join().unwrap();
        assert_eq!(got, Some(before[before.len() - MAX_CHARS..].to_string()));
    }

    /// Manual probe for the real apps (Notepad, Edge, VS Code): run
    /// `cargo test --bin sotto uia_probe -- --ignored --nocapture`, click into
    /// a text field within 3 s, and it prints what auto tone would send.
    /// Prints the field's text to your terminal, so use sample text.
    #[test]
    #[ignore]
    fn uia_probe_the_focused_field() {
        std::thread::sleep(Duration::from_secs(3));
        let hwnd = crate::inject::capture_focus();
        let app = crate::stats::app_name(hwnd);
        let t = Instant::now();
        let raw = read_focused(hwnd, |is_password| {
            println!("app={app} password={is_password} allowed={}", may_read(&app, is_password));
            may_read(&app, is_password)
        });
        println!("read in {} ms (budget {} ms)", t.elapsed().as_millis(), BUDGET.as_millis());
        println!("raw:  {raw:?}\nsent: {:?}", raw.as_deref().and_then(tidy));
    }
}
