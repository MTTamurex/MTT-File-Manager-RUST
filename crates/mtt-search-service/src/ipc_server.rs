use std::hint::black_box;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, RwLock};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Security::{
    InitializeSecurityDescriptor, SetSecurityDescriptorDacl, PSECURITY_DESCRIPTOR,
    SECURITY_ATTRIBUTES,
};
use windows::Win32::Storage::FileSystem::{FlushFileBuffers, ReadFile, WriteFile};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_WAIT,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::Win32::System::IO::OVERLAPPED;

use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;

use crate::file_index::{self, IndexState, VolumeIndex};
use mtt_search_protocol::*;

const PIPE_BUFFER_SIZE: u32 = 64 * 1024;
const PIPE_MAX_INSTANCES: u32 = 32;
/// PIPE_ACCESS_DUPLEX (0x3) | FILE_FLAG_OVERLAPPED (0x40000000)
const PIPE_OPEN_MODE: u32 = 0x40000003;

/// Maximum concurrent client handler threads (rate limiting).
const MAX_ACTIVE_CLIENTS: u32 = 8;
/// Maximum payload size for incoming requests (64 KB).
const MAX_REQUEST_PAYLOAD: usize = 64 * 1024;
/// Maximum search query text length in bytes.
const MAX_QUERY_TEXT_LEN: usize = 1024;
/// Maximum results per search query.
const MAX_QUERY_RESULTS: usize = 10_000;

/// Start the IPC server loop.
pub fn run_ipc_server(indices: Arc<RwLock<Vec<VolumeIndex>>>, shutdown: Arc<AtomicBool>) {
    let is_warming = Arc::new(AtomicBool::new(false));
    let active_clients = Arc::new(AtomicU32::new(0));

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        let pipe = match create_pipe() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[IPC] Failed to create pipe: {}", e);
                std::thread::sleep(std::time::Duration::from_secs(1));
                continue;
            }
        };

        // Wait for client with overlapped I/O so we can check shutdown periodically
        let client_connected = wait_for_client(pipe, &shutdown);
        if !client_connected {
            unsafe {
                let _ = CloseHandle(pipe);
            }
            if shutdown.load(Ordering::Relaxed) {
                break;
            }
            continue;
        }

        if shutdown.load(Ordering::Relaxed) {
            unsafe {
                let _ = CloseHandle(pipe);
            }
            break;
        }

        // Rate limiting: reject if too many concurrent clients
        let current = active_clients.load(Ordering::Relaxed);
        if current >= MAX_ACTIVE_CLIENTS {
            eprintln!(
                "[IPC] Rate limit: rejecting connection ({}/{} active)",
                current, MAX_ACTIVE_CLIENTS
            );
            // Try to send an error response before disconnecting
            let _ = send_response(
                pipe,
                &SearchResponse::Error("Server busy, try again later".to_string()),
            );
            unsafe {
                let _ = FlushFileBuffers(pipe);
                let _ = DisconnectNamedPipe(pipe);
                let _ = CloseHandle(pipe);
            }
            continue;
        }

        active_clients.fetch_add(1, Ordering::Relaxed);

        // Handle each client concurrently so one slow query doesn't block all connections.
        let indices_for_client = indices.clone();
        let warming_for_client = is_warming.clone();
        let active_for_client = active_clients.clone();
        let pipe_raw = pipe.0 as usize;
        std::thread::spawn(move || {
            let pipe = HANDLE(pipe_raw as *mut core::ffi::c_void);
            if let Err(e) = catch_unwind(AssertUnwindSafe(|| {
                handle_client(pipe, &indices_for_client, &warming_for_client)
            })) {
                eprintln!("[IPC] Client handler panic: {:?}", e);
            }
            unsafe {
                let _ = FlushFileBuffers(pipe);
                let _ = DisconnectNamedPipe(pipe);
                let _ = CloseHandle(pipe);
            }
            active_for_client.fetch_sub(1, Ordering::Relaxed);
        });
    }
}

