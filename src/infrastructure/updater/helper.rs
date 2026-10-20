use std::path::Path;
use std::process::Child;
use std::time::{Duration, Instant};

const READY_TIMEOUT: Duration = Duration::from_secs(15);

/// Owns only the unelevated helper waiting for authorization, never the installer.
pub(crate) struct HelperProcess {
    child: Option<Child>,
    started: Instant,
}

impl HelperProcess {
    pub(crate) fn new(child: Child) -> Self {
        Self {
            child: Some(child),
            started: Instant::now(),
        }
    }

    pub(crate) fn ready(&mut self, directory: &Path) -> Result<bool, String> {
        match self.child.as_mut().unwrap().try_wait() {
            Ok(None) => {}
            Ok(Some(_)) => return Err("The update helper stopped before installation".into()),
            Err(_) => return Err("Could not monitor the update helper".into()),
        }
        if directory.join("ready").is_file() {
            return Ok(true);
        }
        if self.started.elapsed() >= READY_TIMEOUT {
            return Err("The update helper did not become ready in time".into());
        }
        Ok(false)
    }

    /// After authorization the helper must outlive MTT; dropping std::Child does
    /// not terminate its process. Before this point Drop cancels the handoff.
    pub(crate) fn detach(mut self) -> Child {
        self.child.take().unwrap()
    }
}

impl Drop for HelperProcess {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            if !matches!(child.try_wait(), Ok(Some(_))) {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn waiting_helper() -> HelperProcess {
        HelperProcess::new(
            std::process::Command::new("powershell.exe")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "Start-Sleep -Seconds 30",
                ])
                .spawn()
                .unwrap(),
        )
    }

    #[test]
    fn cancelled_helper_is_stopped_before_temporary_files_are_removed() {
        use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
        use windows::Win32::System::Threading::{
            OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
        };
        let helper = waiting_helper();
        let process = unsafe {
            OpenProcess(
                PROCESS_SYNCHRONIZE,
                false,
                helper.child.as_ref().unwrap().id(),
            )
        }
        .unwrap();
        drop(helper);
        let waited = unsafe { WaitForSingleObject(process, 0) };
        unsafe {
            let _ = CloseHandle(process);
        }
        assert_eq!(waited, WAIT_OBJECT_0);
    }

    #[test]
    fn readiness_is_bounded_but_an_acknowledged_helper_remains_ready() {
        let directory = tempfile::tempdir().unwrap();
        let mut helper = waiting_helper();
        assert!(!helper.ready(directory.path()).unwrap());
        helper.started = Instant::now() - READY_TIMEOUT;
        assert!(helper.ready(directory.path()).is_err());
        std::fs::write(directory.path().join("ready"), b"ready").unwrap();
        assert!(helper.ready(directory.path()).unwrap());
    }

    #[test]
    fn authorized_helper_is_not_terminated_by_the_guard() {
        let helper = waiting_helper();
        let mut child = helper.detach();
        assert!(child.try_wait().unwrap().is_none());
        child.kill().unwrap();
        child.wait().unwrap();
    }
}
