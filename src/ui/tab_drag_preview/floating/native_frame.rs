//! Strip the Windows non-client frame from the auxiliary preview, never the main window.
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumThreadWindows, GetWindowLongPtrW, GetWindowTextW, SetWindowLongPtrW, SetWindowPos,
    GWL_EXSTYLE, GWL_STYLE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    WS_BORDER, WS_CAPTION, WS_EX_CLIENTEDGE, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_STATICEDGE,
    WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_EX_WINDOWEDGE, WS_THICKFRAME,
};

pub(super) const WINDOW_TITLE: &str = "MTT Tab Drag Preview";
const BORDER_STYLES: isize = (WS_CAPTION.0 | WS_BORDER.0 | WS_THICKFRAME.0) as isize;
const EDGE_STYLES: isize = (WS_EX_WINDOWEDGE.0 | WS_EX_CLIENTEDGE.0 | WS_EX_STATICEDGE.0) as isize;

pub(super) fn suppress() {
    // eframe does not expose a child viewport's HWND. Restrict discovery to this
    // UI thread and our exact internal title/mouse-passthrough identity; other instances
    // and the root window are never candidates.
    let mut preview = None::<HWND>;
    unsafe {
        let _ = EnumThreadWindows(
            GetCurrentThreadId(),
            Some(find_preview),
            LPARAM(&mut preview as *mut Option<HWND> as isize),
        );
    }
    if let Some(hwnd) = preview {
        remove_frame(hwnd);
    }
}

unsafe extern "system" fn find_preview(hwnd: HWND, state: LPARAM) -> BOOL {
    let mut title = [0u16; 64];
    let count = unsafe { GetWindowTextW(hwnd, &mut title) } as usize;
    let passthrough = (WS_EX_TRANSPARENT | WS_EX_LAYERED).0 as isize;
    let matches_style =
        unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } & passthrough == passthrough;
    if matches_style
        && title[..count]
            .iter()
            .copied()
            .eq(WINDOW_TITLE.encode_utf16())
    {
        // The synchronous enumeration owns this pointer until the callback returns.
        unsafe { *(state.0 as *mut Option<HWND>) = Some(hwnd) };
        BOOL(0)
    } else {
        BOOL(1)
    }
}

fn remove_frame(hwnd: HWND) {
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        let extended = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let borderless = style & !BORDER_STYLES;
        let edgeless = (extended & !EDGE_STYLES) | (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE).0 as isize;
        if style != borderless || extended != edgeless {
            SetWindowLongPtrW(hwnd, GWL_STYLE, borderless);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, edgeless);
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
            );
        }
        // Windows 11 can still draw a compositor border even with decorations off.
        const DWMWA_COLOR_NONE: u32 = 0xffff_fffe;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &DWMWA_COLOR_NONE as *const _ as *const _,
            std::mem::size_of::<u32>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &DWMWCP_DONOTROUND as *const _ as *const _,
            std::mem::size_of_val(&DWMWCP_DONOTROUND) as u32,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::{w, PCWSTR};
    use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WS_POPUP};

    struct HiddenWindow(HWND);

    impl HiddenWindow {
        fn new(title: &str) -> Self {
            let title: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
            Self(unsafe {
                CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_WINDOWEDGE,
                    w!("STATIC"),
                    PCWSTR(title.as_ptr()),
                    WS_POPUP | WS_CAPTION | WS_THICKFRAME,
                    0,
                    0,
                    200,
                    60,
                    None,
                    None,
                    None,
                    None,
                )
                .expect("hidden test window")
            })
        }
    }

    impl Drop for HiddenWindow {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }

    #[test]
    fn native_frame_is_removed_only_from_the_preview_and_keeps_mouse_passthrough() {
        let preview = HiddenWindow::new(WINDOW_TITLE);
        let other = HiddenWindow::new("Other window");
        let other_style = unsafe { GetWindowLongPtrW(other.0, GWL_STYLE) };
        let other_extended = unsafe { GetWindowLongPtrW(other.0, GWL_EXSTYLE) };
        assert_ne!(
            unsafe { GetWindowLongPtrW(preview.0, GWL_STYLE) } & BORDER_STYLES,
            0
        );
        suppress();
        let style = unsafe { GetWindowLongPtrW(preview.0, GWL_STYLE) };
        let extended = unsafe { GetWindowLongPtrW(preview.0, GWL_EXSTYLE) };
        assert_eq!(style & BORDER_STYLES, 0);
        assert_eq!(extended & EDGE_STYLES, 0);
        let preserved =
            (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT).0 as isize;
        assert_eq!(extended & preserved, preserved);
        assert_eq!(
            unsafe { GetWindowLongPtrW(other.0, GWL_STYLE) },
            other_style
        );
        assert_eq!(
            unsafe { GetWindowLongPtrW(other.0, GWL_EXSTYLE) },
            other_extended
        );
        suppress(); // Idempotent while the pointer keeps moving.
        assert_eq!(unsafe { GetWindowLongPtrW(preview.0, GWL_STYLE) }, style);
        assert_eq!(
            unsafe { GetWindowLongPtrW(preview.0, GWL_EXSTYLE) },
            extended
        );
    }

    #[test]
    fn a_matching_title_without_preview_styles_is_never_modified() {
        let other = HiddenWindow::new(WINDOW_TITLE);
        let style = unsafe { GetWindowLongPtrW(other.0, GWL_STYLE) };
        let extended = unsafe { GetWindowLongPtrW(other.0, GWL_EXSTYLE) }
            & !((WS_EX_LAYERED | WS_EX_TRANSPARENT).0 as isize);
        unsafe {
            SetWindowLongPtrW(other.0, GWL_EXSTYLE, extended);
        }
        suppress();
        assert_eq!(unsafe { GetWindowLongPtrW(other.0, GWL_STYLE) }, style);
        assert_eq!(unsafe { GetWindowLongPtrW(other.0, GWL_EXSTYLE) }, extended);
    }
}
