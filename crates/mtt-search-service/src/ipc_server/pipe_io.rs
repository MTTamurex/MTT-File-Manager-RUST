use windows::core::PCWSTR;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Security::{
    InitializeSecurityDescriptor, SetSecurityDescriptorDacl, PSECURITY_DESCRIPTOR,
    SECURITY_ATTRIBUTES,
};
use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows::Win32::System::Pipes::{
    CreateNamedPipeW, PIPE_REJECT_REMOTE_CLIENTS, PIPE_WAIT,
};

use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;

use mtt_search_protocol::*;

use super::{MAX_REQUEST_PAYLOAD, PIPE_BUFFER_SIZE, PIPE_MAX_INSTANCES, PIPE_OPEN_MODE};

pub(super) fn create_pipe() -> Result<HANDLE, String> {
    unsafe {
        // Build an explicit DACL that grants access only to BUILTIN\Users and SYSTEM.
        // This replaces the previous NULL DACL (which allowed ALL access, including
        // guest accounts, network service, and any local malware).
        //
        // ACL layout:
        //   ACL header (8 bytes)
        //   ACE 1: BUILTIN\Users  (SID S-1-5-32-545) — 12-byte SID → ACE size = 20
        //   ACE 2: NT AUTHORITY\SYSTEM (SID S-1-5-18) — 12-byte SID → ACE size = 20
        //
        // Total ACL size = 8 + 20 + 20 = 48 bytes (we allocate 256 for safety).

        // --- Build SIDs ---
        // BUILTIN\Users: S-1-5-32-545
        // SID structure: revision(1) + sub-authority-count(1) + identifier-authority(6) + sub-authorities(4*count)
        // S-1-5-32-545 → revision=1, count=2, authority=[0,0,0,0,0,5], sub-auths=[32, 545]
        let mut sid_users = [0u8; 16]; // 8 + 4*2 = 16 bytes
        sid_users[0] = 1; // Revision
        sid_users[1] = 2; // SubAuthorityCount
        sid_users[7] = 5; // IdentifierAuthority (last byte = 5 for NT Authority)
                          // SubAuthority[0] = 32 (SECURITY_BUILTIN_DOMAIN_RID)
        sid_users[8..12].copy_from_slice(&32u32.to_le_bytes());
        // SubAuthority[1] = 545 (DOMAIN_ALIAS_RID_USERS)
        sid_users[12..16].copy_from_slice(&545u32.to_le_bytes());

        // NT AUTHORITY\SYSTEM: S-1-5-18
        // S-1-5-18 → revision=1, count=1, authority=[0,0,0,0,0,5], sub-auths=[18]
        let mut sid_system = [0u8; 12]; // 8 + 4*1 = 12 bytes
        sid_system[0] = 1; // Revision
        sid_system[1] = 1; // SubAuthorityCount
        sid_system[7] = 5; // IdentifierAuthority
                           // SubAuthority[0] = 18 (SECURITY_LOCAL_SYSTEM_RID)
        sid_system[8..12].copy_from_slice(&18u32.to_le_bytes());

        // --- Build ACL with two ACCESS_ALLOWED_ACEs ---
        // ACCESS_ALLOWED_ACE layout:
        //   ACE_HEADER: AceType(1) + AceFlags(1) + AceSize(2) = 4 bytes
        //   Mask: u32 = 4 bytes
        //   SidStart: variable (rest of SID)
        // Total ACE size = sizeof(ACCESS_ALLOWED_ACE) - sizeof(DWORD) + GetLengthSid(pSid)
        //         = 12 - 4 + sid_len = 8 + sid_len
        let sid_users_len = sid_users.len(); // 16
        let sid_system_len = sid_system.len(); // 12

        let ace1_size = 8 + sid_users_len; // 24
        let ace2_size = 8 + sid_system_len; // 20
        let acl_size = 8 + ace1_size + ace2_size; // 8 + 24 + 20 = 52

        let mut acl_buffer = vec![0u8; acl_size];

        // ACL header (8 bytes):
        //   AclRevision: u8 = 2 (ACL_REVISION)
        //   Sbz1: u8 = 0
        //   AclSize: u16 LE
        //   AceCount: u16 LE
        //   Sbz2: u16 = 0
        acl_buffer[0] = 2; // ACL_REVISION
        acl_buffer[2..4].copy_from_slice(&(acl_size as u16).to_le_bytes());
        acl_buffer[4..6].copy_from_slice(&2u16.to_le_bytes()); // AceCount = 2

        // Separate access masks per principal:
        //
        // Users: FILE_GENERIC_READ | FILE_GENERIC_WRITE minus DELETE, WRITE_DAC,
        //   WRITE_OWNER, FILE_EXECUTE, FILE_DELETE_CHILD.  This is the minimum set
        //   required for GENERIC_READ | GENERIC_WRITE pipe clients.
        //   NOTE: FILE_APPEND_DATA (0x0004) shares the same bit as
        //   FILE_CREATE_PIPE_INSTANCE and is included in Windows' GENERIC_WRITE
        //   mapping, so it cannot be removed without breaking client connections.
        //   Pipe squatting is mitigated by PIPE_MAX_INSTANCES and the restricted DACL
        //   (only BUILTIN\Users + SYSTEM, no guest/network).
        //
        // SYSTEM: FILE_ALL_ACCESS — the service creates and owns pipe instances.
        let access_mask_users: u32 = 0x0012019F;
        let access_mask_system: u32 = 0x001F01FF;

        // ACE 1: BUILTIN\Users
        let ace1_offset = 8;
        acl_buffer[ace1_offset] = 0; // ACCESS_ALLOWED_ACE_TYPE
        acl_buffer[ace1_offset + 1] = 0; // AceFlags
        acl_buffer[ace1_offset + 2..ace1_offset + 4]
            .copy_from_slice(&(ace1_size as u16).to_le_bytes());
        acl_buffer[ace1_offset + 4..ace1_offset + 8]
            .copy_from_slice(&access_mask_users.to_le_bytes());
        acl_buffer[ace1_offset + 8..ace1_offset + 8 + sid_users_len].copy_from_slice(&sid_users);

        // ACE 2: SYSTEM
        let ace2_offset = ace1_offset + ace1_size;
        acl_buffer[ace2_offset] = 0; // ACCESS_ALLOWED_ACE_TYPE
        acl_buffer[ace2_offset + 1] = 0; // AceFlags
        acl_buffer[ace2_offset + 2..ace2_offset + 4]
            .copy_from_slice(&(ace2_size as u16).to_le_bytes());
        acl_buffer[ace2_offset + 4..ace2_offset + 8]
            .copy_from_slice(&access_mask_system.to_le_bytes());
        acl_buffer[ace2_offset + 8..ace2_offset + 8 + sid_system_len].copy_from_slice(&sid_system);

        // --- Build Security Descriptor ---
        let mut sd_buffer = vec![0u8; 256];
        let sd_ptr = PSECURITY_DESCRIPTOR(sd_buffer.as_mut_ptr() as *mut _);

        // SECURITY_DESCRIPTOR_REVISION = 1
        InitializeSecurityDescriptor(sd_ptr, 1)
            .map_err(|e| format!("InitializeSecurityDescriptor: {}", e))?;

        // Set our explicit DACL (not a NULL DACL)
        let acl_ptr = acl_buffer.as_ptr() as *const windows::Win32::Security::ACL;
        SetSecurityDescriptorDacl(sd_ptr, true, Some(acl_ptr), false)
            .map_err(|e| format!("SetSecurityDescriptorDacl: {}", e))?;

        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd_ptr.0,
            bInheritHandle: false.into(),
        };

        let pipe_name: Vec<u16> = PIPE_NAME.encode_utf16().chain(std::iter::once(0)).collect();

        let pipe = CreateNamedPipeW(
            PCWSTR(pipe_name.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(PIPE_OPEN_MODE),
            PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS, // BYTE mode + reject network clients
            PIPE_MAX_INSTANCES,
            PIPE_BUFFER_SIZE,
            PIPE_BUFFER_SIZE,
            0,
            Some(&sa as *const _),
        );

        if pipe.is_invalid() {
            return Err(format!(
                "CreateNamedPipeW failed: {:?}",
                windows::Win32::Foundation::GetLastError()
            ));
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
