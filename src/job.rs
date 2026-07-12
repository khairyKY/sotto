//! Windows Job Object that kills our child processes when Sotto exits.
//!
//! On Windows a child process is *not* torn down when its parent dies. Without
//! this, the llama.cpp sidecar would orphan on a crash (or any exit path that
//! skips our cleanup) and keep ~1 GB of VRAM pinned. Assigning each spawned
//! child to a job with `KILL_ON_JOB_CLOSE` guarantees the OS reaps it when our
//! process handle to the job closes — i.e. when Sotto goes away, for any reason.

use std::os::windows::io::AsRawHandle;
use std::process::Child;
use std::sync::OnceLock;
use windows::core::PCWSTR;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

// Stored as isize because a raw pointer isn't `Send`/`Sync`. The handle is
// intentionally never closed — it lives for the whole process, and closing it
// early would kill the very children we want to keep alive.
static JOB: OnceLock<isize> = OnceLock::new();

fn job_handle() -> HANDLE {
    let raw = *JOB.get_or_init(|| unsafe {
        let job = CreateJobObjectW(None, PCWSTR::null()).expect("CreateJobObjectW failed");
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let _ = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        job.0 as isize
    });
    HANDLE(raw as *mut core::ffi::c_void)
}

/// Bind `child` to the kill-on-close job so it dies with Sotto.
pub fn kill_with_parent(child: &Child) {
    unsafe {
        let process = HANDLE(child.as_raw_handle() as *mut core::ffi::c_void);
        if let Err(err) = AssignProcessToJobObject(job_handle(), process) {
            tracing::warn!(error = %err, "could not bind sidecar to kill-on-close job");
        }
    }
}