/// Wait for a client connection using overlapped I/O with 1-second timeout polling.
/// Returns true if a client connected, false if shutdown or error.
fn wait_for_client(pipe: HANDLE, shutdown: &Arc<AtomicBool>) -> bool {
    unsafe {
        let event = match CreateEventW(None, true, false, None) {
            Ok(e) => e,
            Err(_) => return false,
        };

        let mut overlapped = OVERLAPPED::default();
        overlapped.hEvent = event;

        let result = ConnectNamedPipe(pipe, Some(&mut overlapped));

        if result.is_ok() {
            // Client already connected
            let _ = CloseHandle(event);
            return true;
        }

        let err = windows::Win32::Foundation::GetLastError();
        if err.0 == 535 {
            // ERROR_PIPE_CONNECTED: client connected before we called ConnectNamedPipe
            let _ = CloseHandle(event);
            return true;
        }
        if err.0 != 997 {
            // Not ERROR_IO_PENDING — real error
            eprintln!("[IPC] ConnectNamedPipe failed: error {}", err.0);
            let _ = CloseHandle(event);
            return false;
        }

        // ERROR_IO_PENDING: wait with timeout, checking shutdown flag
        loop {
            if shutdown.load(Ordering::Relaxed) {
                let _ = windows::Win32::System::IO::CancelIo(pipe);
                let _ = CloseHandle(event);
                return false;
            }

            let wait = WaitForSingleObject(event, 1000); // 1 second timeout
            if wait == WAIT_OBJECT_0 {
                let _ = CloseHandle(event);
                return true;
            }
            if wait == WAIT_TIMEOUT {
                // Timeout — loop back and check shutdown
                continue;
            }
            // WAIT_FAILED or other — error
            let _ = windows::Win32::System::IO::CancelIo(pipe);
            let _ = CloseHandle(event);
            return false;
        }
    }
}

