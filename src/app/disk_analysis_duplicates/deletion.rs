//! Explicit single-file deletion orchestration. File removal itself belongs
//! exclusively to the application's existing Windows Shell integration.
use super::*;
use crate::app::disk_analysis_model::DiskAnalysisModel;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, TryRecvError},
};

#[cfg(test)]
mod tests;
mod validation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeleteStatus {
    Verifying,
    Removed,
    Unsafe,
    CancelledOrFailed,
    WorkerFailed,
}

#[derive(Clone)]
struct Request {
    report: Arc<DuplicateReport>,
    target: DuplicateMember,
    survivor: DuplicateMember,
    target_path: PathBuf,
    survivor_path: PathBuf,
}

pub(super) struct Completion {
    request: Request,
    pub status: DeleteStatus,
    pub shell_called: bool,
}

#[derive(Default)]
pub struct DuplicateDeletion {
    receiver: Option<Receiver<Completion>>,
    cancel: Arc<AtomicBool>,
    pub status: Option<DeleteStatus>,
}

impl DuplicateDeletion {
    pub fn is_running(&self) -> bool {
        self.receiver.is_some()
    }
    pub fn is_open(&self) -> bool {
        self.is_running() || self.status.is_some()
    }

    pub fn start(
        &mut self,
        report: &Arc<DuplicateReport>,
        review: &DuplicateReview,
        model: &DiskAnalysisModel,
        idx: u32,
        hwnd: isize,
    ) -> bool {
        if self.is_open() {
            return false;
        }
        let Some(request) = Self::prepare(report, review, model, idx) else {
            return false;
        };
        self.launch(request, hwnd);
        true
    }

    fn prepare(
        report: &Arc<DuplicateReport>,
        review: &DuplicateReview,
        model: &DiskAnalysisModel,
        idx: u32,
    ) -> Option<Request> {
        if !review.matches_report(report) || !review.current(idx) {
            return None;
        }
        let group = report.groups.get(review.active_group?)?;
        let target = group.members.iter().find(|m| m.idx == idx)?;
        let other = review
            .entries
            .iter()
            .flatten()
            .find(|e| e.idx != idx && review.current(e.idx))?;
        let survivor = group.members.iter().find(|m| m.idx == other.idx)?;
        if target.physical_identity() == survivor.physical_identity() {
            return None;
        }
        Some(Request {
            report: report.clone(),
            target: target.clone(),
            survivor: survivor.clone(),
            target_path: PathBuf::from(model.path_of(idx)),
            survivor_path: PathBuf::from(model.path_of(other.idx)),
        })
    }

    pub fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if !self.is_running() {
            self.status = None;
        }
    }

    fn launch(&mut self, request: Request, hwnd: isize) {
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        let result = std::thread::Builder::new()
            .name("duplicate-deletion".into())
            .spawn(move || {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    validation::execute(&request, &cancel, |path| native_delete(path, hwnd))
                }));
                let (status, shell_called) = outcome.unwrap_or((DeleteStatus::WorkerFailed, true));
                let _ = sender.send(Completion {
                    request,
                    status,
                    shell_called,
                });
            });
        if result.is_ok() {
            self.receiver = Some(receiver);
            self.status = Some(DeleteStatus::Verifying);
        } else {
            self.status = Some(DeleteStatus::WorkerFailed);
        }
    }

    pub(super) fn poll(&mut self) -> Option<Completion> {
        let result = self.receiver.as_ref()?.try_recv();
        match result {
            Ok(completion) => {
                self.receiver = None;
                self.status = Some(completion.status);
                Some(completion)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.receiver = None;
                self.status = Some(DeleteStatus::WorkerFailed);
                None
            }
        }
    }
}

impl Drop for DuplicateDeletion {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

pub(super) fn apply_completion(session: &mut DuplicateSession, completion: Completion) {
    session.snapshot_stale |= completion.shell_called;
    if !session
        .report
        .as_ref()
        .is_some_and(|r| Arc::ptr_eq(r, &completion.request.report))
    {
        return;
    }
    if completion.status == DeleteStatus::Removed {
        let report = Arc::make_mut(session.report.as_mut().unwrap());
        report::remove_member(report, completion.request.target.idx);
        let survivor = completion.request.survivor.idx;
        let group = report
            .groups
            .iter()
            .position(|g| g.members.iter().any(|m| m.idx == survivor));
        session.review.reset();
        if let Some(group) = group {
            session
                .review
                .open_group(session.report.as_ref().unwrap(), group);
        }
    } else {
        session.review.refresh();
    }
}

#[cfg(windows)]
fn native_delete(path: &std::path::Path, hwnd: isize) -> bool {
    use windows::Win32::{
        Foundation::HWND,
        System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
    };
    // Shell dialogs run on a dedicated STA thread, never on the egui thread.
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if initialized.is_err() {
        return false;
    }
    let result = crate::infrastructure::windows::shell_operations::delete_items_with_shell(
        &[path.to_path_buf()],
        HWND(hwnd as *mut _),
    );
    unsafe {
        CoUninitialize();
    }
    result
}

#[cfg(not(windows))]
fn native_delete(_path: &std::path::Path, _hwnd: isize) -> bool {
    false
}
