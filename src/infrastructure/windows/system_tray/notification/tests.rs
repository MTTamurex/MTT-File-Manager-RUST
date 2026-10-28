use super::super::tests::NativeWindow;
use super::*;

fn reset() {
    STATE.with(|s| *s.borrow_mut() = State::default());
}

#[test]
fn notification_identity_is_shared_across_window_owners() {
    let first = NativeWindow::new();
    let second = NativeWindow::new();
    let first_data = identity(first.0);
    let second_data = identity(second.0);
    assert_eq!(first_data.guidItem, second_data.guidItem);
    assert_eq!(first_data.guidItem, TRAY_GUID);
    assert_eq!(first_data.uFlags & NIF_GUID, NIF_GUID);
    assert_ne!(first_data.hWnd, second_data.hWnd);
}

#[test]
fn count_updates_modify_the_existing_icon_and_keep_exact_tooltips() {
    reset();
    let window = NativeWindow::new();
    let mut operations = Vec::new();
    for count in [1, 1, 2, 3, 10, 17, 1] {
        assert!(update_with(window.0, count, |operation, data| {
            operations.push(operation);
            let end = data.szTip.iter().position(|&c| c == 0).unwrap();
            let tooltip = String::from_utf16(&data.szTip[..end]).unwrap();
            assert!(tooltip.contains(&count.to_string()));
            assert_eq!(data.guidItem, TRAY_GUID);
            assert_eq!(data.hWnd, window.0);
            true
        }));
    }
    assert_eq!(
        operations,
        [NIM_ADD, NIM_MODIFY, NIM_MODIFY, NIM_MODIFY, NIM_MODIFY, NIM_MODIFY]
    );
    assert_eq!(
        STATE.with(|s| s.borrow().presentation.as_ref().unwrap().count),
        1
    );
}

#[test]
fn a_failed_refresh_keeps_the_old_icon_usable_and_retries() {
    reset();
    let window = NativeWindow::new();
    assert!(update_with(window.0, 2, |_, _| true));
    assert!(update_with(window.0, 3, |operation, _| {
        assert_eq!(operation, NIM_MODIFY);
        false
    }));
    assert_eq!(
        STATE.with(|s| s.borrow().presentation.as_ref().unwrap().count),
        2
    );
    assert!(update_with(window.0, 3, |operation, _| {
        assert_eq!(operation, NIM_MODIFY);
        true
    }));
    assert_eq!(
        STATE.with(|s| s.borrow().presentation.as_ref().unwrap().count),
        3
    );
}

#[test]
fn a_stale_registration_is_replaced_without_changing_the_guid() {
    reset();
    let window = NativeWindow::new();
    let mut operations = Vec::new();
    assert!(update_with(window.0, 4, |operation, data| {
        assert_eq!(data.guidItem, TRAY_GUID);
        operations.push(operation);
        operations.len() != 1
    }));
    assert_eq!(operations, [NIM_ADD, NIM_DELETE, NIM_ADD]);
}

#[test]
fn failed_additions_are_not_cached_and_lifecycle_invalidation_cannot_be_undone() {
    reset();
    let window = NativeWindow::new();
    assert!(!update_with(window.0, 4, |_, _| false));
    assert!(STATE.with(|s| s.borrow().presentation.is_none()));
    assert!(!update_with(window.0, 4, |_, _| {
        STATE.with(|s| s.borrow_mut().epoch += 1);
        true
    }));
    assert!(!STATE.with(|s| s.borrow().added));
    assert!(update_with(window.0, 4, |operation, _| {
        assert_eq!(operation, NIM_ADD);
        true
    }));
}

#[test]
fn tooltips_are_nul_terminated_without_splitting_utf16_surrogates() {
    let mut data = identity(HWND::default());
    set_tooltip(&mut data, &format!("{}📁end", "a".repeat(125)));
    let end = data.szTip.iter().position(|&c| c == 0).unwrap();
    assert!(end < data.szTip.len());
    assert!(String::from_utf16(&data.szTip[..end]).is_ok());
}

#[test]
fn explorer_restart_forces_registration_even_when_the_count_is_unchanged() {
    reset();
    let window = NativeWindow::new();
    assert!(update_with(window.0, 3, |_, _| true));
    assert!(invalidate_registration());
    assert!(update_with(window.0, 3, |operation, data| {
        assert_eq!(operation, NIM_ADD);
        assert_eq!(data.guidItem, TRAY_GUID);
        true
    }));
    assert_eq!(
        STATE.with(|s| s.borrow().presentation.as_ref().unwrap().count),
        3
    );
}