fn create_pipe() -> Result<HANDLE, String> {
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
        // Total ACE size = 4 (header) + 4 (mask) + SID_SIZE - 4 (SidStart overlaps first 4 bytes of SID... no)
        // Actually: ACE size = sizeof(ACCESS_ALLOWED_ACE) - sizeof(DWORD) + GetLengthSid(pSid)
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

        // FILE_ALL_ACCESS equivalent for pipes: GENERIC_READ | GENERIC_WRITE | SYNCHRONIZE
        // For Named Pipes the relevant access is FILE_GENERIC_READ | FILE_GENERIC_WRITE
        // We use 0x001F01FF (FILE_ALL_ACCESS) to allow full pipe operations.
        let access_mask: u32 = 0x001F01FF;

        // ACE 1: BUILTIN\Users
        let ace1_offset = 8;
        acl_buffer[ace1_offset] = 0; // ACCESS_ALLOWED_ACE_TYPE
        acl_buffer[ace1_offset + 1] = 0; // AceFlags
        acl_buffer[ace1_offset + 2..ace1_offset + 4]
            .copy_from_slice(&(ace1_size as u16).to_le_bytes());
        acl_buffer[ace1_offset + 4..ace1_offset + 8].copy_from_slice(&access_mask.to_le_bytes());
        acl_buffer[ace1_offset + 8..ace1_offset + 8 + sid_users_len].copy_from_slice(&sid_users);

        // ACE 2: SYSTEM
        let ace2_offset = ace1_offset + ace1_size;
        acl_buffer[ace2_offset] = 0; // ACCESS_ALLOWED_ACE_TYPE
        acl_buffer[ace2_offset + 1] = 0; // AceFlags
        acl_buffer[ace2_offset + 2..ace2_offset + 4]
            .copy_from_slice(&(ace2_size as u16).to_le_bytes());
        acl_buffer[ace2_offset + 4..ace2_offset + 8].copy_from_slice(&access_mask.to_le_bytes());
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
            PIPE_WAIT, // BYTE mode — our protocol already does length-prefix framing
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

fn handle_client(
    pipe: HANDLE,
    indices: &Arc<RwLock<Vec<VolumeIndex>>>,
    is_warming: &Arc<AtomicBool>,
) {
    let request_data = match read_message(pipe) {
        Some(data) => data,
        None => return,
    };

    let request: SearchRequest = match decode_message(&request_data) {
        Ok(r) => r,
        Err(e) => {
            // Log the real error internally, send generic message to client
            eprintln!("[IPC] Failed to decode request: {}", e);
            let _ = send_response(pipe, &SearchResponse::Error("Invalid request".to_string()));
            return;
        }
    };

    match request {
        SearchRequest::Ping => {
            let _ = send_response(pipe, &SearchResponse::Pong);
        }
        SearchRequest::WarmIndex => {
            // Respond immediately so the client is not blocked.
            let _ = send_response(pipe, &SearchResponse::WarmStarted);

            // Only spawn the warming thread if one is not already running.
            if is_warming
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::Relaxed)
                .is_ok()
            {
                let indices_clone = indices.clone();
                let warming_flag = is_warming.clone();
                std::thread::spawn(move || {
                    eprintln!("[IPC] WarmIndex: warming in-memory index...");
                    let start = std::time::Instant::now();
                    if let Ok(lock) = indices_clone.read() {
                        let mut touched = 0u64;
                        for vol in lock.iter() {
                            for (_, record) in &vol.records {
                                black_box(&record.name_lower);
                                touched += 1;
                            }
                        }
                        eprintln!(
                            "[IPC] WarmIndex: touched {} records in {:.2}s",
                            touched,
                            start.elapsed().as_secs_f64()
                        );
                    }
                    warming_flag.store(false, Ordering::SeqCst);
                });
            }
        }
        SearchRequest::GetStatus => {
            let indices_lock = match indices.read() {
                Ok(lock) => lock,
                Err(poisoned) => {
                    eprintln!("[IPC] indices lock poisoned on GetStatus");
                    poisoned.into_inner()
                }
            };
            let mut total_indexed = 0u64;
            let mut volumes = Vec::new();

            for vol in indices_lock.iter() {
                let count = vol.records.len() as u64;
                total_indexed += count;
                volumes.push(VolumeStatus {
                    drive_letter: vol.drive_letter,
                    state: match &vol.state {
                        IndexState::NotStarted => "not_started".to_string(),
                        IndexState::Scanning => "scanning".to_string(),
                        IndexState::Ready => "ready".to_string(),
                        IndexState::Error(e) => format!("error: {}", e),
                    },
                    files_indexed: count,
                });
            }

            let _ = send_response(
                pipe,
                &SearchResponse::Status(IndexStatusInfo {
                    volumes,
                    total_files_indexed: total_indexed,
                }),
            );
        }
        SearchRequest::Query { text, max_results } => {
            // Input validation: cap max_results and text length
            let max_results = (max_results as usize).min(MAX_QUERY_RESULTS);

            let text = if text.len() > MAX_QUERY_TEXT_LEN {
                // Truncate at a char boundary to avoid splitting multi-byte chars
                let truncated = &text[..MAX_QUERY_TEXT_LEN];
                match truncated.char_indices().last() {
                    Some((idx, ch)) => text[..idx + ch.len_utf8()].to_string(),
                    None => String::new(),
                }
            } else {
                text
            };

            if text.is_empty() {
                let _ = send_response(
                    pipe,
                    &SearchResponse::Results {
                        items: Vec::new(),
                        is_final: true,
                        total_found: 0,
                    },
                );
                return;
            }

            let indices_lock = match indices.read() {
                Ok(lock) => lock,
                Err(poisoned) => {
                    eprintln!("[IPC] indices lock poisoned on Query");
                    poisoned.into_inner()
                }
            };
            let results = file_index::search(&indices_lock, &text, max_results);

            let items: Vec<SearchResultItem> = results
                .into_iter()
                .map(|r| SearchResultItem {
                    name: r.name,
                    full_path: r.full_path,
                    is_dir: r.is_dir,
                    size: r.size,
                })
                .collect();

            let total = items.len() as u32;
            let _ = send_response(
                pipe,
                &SearchResponse::Results {
                    items,
                    is_final: true,
                    total_found: total,
                },
            );
        }
    }
}

fn read_message(pipe: HANDLE) -> Option<Vec<u8>> {
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

fn send_response(pipe: HANDLE, response: &SearchResponse) -> Result<(), String> {
    let encoded =
        encode_message(response).map_err(|e| format!("Failed to encode response: {}", e))?;

    let mut bytes_written: u32 = 0;
    unsafe {
        WriteFile(pipe, Some(&encoded), Some(&mut bytes_written), None)
            .map_err(|e| format!("WriteFile failed: {}", e))?;
    }

    Ok(())
}
