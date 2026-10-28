use super::*;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, SetWindowPos, ShowWindow, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SW_HIDE, SW_SHOWNOACTIVATE, WINDOW_EX_STYLE, WS_OVERLAPPEDWINDOW,
};

struct Window(HWND);
impl Window {
    fn new() -> Self {
        Self(unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("Instance number test"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                100,
                100,
                None,
                None,
                None,
                None,
            )
            .unwrap()
        })
    }
}
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}

#[test]
fn restoring_hiding_and_reordering_windows_preserves_their_original_numbers() {
    let first = Window::new();
    let second = Window::new();
    assign(first.0, &[]).unwrap();
    assign(second.0, &[first.0]).unwrap();
    for _ in 0..3 {
        unsafe {
            let _ = ShowWindow(second.0, SW_SHOWNOACTIVATE);
            SetWindowPos(
                second.0,
                Some(HWND_TOP),
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
            )
            .unwrap();
            let _ = ShowWindow(second.0, SW_HIDE);
        }
        assign(second.0, &[second.0, first.0]).unwrap();
        assert_eq!(get(first.0), Some(1));
        assert_eq!(get(second.0), Some(2));
    }
}

#[test]
fn closing_an_instance_does_not_renumber_survivors_or_duplicate_live_numbers() {
    let first = Window::new();
    let second = Window::new();
    let third = Window::new();
    assign(first.0, &[]).unwrap();
    assign(second.0, &[first.0]).unwrap();
    assign(third.0, &[second.0, first.0]).unwrap();
    drop(first);
    let fourth = Window::new();
    assign(fourth.0, &[third.0, second.0]).unwrap();
    assert_eq!(get(second.0), Some(2));
    assert_eq!(get(third.0), Some(3));
    assert_eq!(get(fourth.0), Some(4));
}

#[test]
fn numbering_starts_again_when_no_instances_remain_and_invalid_windows_fail() {
    assert!(assign(HWND::default(), &[]).is_err());
    assert_eq!(get(HWND::default()), None);
    let window = Window::new();
    assert_eq!(get(window.0), None);
    assign(window.0, &[]).unwrap();
    assert_eq!(get(window.0), Some(1));
}
