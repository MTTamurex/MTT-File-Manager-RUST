use super::protocol::{self, Request, Response};
use crate::infrastructure::windows::OwnedHandle;
use crossbeam_channel::{Receiver, Sender};
use parking_lot::Mutex;
use std::{
    io,
    os::windows::io::AsRawHandle,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::JoinHandle,
};
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::HANDLE,
        System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        },
    },
};

#[derive(Default)]
pub(super) struct Control {
    stopped: AtomicBool,
    job: Mutex<Option<OwnedHandle>>,
}

impl Control {
    pub fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }

    fn assign(&self, child: &Child) -> io::Result<()> {
        let mut job = self.job.lock();
        if self.stopped() {
            return Err(io::Error::other("Thumbnail service is stopped"));
        }
        if job.is_none() {
            let raw =
                unsafe { CreateJobObjectW(None, PCWSTR::null()) }.map_err(io::Error::other)?;
            let handle = OwnedHandle::new(raw)
                .ok_or_else(|| io::Error::other("Invalid thumbnail job handle"))?;
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            unsafe {
                SetInformationJobObject(
                    handle.as_raw(),
                    JobObjectExtendedLimitInformation,
                    std::ptr::from_ref(&limits).cast(),
                    std::mem::size_of_val(&limits) as u32,
                )
            }
            .map_err(io::Error::other)?;
            *job = Some(handle);
        }
        let handle = job.as_ref().unwrap();
        // Fail closed: an uncontained helper must not be allowed to decode.
        unsafe { AssignProcessToJobObject(handle.as_raw(), HANDLE(child.as_raw_handle())) }
            .map_err(io::Error::other)
    }

    pub fn terminate(&self) {
        if let Some(job) = self.job.lock().as_ref() {
            let _ = unsafe { TerminateJobObject(job.as_raw(), 1) };
        }
    }

    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
        // No client/decoder lock: UI shutdown can interrupt a stuck request.
        drop(self.job.lock().take());
    }
}

pub(super) struct Work {
    pub request: Request,
    pub reply: Sender<io::Result<Response>>,
}

pub(super) struct Process {
    child: Child,
    control: Arc<Control>,
    sender: Option<Sender<Work>>,
    transport: Option<JoinHandle<()>>,
}

impl Process {
    pub fn spawn(mut command: Command, control: Arc<Control>) -> io::Result<Self> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let child = command.spawn()?;
        let mut process = Self {
            child,
            control,
            sender: None,
            transport: None,
        };
        process.control.assign(&process.child)?;
        // The helper reads its first request before creating its libmpv session.
        let mut input = process
            .child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("Missing stdin pipe"))?;
        let mut output = process
            .child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("Missing stdout pipe"))?;
        let (sender, receiver) = crossbeam_channel::bounded::<Work>(1);
        process.transport = Some(
            std::thread::Builder::new()
                .name("mpv-thumb-ipc".into())
                .spawn(move || {
                    while let Ok(work) = receiver.recv() {
                        // All blocking pipe I/O runs here, not on the supervising thread.
                        let result =
                            protocol::write_request(&mut input, &work.request).and_then(|_| {
                                protocol::read_response(
                                    &mut output,
                                    work.request.id,
                                    work.request.max_side,
                                )
                            });
                        let broken = result.is_err();
                        let _ = work.reply.send(result);
                        if broken {
                            break;
                        }
                    }
                })?,
        );
        process.sender = Some(sender);
        Ok(process)
    }

    pub fn request(&self, request: Request) -> io::Result<Receiver<io::Result<Response>>> {
        let (reply, receiver) = crossbeam_channel::bounded(1);
        self.sender
            .as_ref()
            .ok_or_else(|| io::Error::other("Missing IPC transport"))?
            .try_send(Work { request, reply })
            .map_err(io::Error::other)?;
        Ok(receiver)
    }

    pub fn exited(&mut self) -> io::Result<bool> {
        self.child.try_wait().map(|status| status.is_some())
    }

    #[cfg(test)]
    pub fn id(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // Kill first so both read_exact and write_all unblock, even on a hung peer.
        self.control.terminate();
        let _ = self.child.kill();
        let _ = self.child.wait();
        drop(self.sender.take());
        if let Some(thread) = self.transport.take() {
            let _ = thread.join();
        }
    }
}
