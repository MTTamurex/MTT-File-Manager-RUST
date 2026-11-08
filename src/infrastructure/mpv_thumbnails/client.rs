use super::{
    process::{Control, Process},
    protocol::{Frame, Request, Response},
    Error, HELPER_FLAG, REQUEST_TIMEOUT,
};
use crossbeam_channel::RecvTimeoutError;
use std::{
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};
use windows::Win32::System::Threading::{BELOW_NORMAL_PRIORITY_CLASS, CREATE_NO_WINDOW};

/// Serial supervisor. Each instance owns an independent Job Object/process.
/// Public so the integration tests can select the real application executable.
pub struct Extractor {
    executable: PathBuf,
    pub(super) control: Arc<Control>,
    process: Option<Process>,
    next_id: u64,
}

impl Extractor {
    pub fn new(executable: PathBuf) -> Self {
        Self {
            executable,
            control: Arc::default(),
            process: None,
            next_id: 0,
        }
    }

    fn command(&self) -> Result<Command, Error> {
        if !self.executable.is_absolute() {
            return Err(Error::Unavailable);
        }
        let mut command = Command::new(&self.executable);
        command
            .arg(HELPER_FLAG)
            .creation_flags(CREATE_NO_WINDOW.0 | BELOW_NORMAL_PRIORITY_CLASS.0);
        Ok(command)
    }

    pub fn extract(
        &mut self,
        path: &Path,
        max_side: u32,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Frame, Error> {
        self.extract_until(path, max_side, cancelled, Instant::now() + REQUEST_TIMEOUT)
    }

    pub(super) fn extract_until(
        &mut self,
        path: &Path,
        max_side: u32,
        cancelled: &dyn Fn() -> bool,
        deadline: Instant,
    ) -> Result<Frame, Error> {
        let result = self.perform(path, max_side, cancelled, deadline);
        if result.is_err() {
            // Never retry the same request automatically; the next request gets
            // a fresh process. Existing worker failure/backoff policy still applies.
            drop(self.process.take());
        }
        result
    }

    fn perform(
        &mut self,
        path: &Path,
        max_side: u32,
        cancelled: &dyn Fn() -> bool,
        deadline: Instant,
    ) -> Result<Frame, Error> {
        self.check(cancelled, deadline)?;
        let path = path
            .to_str()
            .filter(|_| path.is_absolute())
            .ok_or(Error::Decode)?;
        if self.process.is_none() {
            self.process = Some(
                Process::spawn(self.command()?, self.control.clone())
                    .map_err(|_| Error::Unavailable)?,
            );
        }
        self.check(cancelled, deadline)?;
        self.next_id = self.next_id.checked_add(1).ok_or(Error::Unavailable)?;
        let response = self
            .process
            .as_ref()
            .unwrap()
            .request(Request {
                id: self.next_id,
                max_side,
                budget_ms: deadline
                    .saturating_duration_since(Instant::now())
                    .as_millis()
                    .clamp(1, 15_000) as u32,
                path: path.into(),
            })
            .map_err(|_| Error::Transport)?;
        loop {
            self.check(cancelled, deadline)?;
            match response.recv_timeout(Duration::from_millis(20)) {
                Ok(Ok(result)) => {
                    self.check(cancelled, deadline)?;
                    return Self::decode_response(result);
                }
                Ok(Err(_)) | Err(RecvTimeoutError::Disconnected) => {
                    self.check(cancelled, deadline)?;
                    return Err(Error::Transport);
                }
                Err(RecvTimeoutError::Timeout) => {
                    if self
                        .process
                        .as_mut()
                        .unwrap()
                        .exited()
                        .map_err(|_| Error::Transport)?
                    {
                        return Err(Error::Transport);
                    }
                }
            }
        }
    }

    fn decode_response(response: Response) -> Result<Frame, Error> {
        response.map_err(Error::Helper)
    }

    fn check(&self, cancelled: &dyn Fn() -> bool, deadline: Instant) -> Result<(), Error> {
        if self.control.stopped() || cancelled() {
            return Err(Error::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(Error::Timeout);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn install_test_process(&mut self, command: Command) {
        drop(self.process.take());
        self.process = Some(Process::spawn(command, self.control.clone()).unwrap());
    }

    #[cfg(test)]
    pub(super) fn process_id(&self) -> Option<u32> {
        self.process.as_ref().map(Process::id)
    }
}
