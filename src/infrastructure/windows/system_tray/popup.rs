//! A standalone tool-window popup using the tray owner's application palette.
mod closing;
pub(super) mod handoff;
mod input;
mod layout;
mod model;
mod paint;
mod resources;

use super::{coordinator, menu, window_ref::WindowRef};
use layout::Layout;
use model::Model;
use resources::Resources;
use std::cell::Cell;
use windows::core::w;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateRoundRectRgn, DeleteObject, InvalidateRect, SetWindowRgn,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, ReleaseCapture, SetCapture, SetFocus, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::*;

thread_local! { static ACTIVE: Cell<bool> = const { Cell::new(false) }; }
struct Active;
impl Drop for Active {
    fn drop(&mut self) {
        ACTIVE.with(|active| active.set(false));
    }
}

struct State {
    model: Model,
    layout: Layout,
    resources: Resources,
    owner: HWND,
    anchor: POINT,
    done: bool,
    resize: bool,
    pressed: Option<input::Hit>,
    hot_close: Option<usize>,
    command: Option<menu::Command>,
}

/// False means creation failed: the caller can use the existing native menu.
pub(super) fn show(owner: HWND) -> bool {
    let mut anchor = POINT::default();
    if unsafe { GetCursorPos(&mut anchor) }.is_err() {
        return false;
    }
    show_at(owner, Model::new(coordinator::windows()), anchor)
}

fn show_at(owner: HWND, mut model: Model, anchor: POINT) -> bool {
    if ACTIVE.with(|active| active.replace(true)) {
        return true;
    }
    let _active = Active;
    let Some(reference) = WindowRef::capture(owner) else {
        return true;
    };
    let Some(layout) = Layout::new(owner, anchor, model.content_height()) else {
        return false;
    };
    model.reveal_selection(layout.viewport());
    let Some(resources) = Resources::new(layout.dpi) else {
        return false;
    };
    let mut state = Box::new(State {
        model,
        layout,
        resources,
        owner,
        anchor,
        done: false,
        resize: false,
        pressed: None,
        hot_close: None,
        command: None,
    });
    let Some(hwnd) = create(&mut state) else {
        return false;
    };
    unsafe {
        round_corners(hwnd, &state.layout);
        let _ = SetForegroundWindow(hwnd);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetFocus(Some(hwnd));
        SetCapture(hwnd);
    }
    // Like TrackPopupMenu, keep dispatching the owning thread's messages. This
    // preserves coordination, IPC, painting and shutdown while the app is hidden.
    let mut message = MSG::default();
    while !state.done && reference.is_current() {
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) }.0;
        if result <= 0 {
            if result == 0 {
                unsafe {
                    PostQuitMessage(message.wParam.0 as i32);
                }
            }
            break;
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        // Execute after the native input callback releases its mutable borrow:
        // synchronous IPC can dispatch messages back into this popup.
        if let Some(plan) = closing::take_plan(&mut state) {
            let result = closing::execute(plan);
            closing::apply(&mut state, result);
        }
        if state.resize && !state.done {
            state.resize = false;
            if let Some(layout) =
                Layout::new(state.owner, state.anchor, state.model.content_height())
            {
                state.layout = layout;
                state.model.reveal_selection(state.layout.viewport());
                let bounds = state.layout.bounds;
                unsafe {
                    let _ = SetWindowPos(
                        hwnd,
                        Some(HWND_TOPMOST),
                        bounds.left,
                        bounds.top,
                        bounds.right - bounds.left,
                        bounds.bottom - bounds.top,
                        SWP_NOACTIVATE,
                    );
                }
                round_corners(hwnd, &state.layout);
                // The viewport can stay at its capped size after removing a
                // row. Repaint even when SetWindowPos did not resize anything.
                unsafe {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            }
        }
    }
    unsafe {
        // Clear userdata before destruction; state remains owned by this stack.
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        if GetCapture() == hwnd {
            let _ = ReleaseCapture();
        }
        let _ = DestroyWindow(hwnd);
    }
    if reference.is_current() {
        if let Some(command) = state.command.take() {
            menu::execute(owner, &command);
        }
    }
    true
}

