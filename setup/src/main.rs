//! Sotto's setup window. The NSIS installer Tauri builds can't be themed past
//! its images (its buttons, text and frame are Windows' own), so this shows
//! the app's own screens, built from `ui/theme.css`, `ui/index.css` and the
//! bundled fonts, and runs that installer silently (`/S /D=<dir>`) behind them.
//!
//! The installer is embedded at build time from `SOTTO_INSTALLER`, its version
//! from `SOTTO_VERSION` (both set by `scripts/build-setup.ps1`). The updater
//! still ships the plain NSIS installer; this is for first installs.
#![windows_subsystem = "windows"]

use std::borrow::Cow;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::windows::{WindowBuilderExtWindows, WindowExtWindows};
use tao::window::WindowBuilder;
use wry::http::Response;
use wry::WebViewBuilder;

const INSTALLER: &[u8] = include_bytes!(env!("SOTTO_INSTALLER"));
const VERSION: &str = env!("SOTTO_VERSION");
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const UNINSTALL_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Sotto";

/// The page and everything it links, served from inside this exe.
fn asset(path: &str) -> Option<(&'static [u8], &'static str)> {
    macro_rules! font {
        ($f:literal) => {
            (include_bytes!(concat!("../../ui/fonts/", $f)) as &[u8], "font/woff2")
        };
    }
    Some(match path {
        "/" | "/setup.html" => (include_bytes!("../setup.html"), "text/html"),
        "/theme.css" => (include_bytes!("../../ui/theme.css"), "text/css"),
        "/index.css" => (include_bytes!("../../ui/index.css"), "text/css"),
        "/fonts.css" => (include_bytes!("../../ui/fonts.css"), "text/css"),
        "/fonts/hanken-grotesk-400.woff2" => font!("hanken-grotesk-400.woff2"),
        "/fonts/hanken-grotesk-500.woff2" => font!("hanken-grotesk-500.woff2"),
        "/fonts/hanken-grotesk-600.woff2" => font!("hanken-grotesk-600.woff2"),
        "/fonts/hanken-grotesk-700.woff2" => font!("hanken-grotesk-700.woff2"),
        "/fonts/jetbrains-mono-400.woff2" => font!("jetbrains-mono-400.woff2"),
        "/fonts/jetbrains-mono-500.woff2" => font!("jetbrains-mono-500.woff2"),
        "/fonts/newsreader-400.woff2" => font!("newsreader-400.woff2"),
        "/fonts/newsreader-500.woff2" => font!("newsreader-500.woff2"),
        "/fonts/newsreader-600.woff2" => font!("newsreader-600.woff2"),
        _ => return None,
    })
}

enum Ui {
    /// Run this in the page (progress, a picked folder, an error).
    Js(String),
    Drag,
    Minimize,
    Close,
}

fn quiet(cmd: &str) -> Command {
    let mut c = Command::new(cmd);
    c.creation_flags(CREATE_NO_WINDOW);
    c
}

/// A REG_SZ from `reg query`, unquoted. Shelling out to reg.exe, as the
/// app's own `startup.rs` does.
fn reg_value(key: &str, name: &str) -> Option<String> {
    let out = quiet("reg").args(["query", key, "/v", name]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|l| l.trim_start().starts_with(name))?;
    let v = line.split("REG_SZ").nth(1)?.trim().trim_matches('"');
    (!v.is_empty()).then(|| v.to_string())
}

fn desktop_link() -> Option<PathBuf> {
    let home = std::env::var("USERPROFILE").ok()?;
    let one = Path::new(&home).join("OneDrive").join("Desktop");
    let dir = if one.is_dir() { one } else { Path::new(&home).join("Desktop") };
    Some(dir.join("Sotto.lnk"))
}

/// What the page starts from: an existing install's folder and choices, or
/// the defaults for a fresh one.
fn initial_state() -> serde_json::Value {
    let existing = reg_value(UNINSTALL_KEY, "InstallLocation");
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let dir = existing.clone().unwrap_or_else(|| format!(r"{local}\Sotto"));
    let models = std::env::var("APPDATA").map(|a| Path::new(&a).join("sotto").is_dir()).unwrap_or(false);
    serde_json::json!({
        "version": VERSION,
        "size": format!("{} MB", (INSTALLER.len() + 512 * 1024) / (1024 * 1024)),
        "update": existing.is_some(),
        "dir": dir,
        "autostart": existing.is_none() || reg_value(RUN_KEY, "Sotto").is_some(),
        "desktop": desktop_link().is_some_and(|p| p.exists()),
        "firstRun": !models,
    })
}

