//! A confirmed batch honors each origin's Shell recycling policy, without per-item confirmations.
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use windows::core::{Error, Result, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{
    SHFileOperationW, FILEOPERATION_FLAGS, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FO_DELETE,
    SHFILEOPSTRUCTW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, IDYES, MB_DEFBUTTON2, MB_ICONWARNING, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO,
    MESSAGEBOX_RESULT, MESSAGEBOX_STYLE,
};

pub(crate) struct DeleteOutcome {
    pub deleted_paths: Vec<PathBuf>,
    pub completed: bool,
}

fn confirmed_flags(permanently: bool) -> FILEOPERATION_FLAGS {
    // Suppression is permitted only after the warning explicitly covers permanent deletion.
    // ALLOWUNDO asks Shell to honor each volume's recycling configuration; it is not a guarantee.
    if permanently {
        FOF_NOCONFIRMATION
    } else {
        FOF_NOCONFIRMATION | FOF_ALLOWUNDO
    }
}

fn is_definitely_absent(path: &Path) -> bool {
    // Access denied and other unknown states must never be interpreted as successful deletion.
    matches!(std::fs::symlink_metadata(path), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
}

fn confirmation_flags() -> MESSAGEBOX_STYLE {
    // The worker's owner is an invisible, non-activating proxy. Without explicit
    // activation/z-order flags, the warning can beep while its dialog stays behind the app.
    MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2 | MB_SETFOREGROUND | MB_TOPMOST
}

#[derive(Debug, PartialEq, Eq)]
enum ConfirmationDecision {
    Confirm,
    Cancel,
    Failed,
}

fn confirmation_decision(result: MESSAGEBOX_RESULT) -> ConfirmationDecision {
    if result == IDYES {
        ConfirmationDecision::Confirm
    } else if result.0 == 0 {
        ConfirmationDecision::Failed
    } else {
        ConfirmationDecision::Cancel
    }
}

pub(crate) fn delete_search_selection(
    paths: &[PathBuf],
    permanently: bool,
    hwnd: HWND,
) -> Result<Option<DeleteOutcome>> {
    if paths.is_empty() {
        return Ok(None);
    }
    let message = if permanently {
        rust_i18n::t!(
            "operations.search_delete_permanent_confirm",
            count = paths.len()
        )
    } else {
        rust_i18n::t!("operations.search_delete_confirm", count = paths.len())
    };
    let message: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
    let title: Vec<u16> = rust_i18n::t!("context_menu.delete")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    // Default to No. Keep this warning visible even though the owner cannot activate.
    log::debug!(
        "[SearchDelete] Opening confirmation for {} items",
        paths.len()
    );
    let result = unsafe {
        MessageBoxW(
            Some(hwnd),
            PCWSTR(message.as_ptr()),
            PCWSTR(title.as_ptr()),
            confirmation_flags(),
        )
    };
    // A zero return is an API failure, not the user choosing No. Read GetLastError
    // before any other Windows call, and never proceed to deletion in this case.
    if confirmation_decision(result) == ConfirmationDecision::Failed {
        return Err(Error::from_thread());
    }
    if confirmation_decision(result) == ConfirmationDecision::Cancel {
        log::debug!("[SearchDelete] Confirmation cancelled");
        return Ok(None);
    }
    log::debug!("[SearchDelete] Confirmation accepted");

    // Only paths known to exist beforehand can be reported as deleted by this operation.
    let existing: Vec<_> = paths
        .iter()
        .filter(|path| std::fs::symlink_metadata(path).is_ok())
        .cloned()
        .collect();
    let mut from = Vec::new();
    for path in paths {
        from.extend(path.as_os_str().encode_wide());
        from.push(0);
    }
    from.push(0);
    let mut operation = SHFILEOPSTRUCTW {
        hwnd,
        wFunc: FO_DELETE,
        pFrom: PCWSTR(from.as_ptr()),
        fFlags: confirmed_flags(permanently).0 as u16,
        ..Default::default()
    };
    // Buffers remain alive throughout the synchronous Shell call on the STA worker.
    let result = unsafe { SHFileOperationW(&mut operation) };
    Ok(Some(DeleteOutcome {
        deleted_paths: existing
            .into_iter()
            .filter(|path| is_definitely_absent(path))
            .collect(),
        completed: result == 0 && !operation.fAnyOperationsAborted.as_bool(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirmed_delete_respects_recycling_unless_explicitly_permanent() {
        assert_eq!(confirmed_flags(false), FOF_NOCONFIRMATION | FOF_ALLOWUNDO);
        assert_eq!(confirmed_flags(true), FOF_NOCONFIRMATION);
    }

    #[test]
    fn absence_check_does_not_remove_existing_items() {
        let directory = tempfile::tempdir().unwrap();
        let present = directory.path().join("file.txt");
        std::fs::write(&present, "test").unwrap();
        assert!(!is_definitely_absent(&present));
        assert!(is_definitely_absent(&directory.path().join("missing.txt")));
    }

    #[test]
    fn search_delete_confirmation_is_visible_above_a_non_activating_proxy_owner() {
        let required = MB_SETFOREGROUND | MB_TOPMOST | MB_DEFBUTTON2;
        assert_eq!(confirmation_flags() & required, required);
        assert_eq!(confirmation_flags() & MB_YESNO, MB_YESNO);
    }

    #[test]
    fn search_delete_confirmation_failure_is_not_silent_cancellation() {
        use windows::Win32::UI::WindowsAndMessaging::{IDCANCEL, IDNO};
        assert_eq!(confirmation_decision(IDYES), ConfirmationDecision::Confirm);
        assert_eq!(confirmation_decision(IDNO), ConfirmationDecision::Cancel);
        assert_eq!(
            confirmation_decision(IDCANCEL),
            ConfirmationDecision::Cancel
        );
        assert_eq!(
            confirmation_decision(MESSAGEBOX_RESULT(0)),
            ConfirmationDecision::Failed
        );
    }
}
