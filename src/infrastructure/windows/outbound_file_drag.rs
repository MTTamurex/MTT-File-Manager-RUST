//! Native Windows OLE drag source for files selected in the application.

use std::path::PathBuf;

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
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetClientRect, GetCursorPos, WindowFromPoint, GA_ROOTOWNER,
};

mod file_data_object;

const ALLOWED_EFFECTS: DROPEFFECT = DROPEFFECT(DROPEFFECT_COPY.0 | DROPEFFECT_LINK.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutboundFileDragResult {
    Cancelled,
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

fn source_window_is_under_cursor(source_hwnd: HWND) -> Option<bool> {
    if source_hwnd.0.is_null() {
        return None;
    }

    let mut cursor = POINT::default();
    if unsafe { GetCursorPos(&mut cursor) }.is_err() {
        return None;
    }

    let window_under_cursor = unsafe { WindowFromPoint(cursor) };
    if window_under_cursor.0.is_null() {
        return None;
    }

    let source_root = unsafe { GetAncestor(source_hwnd, GA_ROOTOWNER) };
    let hovered_root = unsafe { GetAncestor(window_under_cursor, GA_ROOTOWNER) };
    let source_root = if source_root.0.is_null() {
        source_hwnd
    } else {
        source_root
    };
    let hovered_root = if hovered_root.0.is_null() {
        window_under_cursor
    } else {
        hovered_root
    };

    Some(source_root == hovered_root)
}

fn should_return_to_source(
    cursor_inside_source: Option<bool>,
    source_is_topmost: Option<bool>,
) -> bool {
    cursor_inside_source == Some(true) && source_is_topmost == Some(true)
}

/// Starts the modal OLE drag loop. Winit initializes OLE on the UI thread when
/// it registers the application's existing inbound drop target.
pub fn drag_files(paths: &[PathBuf], source_hwnd: HWND) -> Result<OutboundFileDragResult, String> {
    let data_object = file_data_object::create(paths)?;
    let drop_source: IDropSource = FileDropSource { source_hwnd }.into();
    // MOVE requires source-side handling of Performed DropEffect and the
    // Recycle Bin TargetCLSID protocol. Until that is implemented, advertise
    // only non-destructive operations.
    let mut effect = DROPEFFECT_NONE;

    let result = unsafe { DoDragDrop(&data_object, &drop_source, ALLOWED_EFFECTS, &mut effect) };
    if result == DRAGDROP_S_DROP {
        Ok(OutboundFileDragResult::Dropped(effect))
    } else if result == DRAGDROP_S_CANCEL {
        Ok(OutboundFileDragResult::Cancelled)
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
}

fn query_continue_drag_result(
    escape_pressed: bool,
    primary_down: bool,
    cursor_over_source: bool,
) -> HRESULT {
    if escape_pressed {
        DRAGDROP_S_CANCEL
    } else if primary_down {
        HRESULT(0)
    } else if cursor_over_source {
        // Do not feed the outbound payload back into winit's inbound target.
        DRAGDROP_S_CANCEL
    } else {
        DRAGDROP_S_DROP
    }
}

impl IDropSource_Impl for FileDropSource_Impl {
    fn QueryContinueDrag(
        &self,
        escape_pressed: windows::core::BOOL,
        _key_state: MODIFIERKEYS_FLAGS,
    ) -> HRESULT {
        let cursor_over_source = should_return_to_source(
            cursor_inside_client(self.source_hwnd),
            source_window_is_under_cursor(self.source_hwnd),
        );
        query_continue_drag_result(
            escape_pressed.as_bool(),
            crate::infrastructure::windows::key_state::is_primary_mouse_button_down(),
            cursor_over_source,
        )
    }

    fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

#[cfg(test)]
mod tests {
    use super::{query_continue_drag_result, should_return_to_source, ALLOWED_EFFECTS};
    use windows::core::HRESULT;
    use windows::Win32::Foundation::{DRAGDROP_S_CANCEL, DRAGDROP_S_DROP};
    use windows::Win32::System::Ole::{DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE};

    #[test]
    fn outbound_drag_never_advertises_destructive_move() {
        assert!(ALLOWED_EFFECTS.contains(DROPEFFECT_COPY));
        assert!(ALLOWED_EFFECTS.contains(DROPEFFECT_LINK));
        assert!(!ALLOWED_EFFECTS.contains(DROPEFFECT_MOVE));
    }

    #[test]
    fn overlapping_external_window_does_not_cancel_native_drag() {
        assert!(should_return_to_source(Some(true), Some(true)));
        assert!(!should_return_to_source(Some(true), Some(false)));
        assert!(!should_return_to_source(Some(false), Some(true)));
        assert!(!should_return_to_source(None, Some(true)));
    }

    #[test]
    fn crossing_source_while_held_keeps_native_drag_active() {
        assert_eq!(query_continue_drag_result(false, true, true), HRESULT(0));
        assert_eq!(
            query_continue_drag_result(false, false, true),
            DRAGDROP_S_CANCEL
        );
        assert_eq!(
            query_continue_drag_result(false, false, false),
            DRAGDROP_S_DROP
        );
        assert_eq!(
            query_continue_drag_result(true, true, false),
            DRAGDROP_S_CANCEL
        );
    }
}
