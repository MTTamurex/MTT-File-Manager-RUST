//! Window-lifetime identity shared across processes and tray-owner handoffs.
use windows::core::w;
use windows::Win32::Foundation::{HANDLE, HWND};
use windows::Win32::UI::WindowsAndMessaging::{GetPropW, SetPropW};

pub(crate) fn get(hwnd: HWND) -> Option<usize> {
    let value = unsafe { GetPropW(hwnd, w!("MTT.FileManager.InstanceNumber")) };
    (!value.is_invalid()).then_some(value.0 as usize)
}

/// Called while the main-window startup mutex is held, before publishing the
/// window. Existing windows retain their numbers even when Z-order changes.
pub(super) fn assign(hwnd: HWND, windows: &[HWND]) -> windows::core::Result<()> {
    if get(hwnd).is_some() {
        return Ok(());
    }
    let number = windows
        .iter()
        .filter_map(|&window| get(window))
        .max()
        .unwrap_or(0);
    let number = number
        .checked_add(1)
        .filter(|&number| number < isize::MAX as usize)
        .ok_or_else(|| {
            windows::core::Error::new(
                windows::core::HRESULT(0x80004005u32 as i32),
                "Instance number exhausted",
            )
        })?;
    unsafe {
        SetPropW(
            hwnd,
            w!("MTT.FileManager.InstanceNumber"),
            Some(HANDLE(number as *mut _)),
        )
    }
}

#[cfg(test)]
mod tests;
