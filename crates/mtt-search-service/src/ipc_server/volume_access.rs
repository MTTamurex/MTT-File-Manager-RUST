use windows::Win32::Foundation::HANDLE;

use crate::volume_lifecycle::{VolumeLifecycle, VolumeUseGuard};

use super::pipe_io::send_response;

pub(super) fn acquire_all(
    pipe: HANDLE,
    lifecycle: &VolumeLifecycle,
) -> Option<Vec<VolumeUseGuard>> {
    match lifecycle.acquire_all() {
        Ok(guards) => Some(guards),
        Err(()) => {
            let _ = send_response(
                pipe,
                &mtt_search_protocol::SearchResponse::Error(
                    "Volume removal in progress".to_string(),
                ),
            );
            None
        }
    }
}

pub(super) fn acquire_one(
    pipe: HANDLE,
    lifecycle: &VolumeLifecycle,
    drive_letter: char,
) -> Option<Option<VolumeUseGuard>> {
    match lifecycle.acquire_volume(drive_letter) {
        Ok(guard) => Some(guard),
        Err(()) => {
            let _ = send_response(
                pipe,
                &mtt_search_protocol::SearchResponse::Error(
                    "Volume removal in progress".to_string(),
                ),
            );
            None
        }
    }
}
