//! "Launch Sotto at login" via the per-user Run registry key. Implemented by
//! shelling out to the always-present `reg.exe` — no extra `windows` crate
//! features and no unsafe FFI for a toggle flipped at most a few times.

use std::os::windows::process::CommandExt;
use std::process::Command;

const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "Sotto";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn reg() -> Command {
    let mut c = Command::new("reg");
    c.creation_flags(CREATE_NO_WINDOW); // no console flash
    c
}

/// True if the Run entry exists (Sotto is set to launch at login).
pub fn is_enabled() -> bool {
    reg()
        .args(["query", RUN_KEY, "/v", VALUE])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Add or remove the Run entry pointing at the current executable.
pub fn set_enabled(enable: bool) -> anyhow::Result<()> {
    if enable {
        let exe = std::env::current_exe()?;
        let exe = exe.to_string_lossy().to_string();
        let ok = reg()
            .args(["add", RUN_KEY, "/v", VALUE, "/t", "REG_SZ", "/d", &exe, "/f"])
            .status()?
            .success();
        anyhow::ensure!(ok, "reg add failed");
    } else {
        // Deleting a missing value returns nonzero — that's fine, it's absent.
        let _ = reg().args(["delete", RUN_KEY, "/v", VALUE, "/f"]).status();
    }
    Ok(())
}
