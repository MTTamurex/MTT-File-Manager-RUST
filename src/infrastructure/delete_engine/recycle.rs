//! Shell recycling refuses unrecyclable items before deletion, without native
//! confirmation/error dialogs. Permanent fallback is handled separately by MTT.
//! No automatic permanent fallback: the caller must obtain fresh consent.
use crate::infrastructure::copy_engine::CopyOperation;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use windows::core::{implement, Ref, Result, HRESULT, PCWSTR};
use windows::Win32::Foundation::{ERROR_CANCELLED, E_ABORT, HWND};
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::Shell::*;

pub(crate) fn flags() -> FILEOPERATION_FLAGS {
    // Suppression is safe only together with the mandatory PreDeleteItem guard.
    // Do not authorize native permanent fallback or connected sibling deletions.
    FOF_SILENT
        | FOF_NOCONFIRMATION
        | FOF_NOERRORUI
        | FOFX_EARLYFAILURE
        | FOFX_RECYCLEONDELETE
        | FOFX_ADDUNDORECORD
        | FOF_NO_CONNECTED_ELEMENTS
        | FOFX_SHOWELEVATIONPROMPT
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Completed,
    Unavailable,
    Cancelled,
}

fn is_cancellation(result: HRESULT) -> bool {
    result == E_ABORT
        || result == COPYENGINE_E_USER_CANCELLED
        || result == COPYENGINE_E_CANCELLED
        || result == HRESULT::from_win32(ERROR_CANCELLED.0)
}

pub(crate) fn delete(
    paths: &[PathBuf],
    hwnd: HWND,
    operation: Arc<CopyOperation>,
    removed: super::RemovalObserver,
) -> Result<Status> {
    unsafe {
        let shell: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER)?;
        shell.SetOwnerWindow(hwnd)?;
        shell.SetOperationFlags(flags())?;
        let unavailable = Arc::new(AtomicBool::new(false));
        let sink: IFileOperationProgressSink = Sink {
            operation: operation.clone(),
            removed,
            unavailable: unavailable.clone(),
        }
        .into();
        // Queue everything before executing. Queue/setup failure cannot delete.
        for path in paths {
            let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None)?;
            // Attach the guard directly to every item: no queued item may
            // bypass the recycle-only check.
            shell.DeleteItem(&item, &sink)?;
        }
        let result = shell.PerformOperations();
        let aborted = shell.GetAnyOperationsAborted();
        if unavailable.load(Ordering::Relaxed)
            || result
                .as_ref()
                .is_err_and(|error| is_unavailable(error.code()))
        {
            return Ok(Status::Unavailable);
        }
        if let Err(error) = &result {
            if is_cancellation(error.code()) {
                operation.progress.cancel();
            }
        }
        result?;
        Ok(if aborted?.as_bool() || operation.progress.is_cancelled() {
            Status::Cancelled
        } else {
            Status::Completed
        })
    }
}

#[implement(IFileOperationProgressSink)]
struct Sink {
    operation: Arc<CopyOperation>,
    removed: super::RemovalObserver,
    unavailable: Arc<AtomicBool>,
}

fn item_path(item: Ref<IShellItem>) -> Option<PathBuf> {
    unsafe {
        let name = item.as_ref()?.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = name.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(name.0.cast()));
        path
    }
}

fn is_unavailable(result: HRESULT) -> bool {
    [
        COPYENGINE_E_RECYCLE_BIN_NOT_FOUND,
        COPYENGINE_E_RECYCLE_FORCE_NUKE,
        COPYENGINE_E_RECYCLE_PATH_TOO_LONG,
        COPYENGINE_E_RECYCLE_SIZE_TOO_BIG,
        COPYENGINE_E_RECYCLE_UNKNOWN_ERROR,
    ]
    .contains(&result)
}

impl Sink {
    fn check(&self) -> Result<()> {
        if self.operation.progress.is_cancelled() || self.unavailable.load(Ordering::Relaxed) {
            Err(E_ABORT.into())
        } else {
            Ok(())
        }
    }
}

