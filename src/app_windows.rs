//! Lazy windows (#13). With `lazy_windows` on, the main window (`settings`)
//! and the tray menu (`menu`) exist only while they are open: built from their
//! tauri.conf.json entries when opened, destroyed when closed. A window that
//! doesn't exist has no WebView2 renderer process, which is the point: the
//! tree cost ~590 MB with the app hidden in the tray. The overlay pill is
//! always there. With the flag off nothing here builds or destroys a window.
//!
//! A window built on demand has missed every event emitted before it existed,
//! so its page pulls what it shows (`get_settings`, `assets_status`,
//! `scratchpad_state`) and is handed its theme and first page up front
//! (`opened_script`).

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Mutex;
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow, WebviewWindowBuilder, WindowEvent};

/// What opening a window has to do.
#[derive(Debug, PartialEq)]
pub enum Open {
    /// It exists: bring it forward.
    Focus,
    /// Lazy, and not built yet or destroyed since: build it, then show it.
    Build,
    /// Not lazy, and gone: nothing, as before #13. A close no longer gets the
    /// main window here (`close_hides`, #124).
    Nothing,
}

pub fn open_step(exists: bool, lazy: bool) -> Open {
    match (exists, lazy) {
        (true, _) => Open::Focus,
        (false, true) => Open::Build,
        (false, false) => Open::Nothing,
    }
}

/// Does closing a window only hide it? Without lazy windows, yes: nothing
/// would build it again. The ✕ button (`dismiss`) and a real close, Alt+F4 or
/// the taskbar's (`track`, #124), both ask here.
pub fn close_hides(lazy: bool) -> bool {
    !lazy
}

/// Runs before the page's own scripts. A window shown as it loads must be
/// themed before `get_settings` answers, and the `navigate` event fired at a
/// window that is still loading reaches nobody.
fn opened_script(theme: &str, page: Option<&str>) -> String {
    format!("window.__SOTTO_OPENED = {};", serde_json::json!({ "theme": theme, "page": page }))
}

/// `lazy_windows`, set once in setup. An atomic, not the config lock: `lazy`
/// is asked from the main thread (tray clicks, the close handler), and a
/// command holding the config lock while it waits on the main thread for a
/// window call would deadlock against it. The flag only changes at launch.
static LAZY: AtomicBool = AtomicBool::new(false);

pub fn set_lazy(on: bool) {
    LAZY.store(on, Ordering::Relaxed);
}

pub fn lazy(_app: &AppHandle) -> bool {
    LAZY.load(Ordering::Relaxed)
}

/// One build at a time: a double click on the tray icon asks twice.
static BUILDING: Mutex<()> = Mutex::new(());
/// Where the main window was when it last closed, so a rebuilt one comes back
/// there and not centred at its default size. ponytail: this session only,
/// like the hidden window it replaces, and a monitor unplugged in between
/// isn't checked for.
static PLACE: Mutex<Option<(PhysicalPosition<i32>, PhysicalSize<u32>, bool)>> = Mutex::new(None);
/// The main window's handle, kept after the window is destroyed: a take
/// spoken into the Scratchpad can still be in flight (`scratchpad::catches`).
static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);

pub fn main_hwnd() -> isize {
    MAIN_HWND.load(Ordering::Relaxed)
}

/// Launch, with lazy windows on. Tauri has just built both windows from
/// tauri.conf.json, as it always has: the ones launch isn't showing go again.
/// ponytail: built to be destroyed, which costs launch what it already cost.
/// `"create": false` in tauri.conf.json saves that once lazy is the only mode.
pub fn at_launch(app: &AppHandle) {
    for label in ["settings", "menu"] {
        let Some(w) = app.get_webview_window(label) else { continue };
        if !w.is_visible().unwrap_or(false) {
            let _ = w.destroy();
        } else if label == "settings" {
            track(&w);
        }
    }
}

