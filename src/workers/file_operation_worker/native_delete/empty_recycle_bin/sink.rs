use super::{identity::Identity, tracking::Tracking};
use std::sync::{Arc, Mutex};
use windows::core::{implement, Ref, Result, HRESULT};
use windows::Win32::Foundation::E_ABORT;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::*;

pub(super) fn global(tracking: Arc<Tracking>) -> IFileOperationProgressSink {
    EmptyRecycleBinSink {
        tracking,
        root: None,
    }
    .into()
}

pub(super) fn item(
    tracking: Arc<Tracking>,
    index: usize,
    root: &IShellItem,
) -> IFileOperationProgressSink {
    EmptyRecycleBinSink {
        tracking,
        root: Some(RootCallbacks {
            index,
            item: root.clone(),
            identity: Mutex::new(Identity::new(root)),
        }),
    }
    .into()
}

struct RootCallbacks {
    index: usize,
    item: IShellItem,
    // Refresh with the callback's identity before deletion. COM identity and
    // cached parsing names do not require namespace comparison after deletion.
    identity: Mutex<Option<Identity>>,
}

#[implement(IFileOperationProgressSink)]
struct EmptyRecycleBinSink {
    tracking: Arc<Tracking>,
    // Used only on the worker's STA. A separate sink scopes recursive callbacks
    // to one selected root; the global sink never counts item callbacks.
    root: Option<RootCallbacks>,
}

impl EmptyRecycleBinSink {
    fn check(&self) -> Result<()> {
        if self.tracking.operation.progress.is_cancelled() {
            Err(E_ABORT.into())
        } else {
            Ok(())
        }
    }

    fn item_name(&self, item: &Ref<IShellItem>) -> Option<String> {
        let item = item.as_ref()?;
        unsafe {
            let name = item.GetDisplayName(SIGDN_NORMALDISPLAY).ok()?;
            let value = name.to_string();
            CoTaskMemFree(Some(name.as_ptr() as *mut _));
            value.ok()
        }
    }

    fn is_root(&self, item: &Ref<IShellItem>) -> bool {
        let Some(root) = &self.root else {
            return false;
        };
        item.as_ref().is_some_and(|item| {
            let matches_identity = Identity::new(item).is_some_and(|candidate| {
                root.identity
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .as_ref()
                    .is_some_and(|identity| identity.matches(&candidate))
            });
            matches_identity
                || unsafe {
                    root.item
                        .Compare(item, SICHINT_CANONICAL.0 as u32)
                        .is_ok_and(|order| order == 0)
                }
        })
    }
}

#[allow(non_snake_case, unused_variables)]
impl IFileOperationProgressSink_Impl for EmptyRecycleBinSink_Impl {
    fn StartOperations(&self) -> Result<()> {
        self.check()
    }

    fn FinishOperations(&self, hrresult: HRESULT) -> Result<()> {
        if self.root.is_none() {
            self.tracking.finish_result(hrresult);
        }
        Ok(())
    }

    fn PreDeleteItem(&self, dwflags: u32, psiitem: Ref<IShellItem>) -> Result<()> {
        self.check()?;
        if let Some(root) = &self.root {
            if self.is_root(&psiitem) {
                if let Some(identity) = psiitem.as_ref().and_then(Identity::new) {
                    *root
                        .identity
                        .lock()
                        .unwrap_or_else(|error| error.into_inner()) = Some(identity);
                }
            }
            let name = self.item_name(&psiitem);
            self.tracking
                .operation
                .deletion
                .as_ref()
                .unwrap()
                .set_current(name.as_deref().map(std::path::Path::new));
        }
        Ok(())
    }

    fn PostDeleteItem(
        &self,
        dwflags: u32,
        psiitem: Ref<IShellItem>,
        hrdelete: HRESULT,
        psinewlycreated: Ref<IShellItem>,
    ) -> Result<()> {
        if let Some(root) = &self.root {
            // Do not compare here: the selected namespace item may no longer
            // be comparable after the Shell has removed it.
            let is_root = psiitem
                .as_ref()
                .and_then(Identity::new)
                .is_some_and(|candidate| {
                    root.identity
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .as_ref()
                        .is_some_and(|identity| identity.matches(&candidate))
                });
            self.tracking.record(root.index, is_root, hrdelete);
        }
        self.check()
    }

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
        psznewname: &windows::core::PCWSTR,
    ) -> Result<()> {
        self.check()
    }

    fn PostRenameItem(
        &self,
        dwflags: u32,
        psiitem: Ref<IShellItem>,
        psznewname: &windows::core::PCWSTR,
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
        psznewname: &windows::core::PCWSTR,
    ) -> Result<()> {
        self.check()
    }

    fn PostMoveItem(
        &self,
        dwflags: u32,
        psiitem: Ref<IShellItem>,
        psidestinationfolder: Ref<IShellItem>,
        psznewname: &windows::core::PCWSTR,
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
        psznewname: &windows::core::PCWSTR,
    ) -> Result<()> {
        self.check()
    }

    fn PostCopyItem(
        &self,
        dwflags: u32,
        psiitem: Ref<IShellItem>,
        psidestinationfolder: Ref<IShellItem>,
        psznewname: &windows::core::PCWSTR,
        hrcopy: HRESULT,
        psinewlycreated: Ref<IShellItem>,
    ) -> Result<()> {
        Ok(())
    }

    fn PreNewItem(
        &self,
        dwflags: u32,
        psidestinationfolder: Ref<IShellItem>,
        psznewname: &windows::core::PCWSTR,
    ) -> Result<()> {
        self.check()
    }

    fn PostNewItem(
        &self,
        dwflags: u32,
        psidestinationfolder: Ref<IShellItem>,
        psznewname: &windows::core::PCWSTR,
        psztemplate: &windows::core::PCWSTR,
        dwfileattributes: u32,
        hrnew: HRESULT,
        psinewitem: Ref<IShellItem>,
    ) -> Result<()> {
        Ok(())
    }
}