#[allow(non_snake_case, unused_variables)]
impl IFileOperationProgressSink_Impl for Sink_Impl {
    fn StartOperations(&self) -> Result<()> {
        self.check()
    }
    fn FinishOperations(&self, hrresult: HRESULT) -> Result<()> {
        if is_unavailable(hrresult) {
            self.unavailable.store(true, Ordering::Relaxed);
        }
        if is_cancellation(hrresult) && !self.unavailable.load(Ordering::Relaxed) {
            self.operation.progress.cancel();
        }
        Ok(())
    }
    fn PreDeleteItem(&self, dwflags: u32, psiitem: Ref<IShellItem>) -> Result<()> {
        self.check()?;
        // The recycle flag is supplied by Shell for this specific item, not
        // inferred from drive type, free space, or a stale preflight probe.
        if dwflags & TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32 == 0 {
            self.unavailable.store(true, Ordering::Relaxed);
            return Err(E_ABORT.into());
        }
        self.operation
            .deletion
            .as_ref()
            .unwrap()
            .set_current(item_path(psiitem).as_deref());
        Ok(())
    }
    fn PostDeleteItem(
        &self,
        dwflags: u32,
        psiitem: Ref<IShellItem>,
        hrdelete: HRESULT,
        psinewlycreated: Ref<IShellItem>,
    ) -> Result<()> {
        let progress = self.operation.deletion.as_ref().unwrap();
        if is_unavailable(hrdelete) {
            self.unavailable.store(true, Ordering::Relaxed);
        }
        if is_cancellation(hrdelete) {
            if !self.unavailable.load(Ordering::Relaxed) {
                self.operation.progress.cancel();
            }
        } else if hrdelete.0 == 0 {
            progress.removed();
            if let Some(path) = item_path(psiitem) {
                if matches!(std::fs::symlink_metadata(&path), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
                {
                    (self.removed)(&path);
                }
            }
        } else if hrdelete.is_err()
            && !self.operation.progress.is_cancelled()
            && !self.unavailable.load(Ordering::Relaxed)
        {
            progress.failed();
        }
        self.check()
    }
    // Shell work units are not a stable file count; keep progress indeterminate.
    fn UpdateProgress(&self, iworktotal: u32, iworksofar: u32) -> Result<()> {
        self.check()
    }
    fn ResetTimer(&self) -> Result<()> {
        Ok(())
    }
    fn PauseTimer(&self) -> Result<()> {
        Ok(())
    }
    fn ResumeTimer(&self) -> Result<()> {
        Ok(())
    }
    fn PreRenameItem(
        &self,
        dwflags: u32,
        psiitem: Ref<IShellItem>,
        psznewname: &PCWSTR,
    ) -> Result<()> {
        self.check()
    }
    fn PostRenameItem(
        &self,
        dwflags: u32,
        psiitem: Ref<IShellItem>,
        psznewname: &PCWSTR,
        hrrename: HRESULT,
        psinewlycreated: Ref<IShellItem>,
    ) -> Result<()> {
        Ok(())
    }
    fn PreMoveItem(
        &self,
        dwflags: u32,
        psiitem: Ref<IShellItem>,
        psidestinationfolder: Ref<IShellItem>,
        psznewname: &PCWSTR,
    ) -> Result<()> {
        self.check()
    }
    fn PostMoveItem(
        &self,
        dwflags: u32,
        psiitem: Ref<IShellItem>,
        psidestinationfolder: Ref<IShellItem>,
        psznewname: &PCWSTR,
        hrmove: HRESULT,
        psinewlycreated: Ref<IShellItem>,
    ) -> Result<()> {
        Ok(())
    }
    fn PreCopyItem(
        &self,
        dwflags: u32,
        psiitem: Ref<IShellItem>,
        psidestinationfolder: Ref<IShellItem>,
        psznewname: &PCWSTR,
    ) -> Result<()> {
        self.check()
    }
    fn PostCopyItem(
        &self,
        dwflags: u32,
        psiitem: Ref<IShellItem>,
        psidestinationfolder: Ref<IShellItem>,
        psznewname: &PCWSTR,
        hrcopy: HRESULT,
        psinewlycreated: Ref<IShellItem>,
    ) -> Result<()> {
        Ok(())
    }
    fn PreNewItem(
        &self,
        dwflags: u32,
        psidestinationfolder: Ref<IShellItem>,
        psznewname: &PCWSTR,
    ) -> Result<()> {
        self.check()
    }
    fn PostNewItem(
        &self,
        dwflags: u32,
        psidestinationfolder: Ref<IShellItem>,
        psznewname: &PCWSTR,
        psztemplatename: &PCWSTR,
        dwfileattributes: u32,
        hrnew: HRESULT,
        psinewitem: Ref<IShellItem>,
    ) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shell_dialogs_are_suppressed_and_errors_fail_closed() {
        let flags = flags();
        assert_eq!(flags & FOF_WANTNUKEWARNING, FILEOPERATION_FLAGS(0));
        assert_ne!(flags & FOFX_RECYCLEONDELETE, FILEOPERATION_FLAGS(0));
        assert_eq!(
            flags & (FOF_NOCONFIRMATION | FOF_NOERRORUI),
            FOF_NOCONFIRMATION | FOF_NOERRORUI
        );
        assert_ne!(flags & FOFX_EARLYFAILURE, FILEOPERATION_FLAGS(0));
    }

    #[test]
    fn shell_cancellation_is_not_reported_as_an_io_failure() {
        use crate::infrastructure::copy_engine::CopyRuntime;
        use std::sync::atomic::Ordering;
        let runtime = CopyRuntime::default();
        let job = runtime.start_delete(vec![PathBuf::from("source/file.txt")], false);
        let sink: IFileOperationProgressSink = Sink {
            operation: job.operation.clone(),
            removed: Arc::new(|_| {}),
            unavailable: Arc::new(AtomicBool::new(false)),
        }
        .into();
        unsafe {
            let _ = sink.PostDeleteItem(0, None, COPYENGINE_E_USER_CANCELLED, None);
        }
        assert!(job.operation.progress.is_cancelled());
        assert_eq!(
            job.operation
                .deletion
                .as_ref()
                .unwrap()
                .failures
                .load(Ordering::Relaxed),
            0
        );
        assert_eq!(
            job.operation
                .deletion
                .as_ref()
                .unwrap()
                .removed
                .load(Ordering::Relaxed),
            0
        );
        assert!(unsafe { sink.PreDeleteItem(0, None) }.is_err());
    }

    #[test]
    fn unrecyclable_item_is_aborted_before_any_deletion_and_is_not_user_cancel() {
        use crate::infrastructure::copy_engine::CopyRuntime;
        let runtime = CopyRuntime::default();
        let job = runtime.start_delete(vec![PathBuf::from("source/file.txt")], false);
        let unavailable = Arc::new(AtomicBool::new(false));
        let sink: IFileOperationProgressSink = Sink {
            operation: job.operation.clone(),
            removed: Arc::new(|_| {}),
            unavailable: unavailable.clone(),
        }
        .into();
        assert!(
            unsafe { sink.PreDeleteItem(TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32, None) }.is_ok()
        );
        assert!(unsafe { sink.PreDeleteItem(0, None) }.is_err());
        assert!(unavailable.load(Ordering::Relaxed));
        let _ = unsafe { sink.PostDeleteItem(0, None, E_ABORT, None) };
        unsafe { sink.FinishOperations(E_ABORT) }.unwrap();
        assert!(!job.operation.progress.is_cancelled());
        let deletion = job.operation.deletion.as_ref().unwrap();
        assert_eq!(deletion.removed.load(Ordering::Relaxed), 0);
        assert_eq!(deletion.failures.load(Ordering::Relaxed), 0);
        assert!(
            unsafe { sink.PreDeleteItem(TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32, None) }.is_err()
        );
    }
}