/// Builds `label` from its tauri.conf.json entry, hidden, and returns it (or
/// the one that's already there). `loaded` runs once its page has loaded.
///
/// This waits for WebView2 on the main thread, and deadlocks if called there
/// from a command or an event handler (wry#583): give it a thread of its own.
pub fn build(
    app: &AppHandle,
    label: &str,
    page: Option<&str>,
    loaded: impl Fn(&WebviewWindow) + Send + Sync + 'static,
) -> Option<WebviewWindow> {
    let _one = BUILDING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(w) = app.get_webview_window(label) {
        return Some(w);
    }
    // Copied out first: the main thread this is about to wait on may itself
    // be waiting for the config lock.
    let (theme, zoom, lecture_mode) = {
        let state = app.state::<crate::AppState>();
        let cfg = state.cfg.lock().unwrap();
        (cfg.theme.clone(), cfg.zoom, cfg.lecture_mode)
    };
    let conf = app.config().app.windows.iter().find(|w| w.label == label)?;
    let built = WebviewWindowBuilder::from_config(app, conf).and_then(|b| {
        b.initialization_script(opened_script(&theme, page))
            .on_page_load(move |w, payload| {
                if matches!(payload.event(), PageLoadEvent::Finished) {
                    loaded(&w);
                }
            })
            .build()
    });
    let w = match built {
        Ok(w) => w,
        Err(err) => {
            tracing::warn!(?err, label, "couldn't build the window");
            return None;
        }
    };
    // From here on, what `main`'s setup does to the ones Tauri builds at launch.
    if label == "menu" {
        crate::harden_utility_window(&w, false);
        // The lecture item (#23) is one more 33px row than tauri.conf.json's
        // 380 has room for.
        if lecture_mode {
            let _ = w.set_size(tauri::LogicalSize::new(230.0, 413.0));
        }
    } else {
        if (zoom - 1.0).abs() > f64::EPSILON {
            let _ = w.set_zoom(zoom.clamp(crate::ZOOM_MIN, crate::ZOOM_MAX));
        }
        let place = *PLACE.lock().unwrap();
        match place {
            Some((_, _, true)) => {
                let _ = w.maximize();
            }
            Some((at, size, false)) => {
                let _ = w.set_size(size);
                let _ = w.set_position(at);
            }
            None => {}
        }
        track(&w);
    }
    Some(w)
}

/// Notes the main window's handle, and where it is when it closes. Without
/// lazy windows the close itself is refused and the window hidden (#124): a
/// real close would destroy the only one there will be.
pub fn track(w: &WebviewWindow) {
    MAIN_HWND.store(w.hwnd().map_or(0, |h| h.0 as isize), Ordering::Relaxed);
    let closing = w.clone();
    w.on_window_event(move |e| {
        let WindowEvent::CloseRequested { api, .. } = e else { return };
        if !closing.is_minimized().unwrap_or(true) {
            let maximized = closing.is_maximized().unwrap_or(false);
            if let (Ok(at), Ok(size)) = (closing.outer_position(), closing.inner_size()) {
                *PLACE.lock().unwrap() = Some((at, size, maximized));
            }
        }
        if close_hides(lazy(closing.app_handle())) {
            api.prevent_close();
            let _ = closing.hide();
        }
    });
}

/// Opens the main window, on `page` if one is given: the tray icon, a tray
/// menu item, a second launch (#61).
pub fn open_settings(app: &AppHandle, page: Option<String>) {
    let existing = app.get_webview_window("settings");
    match (open_step(existing.is_some(), lazy(app)), existing) {
        (Open::Focus, Some(w)) => {
            // First: show and set_focus don't restore a minimised window (#125).
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
            // "settings" too: it's a modal now, and navigate() is what opens it.
            if let Some(page) = page {
                let _ = w.emit("navigate", page);
            }
        }
        (Open::Build, _) => {
            let app = app.clone();
            std::thread::spawn(move || {
                // Shown once loaded, not before: an empty white window would flash first.
                build(&app, "settings", page.as_deref(), |w| {
                    let _ = w.show();
                    let _ = w.set_focus();
                });
            });
        }
        _ => {}
    }
}

