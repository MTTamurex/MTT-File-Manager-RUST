use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{GetLastError, LocalFree, HANDLE, HLOCAL};
use windows::Win32::Security::Authorization::{
    SetEntriesInAclW, EXPLICIT_ACCESS_W, SET_ACCESS, TRUSTEE_IS_SID, TRUSTEE_IS_USER,
    TRUSTEE_IS_WELL_KNOWN_GROUP, TRUSTEE_W,
};
use windows::Win32::Security::{
    AllocateAndInitializeSid, FreeSid, InitializeSecurityDescriptor, SetSecurityDescriptorDacl,
    ACL, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, SID_IDENTIFIER_AUTHORITY,
};
use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows::Win32::System::Pipes::{CreateNamedPipeW, PIPE_REJECT_REMOTE_CLIENTS, PIPE_WAIT};

use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;

use mtt_search_protocol::*;

use super::{MAX_REQUEST_PAYLOAD, PIPE_BUFFER_SIZE, PIPE_MAX_INSTANCES, PIPE_OPEN_MODE};

/// FILE_FLAG_FIRST_PIPE_INSTANCE (0x00080000): causes CreateNamedPipeW to fail
/// if a pipe with this name already exists. Prevents pre-emptive pipe squatting
/// where an attacker creates the pipe before the service starts.
const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x00080000;
const ACCESS_MASK_USERS: u32 = 0x0012019F;
const ACCESS_MASK_SYSTEM: u32 = 0x001F01FF;
const SECURITY_DESCRIPTOR_REVISION: u32 = 1;
const SECURITY_NT_AUTHORITY: SID_IDENTIFIER_AUTHORITY = SID_IDENTIFIER_AUTHORITY {
    Value: [0, 0, 0, 0, 0, 5],
};

struct SidGuard(PSID);

impl SidGuard {
    /// SEC: We grant pipe access to `Authenticated Users` (S-1-5-11) instead
    /// of the broader `BUILTIN\Users` (S-1-5-32-545). Authenticated Users
    /// excludes the built-in Guest account and anonymous logons, reducing
    /// the surface for unauthenticated/low-trust local processes to probe
    /// the IPC parser, while still permitting any normal interactive user
    /// (the file manager's intended caller) to connect.
    fn authenticated_users() -> Result<Self, String> {
        Self::allocate(1, 11, 0)
    }

    fn local_system() -> Result<Self, String> {
        Self::allocate(1, 18, 0)
    }

    fn allocate(sub_authority_count: u8, sub0: u32, sub1: u32) -> Result<Self, String> {
        let mut sid = PSID::default();
        unsafe {
            AllocateAndInitializeSid(
                &SECURITY_NT_AUTHORITY,
                sub_authority_count,
                sub0,
                sub1,
                0,
                0,
                0,
                0,
                0,
                0,
                &mut sid,
            )
            .map_err(|e| format!("AllocateAndInitializeSid failed: {}", e))?;
        }
        Ok(Self(sid))
    }

    fn as_trustee_name(&self) -> PWSTR {
        PWSTR(self.0 .0.cast())
    }
}

impl Drop for SidGuard {
    fn drop(&mut self) {
        if !self.0 .0.is_null() {
            unsafe {
                let _ = FreeSid(self.0);
            }
        }
    }
}

struct AclGuard(*mut ACL);

impl AclGuard {
    fn from_entries(entries: &[EXPLICIT_ACCESS_W]) -> Result<Self, String> {
        let mut acl = std::ptr::null_mut::<ACL>();
        let result = unsafe { SetEntriesInAclW(Some(entries), None, &mut acl) };
        if result.0 != 0 {
            return Err(format!(
                "SetEntriesInAclW failed with error code {}",
                result.0
            ));
        }

        Ok(Self(acl))
    }

    fn as_ptr(&self) -> *const ACL {
        self.0 as *const _
    }
}

impl Drop for AclGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                LocalFree(Some(HLOCAL(self.0.cast())));
            }
        }
    }
}

