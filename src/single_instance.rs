use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, OpenEventW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE, INFINITE,
};
use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY};

/// Named auto-reset event a second launch sets to bring the running
/// instance's window forward (#61).
const WAKE_EVENT: &str = "Sotto_Wake_9F3D2A1C";

/// Holds a named OS mutex for the lifetime of the process. Dropping it releases
/// the mutex, letting a future instance acquire it.
pub struct SingleInstanceGuard {
    handle: HANDLE,
}

impl SingleInstanceGuard {
    /// Returns `Ok(Some(guard))` if this is the only running instance, or
    /// `Ok(None)` if another instance already holds the lock.
    pub fn acquire() -> anyhow::Result<Option<Self>> {
        let name: Vec<u16> = "Sotto_SingleInstance_9F3D2A1C\0"
            .encode_utf16()
            .collect();

        // SAFETY: `name` is a valid null-terminated UTF-16 buffer that outlives
        // this call, and the returned handle is checked before use.
        let handle = unsafe { CreateMutexW(None, true, PCWSTR(name.as_ptr()))? };

        let already_running = unsafe { windows::Win32::Foundation::GetLastError() } == ERROR_ALREADY_EXISTS;

        if already_running {
            unsafe {
                let _ = CloseHandle(handle);
            }
            Ok(None)
        } else {
            Ok(Some(Self { handle }))
        }
    }
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

/// Called by a second launch just before it exits. Also hands over the right
/// to take the foreground, which Windows gives the process the user just
/// started, not the one already running in the tray.
pub fn wake_running() {
    // SAFETY: plain Win32 call with no pointers.
    let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    signal(WAKE_EVENT);
}

/// Runs `on_wake` on a background thread each time a second launch calls
/// `wake_running`.
/// ponytail: the event exists from Tauri setup on, so a launch during the
/// first instance's own startup second is dropped. Create it in `acquire`
/// if that ever matters.
pub fn on_wake(on_wake: impl Fn() + Send + 'static) {
    listen(WAKE_EVENT, on_wake);
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

fn signal(name: &str) {
    let name = wide(name);
    // SAFETY: `name` is NUL-terminated and outlives the call; the handle is
    // closed right after use. No listener (no event) is a quiet no-op.
    unsafe {
        if let Ok(ev) = OpenEventW(EVENT_MODIFY_STATE, false, PCWSTR(name.as_ptr())) {
            let _ = SetEvent(ev);
            let _ = CloseHandle(ev);
        }
    }
}

fn listen(name: &str, on_wake: impl Fn() + Send + 'static) {
    let name = wide(name);
    // SAFETY: as in `signal`. The handle stays open for the life of the
    // waiting thread, which is the life of the process.
    let Ok(ev) = (unsafe { CreateEventW(None, false, false, PCWSTR(name.as_ptr())) }) else {
        return;
    };
    let raw = ev.0 as usize; // HANDLE isn't Send
    std::thread::spawn(move || {
        let ev = HANDLE(raw as *mut _);
        while unsafe { WaitForSingleObject(ev, INFINITE) } == WAIT_OBJECT_0 {
            on_wake();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signal_wakes_the_listener_and_an_unheard_one_is_harmless() {
        // A per-process name, so this never touches a running Sotto.
        let name = format!("Sotto_WakeTest_{}", std::process::id());
        let (tx, rx) = std::sync::mpsc::channel();
        listen(&name, move || {
            let _ = tx.send(());
        });
        signal(&name);
        assert!(rx.recv_timeout(std::time::Duration::from_secs(5)).is_ok());
        signal(&format!("{name}_nobody"));
    }
}
