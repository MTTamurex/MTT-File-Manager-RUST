use windows::Win32::Foundation::HANDLE;

use crate::volume_lifecycle::{VolumeLifecycle, VolumeUseGuard};

use super::pipe_io::send_response;

pub(super) fn acquire_one(
    pipe: HANDLE,
    lifecycle: &VolumeLifecycle,
    drive_letter: char,
) -> Option<Option<VolumeUseGuard>> {
    acquisition_response(pipe, lifecycle.acquire_hardware_volume(drive_letter))
}

pub(super) fn acquire_indexed(
    pipe: HANDLE,
    lifecycle: &VolumeLifecycle,
    drive_letter: char,
) -> Option<Option<VolumeUseGuard>> {
    if super::drive_indexing::reject_disabled(pipe, lifecycle, drive_letter) {
        return None;
    }
    let result = lifecycle.acquire_volume(drive_letter);
    if result.is_err() && super::drive_indexing::reject_disabled(pipe, lifecycle, drive_letter) {
        return None;
    }
    acquisition_response(pipe, result)
}

fn acquisition_response(
    pipe: HANDLE,
    result: Result<Option<VolumeUseGuard>, ()>,
) -> Option<Option<VolumeUseGuard>> {
    match result {
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