pub(super) fn create_pipe(first_instance: bool) -> Result<HANDLE, String> {
    unsafe {
        let users_sid = SidGuard::authenticated_users()?;
        let system_sid = SidGuard::local_system()?;

        let entries = [
            EXPLICIT_ACCESS_W {
                grfAccessPermissions: ACCESS_MASK_USERS,
                grfAccessMode: SET_ACCESS,
                Trustee: TRUSTEE_W {
                    TrusteeForm: TRUSTEE_IS_SID,
                    TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
                    ptstrName: users_sid.as_trustee_name(),
                    ..Default::default()
                },
                ..Default::default()
            },
            EXPLICIT_ACCESS_W {
                grfAccessPermissions: ACCESS_MASK_SYSTEM,
                grfAccessMode: SET_ACCESS,
                Trustee: TRUSTEE_W {
                    TrusteeForm: TRUSTEE_IS_SID,
                    TrusteeType: TRUSTEE_IS_USER,
                    ptstrName: system_sid.as_trustee_name(),
                    ..Default::default()
                },
                ..Default::default()
            },
        ];
        let acl = AclGuard::from_entries(&entries)?;

        let mut security_descriptor = [0u8; 256];
        let sd_ptr = PSECURITY_DESCRIPTOR(security_descriptor.as_mut_ptr().cast());
        InitializeSecurityDescriptor(sd_ptr, SECURITY_DESCRIPTOR_REVISION)
            .map_err(|e| format!("InitializeSecurityDescriptor: {}", e))?;

        SetSecurityDescriptorDacl(sd_ptr, true, Some(acl.as_ptr()), false)
            .map_err(|e| format!("SetSecurityDescriptorDacl: {}", e))?;

        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd_ptr.0,
            bInheritHandle: false.into(),
        };

        let pipe_name: Vec<u16> = PIPE_NAME.encode_utf16().chain(std::iter::once(0)).collect();

        // SEC: On the first instance, include FILE_FLAG_FIRST_PIPE_INSTANCE so that
        // CreateNamedPipeW fails if the pipe name is already taken (pipe squatting).
        let open_mode = if first_instance {
            PIPE_OPEN_MODE | FILE_FLAG_FIRST_PIPE_INSTANCE
        } else {
            PIPE_OPEN_MODE
        };

        let pipe = CreateNamedPipeW(
            PCWSTR(pipe_name.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(open_mode),
            PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS, // BYTE mode + reject network clients
            PIPE_MAX_INSTANCES,
            PIPE_BUFFER_SIZE,
            PIPE_BUFFER_SIZE,
            0,
            Some(&sa as *const _),
        );

        if pipe.is_invalid() {
            return Err(format!("CreateNamedPipeW failed: {:?}", GetLastError()));
        }

        Ok(pipe)
    }
}

pub(super) fn read_message(pipe: HANDLE) -> Option<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    let mut bytes_read: u32 = 0;

    let ok = unsafe { ReadFile(pipe, Some(&mut len_buf), Some(&mut bytes_read), None) };

    if ok.is_err() || bytes_read != 4 {
        return None;
    }

    let payload_len = u32::from_le_bytes(len_buf) as usize;
    if payload_len == 0 || payload_len > MAX_REQUEST_PAYLOAD {
        return None;
    }

    let mut payload = vec![0u8; payload_len];
    let mut total_read = 0usize;

    while total_read < payload_len {
        let mut chunk_read: u32 = 0;
        let ok = unsafe {
            ReadFile(
                pipe,
                Some(&mut payload[total_read..]),
                Some(&mut chunk_read),
                None,
            )
        };

        if ok.is_err() || chunk_read == 0 {
            return None;
        }

        total_read += chunk_read as usize;
    }

    Some(payload)
}

pub(super) fn send_response(pipe: HANDLE, response: &SearchResponse) -> Result<(), String> {
    let encoded =
        encode_message(response).map_err(|e| format!("Failed to encode response: {}", e))?;
    write_all(pipe, &encoded)
}

fn write_all(pipe: HANDLE, data: &[u8]) -> Result<(), String> {
    let mut offset = 0usize;

    while offset < data.len() {
        let mut bytes_written: u32 = 0;
        unsafe {
            WriteFile(pipe, Some(&data[offset..]), Some(&mut bytes_written), None)
                .map_err(|e| format!("WriteFile failed: {}", e))?;
        }

        if bytes_written == 0 {
            return Err("Pipe closed during write".to_string());
        }

        let written = bytes_written as usize;
        if written > data.len().saturating_sub(offset) {
            return Err("WriteFile wrote beyond buffer bounds".to_string());
        }

        offset += written;
    }

    Ok(())
}
