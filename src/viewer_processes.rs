//! Tracks standalone viewer subprocesses spawned by the main app.

use std::ffi::OsStr;
use std::os::windows::io::AsRawHandle;
use std::process::Child;
use std::sync::{Mutex, OnceLock};

use windows::core::PCWSTR;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

use crate::infrastructure::windows::OwnedHandle;

struct ViewerJob {
    handle: OwnedHandle,
}

impl ViewerJob {
    fn create() -> Option<Self> {
        // Null security attributes make the returned handle non-inheritable.
        let raw_handle = match unsafe { CreateJobObjectW(None, PCWSTR::null()) } {
            Ok(handle) => handle,
            Err(error) => {
                log::warn!(
                    "[VIEWER-PROCESS] Failed to create viewer Job Object; viewers remain open: {}",
                    error
                );
                return None;
            }
        };
        let Some(handle) = OwnedHandle::new(raw_handle) else {
            log::warn!(
                "[VIEWER-PROCESS] CreateJobObjectW returned an invalid handle; viewers remain open"
            );
            return None;
        };

        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let result = unsafe {
            SetInformationJobObject(
                handle.as_raw(),
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&limits).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        };
        if let Err(error) = result {
            log::warn!(
                "[VIEWER-PROCESS] Failed to configure viewer Job Object; viewers remain open: {}",
                error
            );
            return None;
        }

        Some(Self { handle })
    }
}

fn is_viewer_subprocess_flag(flag: Option<&OsStr>) -> bool {
    let Some(flag) = flag else {
        return false;
    };
    let flag = flag.to_string_lossy();
    [
        "--image-viewer",
        "--pdf-viewer",
        "--text-viewer",
        "--video-player",
    ]
    .iter()
    .any(|viewer_flag| flag.eq_ignore_ascii_case(viewer_flag))
}

fn viewer_job() -> Option<&'static ViewerJob> {
    static VIEWER_JOB: OnceLock<Option<ViewerJob>> = OnceLock::new();

    VIEWER_JOB
        .get_or_init(|| {
            if is_viewer_subprocess_flag(std::env::args_os().nth(1).as_deref()) {
                None
            } else {
                ViewerJob::create()
            }
        })
        .as_ref()
}

fn child_processes() -> &'static Mutex<Vec<Child>> {
    static CHILD_PROCESSES: OnceLock<Mutex<Vec<Child>>> = OnceLock::new();
    CHILD_PROCESSES.get_or_init(|| Mutex::new(Vec::new()))
}

pub fn register(child: Child) {
    assign_to_job(&child);

    let Ok(mut children) = child_processes().lock() else {
        log::warn!("[VIEWER-PROCESS] Failed to track spawned viewer process");
        return;
    };

    children.push(child);
}

/// Best-effort assignment, not a security boundary. Explicit kill handles remain the fallback.
pub fn assign_to_job(child: &Child) {
    let Some(job) = viewer_job() else {
        return;
    };

    let process_handle = HANDLE(child.as_raw_handle());
    if let Err(error) = unsafe { AssignProcessToJobObject(job.handle.as_raw(), process_handle) } {
        log::warn!(
            "[VIEWER-PROCESS] Failed to assign viewer pid={} to Job Object; viewer remains open (nested job restrictions may apply): {}",
            child.id(),
            error
        );
    }
}

pub fn reap_exited() {
    let Ok(mut children) = child_processes().lock() else {
        return;
    };

    children.retain_mut(|child| match child.try_wait() {
        Ok(Some(_)) => false,
        Ok(None) => true,
        Err(error) => {
            log::warn!(
                "[VIEWER-PROCESS] Failed to query viewer subprocess pid={}: {}",
                child.id(),
                error
            );
            false
        }
    });
}

pub fn terminate_all() {
    let Ok(mut children) = child_processes().lock() else {
        log::warn!("[VIEWER-PROCESS] Failed to lock viewer process registry during shutdown");
        return;
    };

    for mut child in children.drain(..) {
        match child.try_wait() {
            Ok(Some(_)) => {}
            Ok(None) => {
                log::debug!(
                    "[VIEWER-PROCESS] Terminating viewer subprocess pid={}",
                    child.id()
                );
                if let Err(error) = child.kill() {
                    log::warn!(
                        "[VIEWER-PROCESS] Failed to terminate viewer subprocess pid={}: {}",
                        child.id(),
                        error
                    );
                }
            }
            Err(error) => {
                log::warn!(
                    "[VIEWER-PROCESS] Failed to query viewer subprocess pid={}: {}",
                    child.id(),
                    error
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_viewer_subprocess_flag, ViewerJob};
    use std::ffi::OsStr;
    use windows::Win32::Foundation::{GetHandleInformation, HANDLE_FLAG_INHERIT};
    use windows::Win32::System::JobObjects::{
        JobObjectExtendedLimitInformation, QueryInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    #[test]
    fn recognizes_all_viewer_subprocess_modes() {
        for flag in [
            "--image-viewer",
            "--pdf-viewer",
            "--text-viewer",
            "--video-player",
            "--VIDEO-PLAYER",
        ] {
            assert!(is_viewer_subprocess_flag(Some(OsStr::new(flag))));
        }
        assert!(!is_viewer_subprocess_flag(Some(OsStr::new(
            "--set-volume-label"
        ))));
        assert!(!is_viewer_subprocess_flag(None));
    }

    #[test]
    fn viewer_job_is_non_inheritable_and_has_kill_on_close() {
        let job = ViewerJob::create().expect("test Job Object should be available");

        let mut handle_flags = 0;
        unsafe { GetHandleInformation(job.handle.as_raw(), &mut handle_flags) }
            .expect("querying Job Object handle flags should succeed");
        assert_eq!(handle_flags & HANDLE_FLAG_INHERIT.0, 0);

        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        unsafe {
            QueryInformationJobObject(
                Some(job.handle.as_raw()),
                JobObjectExtendedLimitInformation,
                std::ptr::from_mut(&mut limits).cast(),
                std::mem::size_of_val(&limits) as u32,
                None,
            )
        }
        .expect("querying Job Object limits should succeed");
        assert!(limits
            .BasicLimitInformation
            .LimitFlags
            .contains(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE));
    }
}
