use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;

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
