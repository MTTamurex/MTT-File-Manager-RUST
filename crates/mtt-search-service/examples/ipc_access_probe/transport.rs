use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;

use mtt_search_protocol::{
    decode_message, encode_message, SearchRequest, SearchResponse, PIPE_CLIENT_ACCESS, PIPE_NAME,
};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Pipes::GetNamedPipeServerProcessId;
use windows_service::service::{ServiceAccess, ServiceState};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

const MAX_DIAGNOSTIC_RESPONSE: usize = 64 * 1024;

pub(super) fn request(request: &SearchRequest) -> Result<SearchResponse, &'static str> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|_| "cannot query the local service manager")?;
    let service = manager
        .open_service("MTTFileManagerSearch", ServiceAccess::QUERY_STATUS)
        .map_err(|_| "registered search service not found or inaccessible")?;
    let status = service
        .query_status()
        .map_err(|_| "cannot query service status")?;
    if status.current_state != ServiceState::Running {
        return Err("registered service is not running");
    }
    let pid = status
        .process_id
        .filter(|pid| *pid != 0)
        .ok_or("service PID unavailable")?;
    let mut pipe = OpenOptions::new()
        .read(true)
        .write(true)
        .access_mode(PIPE_CLIENT_ACCESS)
        // Match the app's SQOS: allow the verified service to check this
        // client's permissions. Send no data before checking the server PID.
        .custom_flags(0x00100000 | 0x00020000)
        .open(PIPE_NAME)
        .map_err(|_| "cannot connect to the registered service pipe")?;
    let mut server_pid = 0;
    unsafe { GetNamedPipeServerProcessId(HANDLE(pipe.as_raw_handle()), &mut server_pid) }
        .map_err(|_| "cannot identify pipe server")?;
    if server_pid != pid {
        return Err("pipe server does not match the registered running service");
    }
    let frame = encode_message(request).map_err(|_| "cannot encode diagnostic request")?;
    pipe.write_all(&frame)
        .map_err(|_| "diagnostic write failed")?;
    read_response(&mut pipe)
}

fn read_response(reader: &mut impl Read) -> Result<SearchResponse, &'static str> {
    let mut length = [0u8; 4];
    reader
        .read_exact(&mut length)
        .map_err(|_| "response header missing or truncated")?;
    let length = u32::from_le_bytes(length) as usize;
    if length == 0 || length > MAX_DIAGNOSTIC_RESPONSE {
        return Err("response exceeds diagnostic size bounds; not treated as a denial");
    }
    let mut payload = vec![0; length];
    reader
        .read_exact(&mut payload)
        .map_err(|_| "response body missing or truncated")?;
    let response: SearchResponse =
        decode_message(&payload).map_err(|_| "response decoding failed")?;
    response
        .validate()
        .map_err(|_| "response validation failed")?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn explicit_denial_round_trips_without_a_service() {
        let frame = encode_message(&SearchResponse::Error("Authorization failed".into())).unwrap();
        assert!(matches!(
            read_response(&mut Cursor::new(frame)),
            Ok(SearchResponse::Error(message)) if message == "Authorization failed"
        ));
    }

    #[test]
    fn invalid_frames_are_inconclusive_not_success() {
        for frame in [
            vec![],
            vec![1, 0],
            0u32.to_le_bytes().to_vec(),
            (MAX_DIAGNOSTIC_RESPONSE as u32 + 1).to_le_bytes().to_vec(),
            vec![4, 0, 0, 0, 1],
            vec![4, 0, 0, 0, 255, 255, 255, 255],
        ] {
            assert!(read_response(&mut Cursor::new(frame)).is_err());
        }
    }

    #[test]
    fn oversized_reply_is_rejected_before_reading_its_body() {
        let mut frame = Cursor::new(u32::MAX.to_le_bytes());
        assert!(read_response(&mut frame).is_err());
        assert_eq!(frame.position(), 4);
    }
}