/// Opens the tray menu with its corner at the click.
pub fn open_menu(app: &AppHandle, at: PhysicalPosition<f64>) {
    let existing = app.get_webview_window("menu");
    match (open_step(existing.is_some(), lazy(app)), existing) {
        (Open::Focus, Some(w)) => show_menu(&w, at),
        (Open::Build, _) => {
            let app = app.clone();
            std::thread::spawn(move || {
                // Shown once loaded: its page is what closes it on a click
                // away, so it must be listening by the time it has focus.
                build(&app, "menu", None, move |w| show_menu(w, at));
            });
        }
        _ => {}
    }
}

fn show_menu(w: &WebviewWindow, at: PhysicalPosition<f64>) {
    // Use the window's real size (already physical px) so this never drifts
    // from tauri.conf.json / menu.html — hardcoded 230x260 here is what
    // chipped the menu.
    let (mw, mh) = w.outer_size().map(|s| (s.width as f64, s.height as f64)).unwrap_or((230.0, 380.0));
    let x = (at.x - mw + 10.0) as i32;
    let y = (at.y - mh - 5.0) as i32;
    let _ = w.set_position(PhysicalPosition::new(x, y));
    crate::harden_utility_window(w, false);
    let _ = w.show();
    crate::harden_utility_window(w, false);
    let _ = w.set_focus();
}

/// A window put away: hidden, or with lazy windows closed for good, its
/// renderer with it. Safe on any thread: a close is queued for the event loop.
pub fn dismiss(w: &WebviewWindow) {
    let _ = if close_hides(lazy(w.app_handle())) { w.hide() } else { w.close() };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_that_exists_is_focused_and_a_missing_one_is_built_only_when_lazy() {
        assert_eq!(open_step(true, false), Open::Focus);
        assert_eq!(open_step(true, true), Open::Focus);
        assert_eq!(open_step(false, true), Open::Build);
        // Flag off: opening never builds anything, exactly as before #13.
        assert_eq!(open_step(false, false), Open::Nothing);
    }

    /// #124: an Alt+F4 with the flag off destroyed the window, and the tray
    /// then had nothing to open until a restart.
    #[test]
    fn a_closed_window_can_always_be_opened_again() {
        for lazy in [false, true] {
            // Hidden, it still exists; destroyed, it doesn't.
            let exists = close_hides(lazy);
            assert_ne!(open_step(exists, lazy), Open::Nothing, "lazy={lazy}");
        }
        assert!(close_hides(false) && !close_hides(true));
    }

    #[test]
    fn the_opened_script_carries_the_theme_and_page_as_json() {
        let opened = |theme, page| -> serde_json::Value {
            let script = opened_script(theme, page);
            let json = script.strip_prefix("window.__SOTTO_OPENED = ").and_then(|s| s.strip_suffix(';')).unwrap();
            serde_json::from_str(json).unwrap()
        };
        assert_eq!(opened("system", None), serde_json::json!({ "theme": "system", "page": null }));
        // Whatever the page is called, it stays inside its string.
        assert_eq!(opened("dark", Some("a\";alert(1)//"))["page"], "a\";alert(1)//");
    }

    /// `build` finds its windows by these labels, and quietly builds nothing
    /// for one that tauri.conf.json no longer has.
    #[test]
    fn tauri_conf_still_has_the_windows_build_is_asked_for() {
        let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let labels: Vec<&str> =
            conf["app"]["windows"].as_array().unwrap().iter().map(|w| w["label"].as_str().unwrap()).collect();
        assert!(labels.contains(&"settings") && labels.contains(&"menu"), "{labels:?}");
    }
}
