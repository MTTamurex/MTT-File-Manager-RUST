use windows::Win32::Foundation::HANDLE;

use crate::usn_journal;
use crate::volume_lifecycle::VolumeStopSignal;

struct VolumeHandle(HANDLE);

impl Drop for VolumeHandle {
    fn drop(&mut self) {
        usn_journal::close_volume(self.0);
    }
}

/// Keep raw volume handles inside the I/O operation, never across cache work,
/// index locks or the idle interval between journal queries.
pub(super) fn with_volume<T>(
    drive_letter: char,
    stop: &VolumeStopSignal,
    operation: impl FnOnce(HANDLE) -> Result<T, String>,
) -> Result<T, String> {
    if stop.is_stopped() {
        return Err("Volume I/O cancelled".to_string());
    }
    let volume = VolumeHandle(usn_journal::open_volume(drive_letter)?);
    if stop.is_stopped() {
        return Err("Volume I/O cancelled".to_string());
    }
    with_handle(volume, operation)
}

fn with_handle<T>(
    volume: VolumeHandle,
    operation: impl FnOnce(HANDLE) -> Result<T, String>,
) -> Result<T, String> {
    operation(volume.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::CreateEventW;

    #[test]
    fn volume_handle_is_closed_before_success_or_failure_is_returned() {
        for fail in [false, true] {
            let handle = unsafe { CreateEventW(None, true, false, None).unwrap() };
            let result = with_handle(VolumeHandle(handle), |_| {
                if fail {
                    Err("I/O failed".to_string())
                } else {
                    Ok(())
                }
            });
            assert_eq!(result.is_err(), fail);
            assert!(unsafe { CloseHandle(handle) }.is_err());
        }
    }

    #[test]
    fn volume_handle_is_closed_when_io_panics() {
        let handle = unsafe { CreateEventW(None, true, false, None).unwrap() };
        let result = std::panic::catch_unwind(|| {
            let _ = with_handle::<()>(VolumeHandle(handle), |_| panic!("I/O panic"));
        });
        assert!(result.is_err());
        assert!(unsafe { CloseHandle(handle) }.is_err());
    }
}