/// The whole install, step by step; `step(n)` / `fail(msg)` drive the page.
fn install(dir: String, autostart: bool, desktop: bool, send: impl Fn(String)) {
    let step = |n: u8| send(format!("setup.step({n})"));
    let fail = |m: &str| send(format!("setup.fail({})", serde_json::Value::from(m)));

    step(0); // close the running app, or its exe is locked
    for exe in ["sotto.exe", "llama-server.exe"] {
        let _ = quiet("taskkill").args(["/F", "/IM", exe, "/T"]).status();
    }

    step(1);
    let tmp = std::env::temp_dir().join(format!("sotto-setup-{}.exe", std::process::id()));
    if std::fs::write(&tmp, INSTALLER).is_err() {
        return fail("Couldn't unpack the installer. Check there's free space on your system drive.");
    }
    // NSIS wants /D last and unquoted, even with spaces in it.
    let ok = quiet(tmp.to_str().unwrap_or_default())
        .raw_arg("/S")
        .raw_arg(format!("/D={dir}"))
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let _ = std::fs::remove_file(&tmp);
    let exe = Path::new(&dir).join("sotto.exe");
    if !ok || !exe.exists() {
        return fail("Sotto couldn't be installed in that folder. Try another one.");
    }

    step(2); // the silent installer makes the desktop shortcut; keep it or not
    if let Some(link) = desktop_link() {
        if !desktop {
            let _ = std::fs::remove_file(&link);
        } else if !link.exists() {
            let ps = format!(
                "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}');$s.TargetPath='{}';$s.Save()",
                link.display().to_string().replace('\'', "''"),
                exe.display().to_string().replace('\'', "''"),
            );
            let _ = quiet("powershell").args(["-NoProfile", "-Command", &ps]).status();
        }
    }

    step(3); // the same Run value startup.rs reads and writes
    if autostart {
        let _ = quiet("reg").args(["add", RUN_KEY, "/v", "Sotto", "/t", "REG_SZ", "/d", &exe.to_string_lossy(), "/f"]).status();
    } else {
        let _ = quiet("reg").args(["delete", RUN_KEY, "/v", "Sotto", "/f"]).status();
    }
    step(4);
}

/// The system's folder picker, starting at `start`.
fn pick_folder(hwnd: isize, start: &str) -> Option<String> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{FileOpenDialog, IFileOpenDialog, IShellItem, SHCreateItemFromParsingName, FOS_PICKFOLDERS, SIGDN_FILESYSPATH};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let dlg: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        dlg.SetOptions(dlg.GetOptions().ok()? | FOS_PICKFOLDERS).ok()?;
        if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(start), None) {
            let _ = dlg.SetFolder(&item);
        }
        dlg.Show(Some(HWND(hwnd as _))).ok()?;
        let path = dlg.GetResult().ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = path.to_string().ok();
        windows::Win32::System::Com::CoTaskMemFree(Some(path.0 as _));
        s
    }
}

/// Windows 11 doesn't round an undecorated window's corners on its own.
fn round_corners(hwnd: isize) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
    unsafe {
        let pref = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(HWND(hwnd as _), DWMWA_WINDOW_CORNER_PREFERENCE, &pref as *const _ as _, 4);
    }
}

fn main() -> wry::Result<()> {
    let event_loop = EventLoopBuilder::<Ui>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let window = WindowBuilder::new()
        .with_title("Sotto setup")
        .with_inner_size(LogicalSize::new(520.0, 400.0))
        .with_resizable(false)
        .with_decorations(false)
        .with_undecorated_shadow(true)
        .with_visible(false)
        .build(&event_loop)
        .expect("window");
    let hwnd = window.hwnd() as isize;
    round_corners(hwnd);

    let init = format!("window.SETUP = {};", initial_state());
    let ipc_proxy = proxy.clone();
    let webview = WebViewBuilder::new()
        .with_initialization_script(init)
        .with_custom_protocol("sotto".into(), |_, req| match asset(req.uri().path()) {
            Some((body, mime)) => Response::builder().header("Content-Type", mime).body(Cow::Borrowed(body)).unwrap(),
            None => Response::builder().status(404).body(Cow::Borrowed(&[][..])).unwrap(),
        })
        .with_url("sotto://localhost/setup.html")
        .with_ipc_handler(move |req| {
            let msg: serde_json::Value = serde_json::from_str(req.body()).unwrap_or_default();
            let p = ipc_proxy.clone();
            match msg["cmd"].as_str().unwrap_or("") {
                "drag" => { let _ = p.send_event(Ui::Drag); }
                "minimize" => { let _ = p.send_event(Ui::Minimize); }
                "close" => { let _ = p.send_event(Ui::Close); }
                "pick" => {
                    let start = msg["dir"].as_str().unwrap_or("").to_string();
                    if let Some(dir) = pick_folder(hwnd, &start) {
                        let _ = p.send_event(Ui::Js(format!("setup.picked({})", serde_json::Value::from(dir))));
                    }
                }
                "install" => {
                    let dir = msg["dir"].as_str().unwrap_or("").trim().to_string();
                    let (auto, desk) = (msg["autostart"].as_bool().unwrap_or(true), msg["desktop"].as_bool().unwrap_or(false));
                    std::thread::spawn(move || install(dir, auto, desk, |js| { let _ = p.send_event(Ui::Js(js)); }));
                }
                "open" => {
                    let exe = Path::new(msg["dir"].as_str().unwrap_or("")).join("sotto.exe");
                    let _ = Command::new(exe).spawn();
                    let _ = p.send_event(Ui::Close);
                }
                _ => {}
            }
        })
        .build(&window)?;
    window.set_visible(true);

    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(Ui::Js(js)) => { let _ = webview.evaluate_script(&js); }
            Event::UserEvent(Ui::Drag) => { let _ = window.drag_window(); }
            Event::UserEvent(Ui::Minimize) => window.set_minimized(true),
            Event::UserEvent(Ui::Close) | Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                *flow = ControlFlow::Exit
            }
            _ => {}
        }
    });
}
