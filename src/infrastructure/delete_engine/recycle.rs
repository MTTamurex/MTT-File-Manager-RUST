//! Shell recycling keeps Windows' explicit warning on permanent fallback.
//! Never retry a partially executed Shell operation through another backend.
use crate::infrastructure::copy_engine::CopyOperation;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::Arc;
use windows::core::{implement, Ref, Result, HRESULT, PCWSTR};
use windows::Win32::Foundation::{ERROR_CANCELLED, E_ABORT, HWND};
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::Shell::*;

pub(crate) fn flags() -> FILEOPERATION_FLAGS {
    // SILENT hides only Shell progress, not safety dialogs. Do not add
    // NOCONFIRMATION/NOERRORUI: fallback must always warn before destruction.
    FOF_SILENT | FOF_ALLOWUNDO | FOF_WANTNUKEWARNING | FOFX_RECYCLEONDELETE | FOFX_ADDUNDORECORD
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
) -> Result<bool> {
    unsafe {
        let shell: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER)?;
        shell.SetOwnerWindow(hwnd)?;
        shell.SetOperationFlags(flags())?;
        let sink: IFileOperationProgressSink = Sink {
            operation: operation.clone(),
            removed,
        }
        .into();
        // Queue everything before executing. Queue/setup failure cannot delete.
        for path in paths {
            let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None)?;
            shell.DeleteItem(&item, None)?;
        }
        let cookie = shell.Advise(&sink)?;
        let result = shell.PerformOperations();
        let aborted = shell.GetAnyOperationsAborted();
        let unadvise = shell.Unadvise(cookie);
        if let Err(error) = &result {
            if is_cancellation(error.code()) {
                operation.progress.cancel();
            }
        }
        result?;
        unadvise?;
        Ok(!aborted?.as_bool())
    }
}

#[implement(IFileOperationProgressSink)]
struct Sink {
    operation: Arc<CopyOperation>,
    removed: super::RemovalObserver,
}

fn item_path(item: Ref<IShellItem>) -> Option<PathBuf> {
    unsafe {
        let name = item.as_ref()?.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = name.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(name.0.cast()));
        path
    }
}

impl Sink {
    fn check(&self) -> Result<()> {
        if self.operation.progress.is_cancelled() {
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
        if is_cancellation(hrresult) {
            self.operation.progress.cancel();
        }
        Ok(())
    }
    fn PreDeleteItem(&self, dwflags: u32, psiitem: Ref<IShellItem>) -> Result<()> {
        self.check()?;
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
        if is_cancellation(hrdelete) {
            self.operation.progress.cancel();
        } else if hrdelete.0 == 0 {
            progress.removed();
            if let Some(path) = item_path(psiitem) {
                if matches!(std::fs::symlink_metadata(&path), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
                {
                    (self.removed)(&path);
                }
            }
        } else if hrdelete.is_err() && !self.operation.progress.is_cancelled() {
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
    fn recycling_never_suppresses_permanent_fallback_warning() {
        let flags = flags();
        assert_ne!(flags & FOF_WANTNUKEWARNING, FILEOPERATION_FLAGS(0));
        assert_ne!(flags & FOFX_RECYCLEONDELETE, FILEOPERATION_FLAGS(0));
        assert_eq!(
            flags & (FOF_NOCONFIRMATION | FOF_NOERRORUI),
            FILEOPERATION_FLAGS(0)
        );
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
}
