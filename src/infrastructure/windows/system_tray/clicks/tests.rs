use super::*;

#[test]
fn single_instance_double_click_opens_without_showing_a_menu_on_release() {
    let hwnd = HWND::default();
    let mut clicks = Clicks::default();
    assert_eq!(clicks.event(hwnd, WM_LBUTTONUP, 1), Action::DelayMenu);
    assert_eq!(clicks.pending, Some(hwnd));
    assert_eq!(clicks.event(hwnd, WM_LBUTTONDBLCLK, 1), Action::Open);
    assert_eq!(clicks.pending, None);
    assert_eq!(clicks.event(hwnd, WM_LBUTTONUP, 1), Action::None);
    assert_eq!(clicks.event(hwnd, WM_LBUTTONUP, 1), Action::DelayMenu);
}

#[test]
fn multiple_instances_keep_the_existing_menu_behavior() {
    let mut clicks = Clicks::default();
    let hwnd = HWND::default();
    assert_eq!(clicks.event(hwnd, WM_LBUTTONUP, 2), Action::Menu);
    assert_eq!(clicks.event(hwnd, WM_LBUTTONDBLCLK, 2), Action::None);
    assert_eq!(clicks.event(hwnd, WM_LBUTTONUP, 2), Action::Menu);
    assert_eq!(clicks.pending, None);
}

#[test]
fn right_click_cancels_a_pending_single_click_and_opens_the_menu_immediately() {
    let mut clicks = Clicks::default();
    let hwnd = HWND::default();
    clicks.event(hwnd, WM_LBUTTONUP, 1);
    assert_eq!(clicks.event(hwnd, WM_RBUTTONUP, 1), Action::Menu);
    assert_eq!(clicks.pending, None);
}

#[test]
fn double_click_restores_the_existing_hidden_window() {
    use super::super::{take_action, tests::NativeWindow, TrayAction};
    use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;
    while take_action().is_some() {}
    let window = NativeWindow::new();
    assert!(!unsafe { IsWindowVisible(window.0) }.as_bool());
    // Inject the count so other running instances cannot affect this regression.
    handle_with_count(window.0, WM_LBUTTONUP, 1);
    handle_with_count(window.0, WM_LBUTTONDBLCLK, 1);
    handle_with_count(window.0, WM_LBUTTONUP, 1);
    assert!(unsafe { IsWindowVisible(window.0) }.as_bool());
    assert_eq!(take_action(), Some(TrayAction::Open));
    assert!(CLICKS.with(|state| state.borrow().pending.is_none()));
    cancel(window.0);
}
