//! Native Windows dialogs for mapping and disconnecting network drives.

use std::io;
use windows::core::PWSTR;
use windows::Win32::Foundation::{ERROR_CANCELLED, HWND};
use windows::Win32::NetworkManagement::WNet::{
    WNetConnectionDialog1W, WNetDisconnectDialog1W, CONNDLG_USE_MRU, CONNECTDLGSTRUCTW,
    DISCDLGSTRUCTW, DISC_NO_FORCE, DISC_UPDATE_PROFILE, NETRESOURCEW, RESOURCETYPE_DISK,
};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DialogOutcome {
    Completed,
    Cancelled,
}

fn dialog_outcome(status: u32) -> io::Result<DialogOutcome> {
    match status {
        0 => Ok(DialogOutcome::Completed),
        u32::MAX => Ok(DialogOutcome::Cancelled),
        code if code == ERROR_CANCELLED.0 => Ok(DialogOutcome::Cancelled),
        code => Err(io::Error::from_raw_os_error(code as i32)),
    }
}

pub(crate) fn run_map_dialog(owner: isize) -> io::Result<DialogOutcome> {
    let mut resource = NETRESOURCEW {
        dwType: RESOURCETYPE_DISK,
        ..Default::default()
    };
    let mut dialog = CONNECTDLGSTRUCTW {
        cbStructure: std::mem::size_of::<CONNECTDLGSTRUCTW>() as u32,
        hwndOwner: HWND(owner as *mut _),
        lpConnRes: &mut resource,
        dwFlags: CONNDLG_USE_MRU,
        ..Default::default()
    };
    // The backing resource lives until the synchronous native dialog returns.
    dialog_outcome(unsafe { WNetConnectionDialog1W(&mut dialog) }.0)
}

fn local_device_name(root: &str) -> Option<String> {
    super::normalize_drive_root_path(std::path::Path::new(root)).map(|root| root[..2].to_owned())
}

pub(crate) fn run_disconnect_dialog(owner: isize, root: &str) -> io::Result<DialogOutcome> {
    let device = local_device_name(root)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Expected a drive root"))?;
    let mut device: Vec<u16> = device.encode_utf16().chain(Some(0)).collect();
    let dialog = DISCDLGSTRUCTW {
        cbStructure: std::mem::size_of::<DISCDLGSTRUCTW>() as u32,
        hwndOwner: HWND(owner as *mut _),
        lpLocalName: PWSTR(device.as_mut_ptr()),
        // Ask before forcing open files closed, and remove the saved mapping.
        dwFlags: DISC_NO_FORCE | DISC_UPDATE_PROFILE,
        ..Default::default()
    };
    dialog_outcome(unsafe { WNetDisconnectDialog1W(&dialog) }.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_drive_dialog_classifies_completion_cancellation_and_errors() {
        assert_eq!(dialog_outcome(0).unwrap(), DialogOutcome::Completed);
        assert_eq!(dialog_outcome(u32::MAX).unwrap(), DialogOutcome::Cancelled);
        assert_eq!(
            dialog_outcome(ERROR_CANCELLED.0).unwrap(),
            DialogOutcome::Cancelled
        );
        assert_eq!(dialog_outcome(5).unwrap_err().raw_os_error(), Some(5));
    }

    #[test]
    fn network_drive_dialog_accepts_only_letter_roots() {
        assert_eq!(local_device_name("z:\\"), Some("Z:".into()));
        assert_eq!(local_device_name("Z:/"), Some("Z:".into()));
        assert_eq!(local_device_name("Z:"), Some("Z:".into()));
        for invalid in ["", "Z:\\folder", "\\\\server\\share", "1:\\", "é:\\"] {
            assert_eq!(local_device_name(invalid), None);
        }
    }
}
