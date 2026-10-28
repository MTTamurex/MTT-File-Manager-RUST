//! Defer a single left click so the menu cannot consume a double click.
use std::cell::RefCell;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
use windows::Win32::UI::WindowsAndMessaging::{
    KillTimer, SetTimer, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_RBUTTONUP,
};

const CLICK_TIMER_ID: usize = 0x4D545439;

#[derive(Default)]
struct Clicks {
    pending: Option<HWND>,
    suppress_release: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum Action {
    None,
    DelayMenu,
    Menu,
    Open,
}

impl Clicks {
    fn event(&mut self, hwnd: HWND, event: u32, count: usize) -> Action {
        match event {
            WM_LBUTTONDBLCLK if count == 1 => {
                self.pending = None;
                self.suppress_release = true;
                Action::Open
            }
            WM_LBUTTONUP if self.suppress_release => {
                self.suppress_release = false;
                Action::None
            }
            WM_LBUTTONUP if count == 1 => {
                self.pending = Some(hwnd);
                Action::DelayMenu
            }
            WM_LBUTTONUP | WM_RBUTTONUP => {
                self.pending = None;
                self.suppress_release = false;
                Action::Menu
            }
            _ => Action::None,
        }
    }
}

thread_local! { static CLICKS: RefCell<Clicks> = RefCell::new(Clicks::default()); }

pub(super) fn handle(hwnd: HWND, event: u32) {
    handle_with_count(hwnd, event, super::coordinator::window_count());
}

fn handle_with_count(hwnd: HWND, event: u32, count: usize) {
    let action = CLICKS.with(|state| state.borrow_mut().event(hwnd, event, count));
    match action {
        Action::DelayMenu => {
            let timer = unsafe {
                SetTimer(
                    Some(hwnd),
                    CLICK_TIMER_ID,
                    GetDoubleClickTime(),
                    Some(show_delayed_menu),
                )
            };
            if timer == 0 {
                cancel(hwnd);
                super::menu::show(hwnd);
            }
        }
        Action::Open => {
            unsafe {
                let _ = KillTimer(Some(hwnd), CLICK_TIMER_ID);
            }
            super::request_open(hwnd);
        }
        Action::Menu => {
            cancel(hwnd);
            super::menu::show(hwnd);
        }
        Action::None => {}
    }
}

pub(super) fn cancel(hwnd: HWND) {
    unsafe {
        let _ = KillTimer(Some(hwnd), CLICK_TIMER_ID);
    }
    CLICKS.with(|state| *state.borrow_mut() = Clicks::default());
}

unsafe extern "system" fn show_delayed_menu(hwnd: HWND, _: u32, _: usize, _: u32) {
    unsafe {
        let _ = KillTimer(Some(hwnd), CLICK_TIMER_ID);
    }
    let pending = CLICKS.with(|state| state.borrow_mut().pending.take());
    if pending == Some(hwnd) {
        super::menu::show(hwnd);
    }
}

#[cfg(test)]
mod tests;
