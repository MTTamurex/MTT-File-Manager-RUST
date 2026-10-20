//! Carry helper failure stages through its exit status, without logging identities.

use super::AccessError;
use crate::infrastructure::diagnostic_logger::{diag_info, diag_warn, field_label, field_u64};

const MARKER: u32 = 0x2000_0000;

#[derive(Clone, Copy)]
#[repr(u32)]
pub(super) enum HelperStage {
    Elevation = 1,
    Requester,
    CreatePipe,
    WaitClient,
    VerifyClient,
    ReceivePath,
    DecodePath,
    ClientIdentity,
    OpenDirectory,
    InspectDirectory,
    ReadAcl,
    MergeAcl,
    WriteAcl,
    BackupRestorePrivileges,
}

impl HelperStage {
    pub(super) fn encode(self, code: u32) -> u32 {
        // Preserve non-Win32 statuses rather than truncating them.
        if code > 0xffff {
            code
        } else {
            MARKER | ((self as u32) << 16) | code
        }
    }
}

pub(super) fn decode(code: u32) -> (&'static str, u32) {
    let stage = match code >> 16 {
        0x2001 => "helper_elevation",
        0x2002 => "authenticate_requester",
        0x2003 => "create_pipe",
        0x2004 => "wait_client",
        0x2005 => "verify_client",
        0x2006 => "receive_path",
        0x2007 => "decode_path",
        0x2008 => "client_identity",
        0x2009 => "open_directory",
        0x200a => "inspect_directory",
        0x200b => "read_acl",
        0x200c => "merge_acl",
        0x200d => "write_acl",
        0x200e => "enable_backup_restore_privileges",
        _ => return ("helper_exit", code),
    };
    (stage, code & 0xffff)
}

pub(super) fn event(stage: &'static str) {
    diag_info("folder_access", "progress", &[field_label("stage", stage)]);
}

pub(super) fn failure(stage: &'static str, code: u32) -> AccessError {
    let (helper_stage, native_code) = decode(code);
    let stage = if helper_stage == "helper_exit" {
        stage
    } else {
        helper_stage
    };
    diag_warn(
        "folder_access",
        "failed",
        &[
            field_label("stage", stage),
            field_u64("code", native_code as u64),
        ],
    );
    AccessError::Failed(native_code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_stages_preserve_native_error_codes() {
        for stage in [
            HelperStage::Elevation,
            HelperStage::Requester,
            HelperStage::CreatePipe,
            HelperStage::WaitClient,
            HelperStage::VerifyClient,
            HelperStage::ReceivePath,
            HelperStage::DecodePath,
            HelperStage::ClientIdentity,
            HelperStage::OpenDirectory,
            HelperStage::InspectDirectory,
            HelperStage::ReadAcl,
            HelperStage::MergeAcl,
            HelperStage::WriteAcl,
            HelperStage::BackupRestorePrivileges,
        ] {
            let encoded = stage.encode(5);
            assert_ne!(decode(encoded).0, "helper_exit");
            assert_eq!(decode(encoded).1, 5);
            assert_eq!(failure("helper_exit", encoded), AccessError::Failed(5));
        }
        assert_eq!(
            decode(HelperStage::OpenDirectory.encode(5)),
            ("open_directory", 5)
        );
        assert_eq!(decode(HelperStage::WriteAcl.encode(5)), ("write_acl", 5));
        assert_eq!(decode(5), ("helper_exit", 5));
        assert_eq!(HelperStage::WriteAcl.encode(0xc000_0005), 0xc000_0005);
        assert_eq!(failure("launch_helper", 5), AccessError::Failed(5));
    }
}
