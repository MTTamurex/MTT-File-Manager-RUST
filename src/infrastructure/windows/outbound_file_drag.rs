//! Native Windows OLE drag source for files selected in the application.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use windows::core::{implement, Error, HRESULT};
use windows::Win32::Foundation::{
    DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, HWND, POINT, RECT,
};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Ole::{
    DoDragDrop, IDropSource, IDropSource_Impl, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK,
    DROPEFFECT_NONE,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, GetCursorPos};

mod file_data_object;

const ALLOWED_EFFECTS: DROPEFFECT = DROPEFFECT(DROPEFFECT_COPY.0 | DROPEFFECT_LINK.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutboundFileDragResult {
    Cancelled,
    ReturnedToSource,
    Dropped(DROPEFFECT),
}

/// Returns true only after the pointer has left the main window's client area.
pub fn cursor_is_outside_client(hwnd: HWND) -> bool {
    cursor_inside_client(hwnd) == Some(false)
}

fn cursor_inside_client(hwnd: HWND) -> Option<bool> {
    if hwnd.0.is_null() {
        return None;
    }

    let mut cursor = POINT::default();
    let mut client = RECT::default();
    unsafe {
        if GetCursorPos(&mut cursor).is_err()
            || GetClientRect(hwnd, &mut client).is_err()
            || !ScreenToClient(hwnd, &mut cursor).as_bool()
        {
            return None;
        }
    }

    Some(
        cursor.x >= client.left
            && cursor.y >= client.top
            && cursor.x < client.right
            && cursor.y < client.bottom,
    )
}

/// Starts the modal OLE drag loop. Winit initializes OLE on the UI thread when
/// it registers the application's existing inbound drop target.
pub fn drag_files(paths: &[PathBuf], source_hwnd: HWND) -> Result<OutboundFileDragResult, String> {
    let data_object = file_data_object::create(paths)?;
    let returned_to_source = Arc::new(AtomicBool::new(false));
    let drop_source: IDropSource = FileDropSource {
        source_hwnd,
        returned_to_source: Arc::clone(&returned_to_source),
    }
    .into();
    // MOVE requires source-side handling of Performed DropEffect and the
    // Recycle Bin TargetCLSID protocol. Until that is implemented, advertise
    // only non-destructive operations.
    let mut effect = DROPEFFECT_NONE;

    let result = unsafe { DoDragDrop(&data_object, &drop_source, ALLOWED_EFFECTS, &mut effect) };
    if result == DRAGDROP_S_DROP {
        Ok(OutboundFileDragResult::Dropped(effect))
    } else if result == DRAGDROP_S_CANCEL {
        if returned_to_source.load(Ordering::Relaxed) {
            Ok(OutboundFileDragResult::ReturnedToSource)
        } else {
            Ok(OutboundFileDragResult::Cancelled)
        }
    } else {
        Err(format!(
            "DoDragDrop failed: {}",
            Error::from_hresult(result)
        ))
    }
}

#[implement(IDropSource)]
struct FileDropSource {
    source_hwnd: HWND,
    returned_to_source: Arc<AtomicBool>,
}

impl IDropSource_Impl for FileDropSource_Impl {
    fn QueryContinueDrag(
        &self,
        escape_pressed: windows::core::BOOL,
        _key_state: MODIFIERKEYS_FLAGS,
    ) -> HRESULT {
        if escape_pressed.as_bool() {
            DRAGDROP_S_CANCEL
        } else if !cursor_is_outside_client(self.source_hwnd) {
            self.returned_to_source.store(true, Ordering::Relaxed);
            DRAGDROP_S_CANCEL
        } else if !crate::infrastructure::windows::key_state::is_primary_mouse_button_down() {
            DRAGDROP_S_DROP
        } else {
            HRESULT(0)
        }
    }

    fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

#[cfg(test)]
mod tests {
    use super::ALLOWED_EFFECTS;
    use windows::Win32::System::Ole::{DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE};

    #[test]
    fn outbound_drag_never_advertises_destructive_move() {
        assert!(ALLOWED_EFFECTS.contains(DROPEFFECT_COPY));
        assert!(ALLOWED_EFFECTS.contains(DROPEFFECT_LINK));
        assert!(!ALLOWED_EFFECTS.contains(DROPEFFECT_MOVE));
    }
}