fn create(state: &mut State) -> Option<HWND> {
    unsafe {
        let instance = HINSTANCE(GetModuleHandleW(None).ok()?.0);
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hCursor: LoadCursorW(None, IDC_ARROW).ok()?,
            lpszClassName: w!("MTT.FileManager.TrayPopup"),
            style: CS_DROPSHADOW,
            ..Default::default()
        };
        // The class is process-wide; registering an existing class is harmless.
        RegisterClassW(&class);
        let bounds = state.layout.bounds;
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            class.lpszClassName,
            w!("MTT File Manager"),
            WS_POPUP,
            bounds.left,
            bounds.top,
            bounds.right - bounds.left,
            bounds.bottom - bounds.top,
            None,
            None,
            Some(instance),
            Some((state as *mut State).cast()),
        )
        .ok()
    }
}

fn round_corners(hwnd: HWND, layout: &Layout) {
    crate::infrastructure::windows::window_corners::apply_window_corner_preference(hwnd, false);
    unsafe {
        let region = CreateRoundRectRgn(
            0,
            0,
            layout.bounds.right - layout.bounds.left + 1,
            layout.bounds.bottom - layout.bounds.top + 1,
            layout.px(16),
            layout.px(16),
        );
        if !region.is_invalid() && SetWindowRgn(hwnd, Some(region), true) == 0 {
            let _ = DeleteObject(region.into());
        }
        // SetWindowRgn transfers ownership of a successfully assigned region.
    }
}

fn point(lparam: LPARAM, layout: &Layout) -> POINT {
    POINT {
        x: layout.logical(lparam.0 as u16 as i16 as i32),
        y: layout.logical((lparam.0 >> 16) as u16 as i16 as i32),
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
    }
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut State;
    if pointer.is_null() {
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    }
    if message == WM_ERASEBKGND {
        return LRESULT(1);
    }
    if message == WM_PAINT {
        paint::repaint(hwnd, unsafe { &*pointer });
        return LRESULT(0);
    }
    if !matches!(
        message,
        WM_CLOSE
            | WM_DESTROY
            | WM_ACTIVATE
            | WM_CAPTURECHANGED
            | WM_CANCELMODE
            | WM_MOUSEMOVE
            | WM_LBUTTONDOWN
            | WM_RBUTTONDOWN
            | WM_LBUTTONUP
            | WM_MOUSEWHEEL
            | WM_KEYDOWN
            | WM_SETTINGCHANGE
            | WM_THEMECHANGED
            | WM_SYSCOLORCHANGE
            | WM_DISPLAYCHANGE
            | WM_DPICHANGED
    ) {
        // Do not borrow popup state across DefWindowProc: native calls can
        // synchronously reenter this procedure during creation and resizing.
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    }
    let state = unsafe { &mut *pointer };
    match message {
        WM_CLOSE | WM_DESTROY => state.done = true,
        WM_ACTIVATE if wparam.0 as u16 == WA_INACTIVE as u16 => state.done = true,
        WM_CAPTURECHANGED | WM_CANCELMODE => state.done = true,
        WM_MOUSEMOVE => {
            let position = point(lparam, &state.layout);
            state.pointer_move(position);
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN => {
            let position = point(lparam, &state.layout);
            if position.x < 0
                || position.x >= state.layout.width
                || position.y < 0
                || position.y >= state.layout.height
            {
                state.done = true;
            } else {
                state.pressed = if message == WM_LBUTTONDOWN {
                    state.mouse_hit(position)
                } else {
                    None
                };
            }
        }
        WM_LBUTTONUP => {
            let position = point(lparam, &state.layout);
            if let Some(hit) = state.pressed.take() {
                if state.mouse_hit(position) == Some(hit) {
                    state.click(hit);
                }
            }
        }
        WM_MOUSEWHEEL => state.wheel((wparam.0 >> 16) as u16 as i16 as i32),
        WM_KEYDOWN => state.key(VIRTUAL_KEY(wparam.0 as u16)),
        WM_SETTINGCHANGE | WM_THEMECHANGED | WM_SYSCOLORCHANGE | WM_DISPLAYCHANGE
        | WM_DPICHANGED => state.done = true,
        _ => {}
    }
    let done = state.done;
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
        if done {
            // Sent focus/cancel messages must also wake the nested GetMessage.
            let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        }
    }
    LRESULT(0)
}

#[cfg(test)]
mod input_tests;
#[cfg(test)]
mod render_tests;
#[cfg(test)]
mod tests;
