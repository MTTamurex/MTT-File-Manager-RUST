use super::super::{model::Snapshot, tests::NativeWindow};
use super::input::Part;
use super::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_DELETE, VK_RIGHT};

#[test]
fn single_non_split_tab_never_expands_and_clicking_restores_without_a_target() {
    let owner = NativeWindow::new();
    for dual in [false, true] {
        for x in [16, 100, layout::WIDTH - 60] {
            let mut state = tests::state(owner.0);
            let mut snapshot = state.model.windows[0].1.clone();
            if !dual {
                snapshot.workspaces[0].panels = None;
            }
            state.model = Model::new(vec![(
                WindowRef::capture(owner.0).unwrap(),
                snapshot.clone(),
            )]);
            let point = POINT {
                x,
                y: layout::HEADER + 20,
            };
            state.pointer_move(point);
            if !dual {
                state.key(VK_RIGHT);
                assert_eq!(state.model.rows[0].expanded, None);
            }
            let hit = state.mouse_hit(point).unwrap();
            assert_eq!(hit.part, Part::Body);
            state.click(hit);
            if dual {
                assert_eq!(state.model.rows[0].expanded, Some(true));
                assert!(!state.done);
                assert!(state.command.is_none());
            } else {
                assert!(
                    matches!(state.command, Some(menu::Command::Open(window)) if window.hwnd == owner.0)
                );
            }
            assert_eq!(state.model.windows[0].1, snapshot);
        }
    }
}

#[test]
fn close_button_has_its_own_hit_area_and_does_not_expand_or_restore() {
    let owner = NativeWindow::new();
    let mut state = tests::state(owner.0);
    let point = POINT {
        x: state.layout.width - 20,
        y: layout::HEADER + state.model.rows[0].height / 2,
    };
    state.pointer_move(point);
    assert_eq!(state.hot_close, Some(0));
    let hit = state.mouse_hit(point).unwrap();
    assert_eq!(hit.part, Part::Close);
    state.click(hit);
    assert!(!state.done);
    assert!(matches!(state.command, Some(menu::Command::Exit(window)) if window.hwnd == owner.0));
    assert_eq!(state.model.rows[0].expanded, Some(false));
}

#[test]
fn close_requires_press_and_release_on_the_same_button() {
    let owner = NativeWindow::new();
    for drag in [false, true] {
        let mut state = tests::state(owner.0);
        let hwnd = create(&mut state).unwrap();
        let y = state.layout.px(layout::HEADER + 36);
        let dpi = state.layout.dpi;
        let position =
            |x: i32| LPARAM(((y as u32) << 16 | ((x * dpi as i32 + 48) / 96) as u32) as isize);
        unsafe {
            SendMessageW(
                hwnd,
                WM_LBUTTONDOWN,
                Some(WPARAM(0)),
                Some(position(state.layout.width - 20)),
            );
            SendMessageW(
                hwnd,
                WM_LBUTTONUP,
                Some(WPARAM(0)),
                Some(position(if drag { 20 } else { state.layout.width - 20 })),
            );
            if drag {
                assert!(!state.done);
                assert!(state.command.is_none());
            } else {
                assert!(matches!(state.command, Some(menu::Command::Exit(_))));
            }
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            DestroyWindow(hwnd).unwrap();
        }
    }
}

#[test]
fn delete_closes_only_the_selected_instance_without_a_submenu() {
    let owner = NativeWindow::new();
    let second = NativeWindow::new();
    let mut state = tests::state(owner.0);
    state.model = Model::new(
        [&owner, &second]
            .map(|window| (WindowRef::capture(window.0).unwrap(), Snapshot::default()))
            .into(),
    );
    state.model.selected = Some(1);
    state.key(VK_DELETE);
    assert!(matches!(state.command, Some(menu::Command::Exit(window)) if window.hwnd == second.0));
}

#[test]
fn scrolling_or_keyboard_navigation_cancels_a_pending_close_click() {
    let owner = NativeWindow::new();
    for keyboard in [false, true] {
        let mut state = tests::state(owner.0);
        let point = POINT {
            x: state.layout.width - 20,
            y: layout::HEADER + 36,
        };
        state.pressed = state.mouse_hit(point);
        assert_eq!(state.pressed.unwrap().part, Part::Close);
        if keyboard {
            state.key(VK_RIGHT);
        } else {
            state.wheel(-120);
        }
        assert!(state.pressed.is_none());
        assert!(!state.done);
        assert!(state.command.is_none());
    }
}

#[test]
fn restore_all_and_exit_have_separate_fixed_footer_hit_areas() {
    let owner = NativeWindow::new();
    for scroll in [0, 300] {
        for restore in [true, false] {
            let mut state = tests::state(owner.0);
            state.model.scroll = scroll;
            let y = state.layout.height - layout::FOOTER
                + if restore {
                    layout::FOOTER_ITEM_HEIGHT / 2
                } else {
                    layout::FOOTER_ITEM_HEIGHT + layout::FOOTER_ITEM_HEIGHT / 2
                };
            let hit = state.mouse_hit(POINT { x: 60, y }).unwrap();
            assert_eq!(hit.part, Part::Body);
            assert_eq!(hit.index, state.model.rows.len() + usize::from(!restore));
            state.click(hit);
            assert!(state.done);
            if restore {
                assert!(matches!(state.command, Some(menu::Command::RestoreAll)));
            } else {
                assert!(matches!(state.command, Some(menu::Command::ExitAll)));
            }
        }
    }
}

#[test]
fn compact_footer_actions_meet_without_overlapping_click_regions() {
    let owner = NativeWindow::new();
    let state = tests::state(owner.0);
    let boundary = state.layout.height - layout::FOOTER_ITEM_HEIGHT;
    let restore = state
        .mouse_hit(POINT {
            x: 60,
            y: boundary - 1,
        })
        .unwrap();
    let exit = state.mouse_hit(POINT { x: 60, y: boundary }).unwrap();
    assert_eq!(restore.index, state.model.rows.len());
    assert_eq!(exit.index, state.model.rows.len() + 1);
    assert_eq!(restore.part, Part::Body);
    assert_eq!(exit.part, Part::Body);
}

#[test]
fn keyboard_can_select_restore_all_without_triggering_exit_or_changing_tabs() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{VK_END, VK_RETURN, VK_SPACE, VK_UP};
    let owner = NativeWindow::new();
    for key in [VK_RETURN, VK_SPACE] {
        let mut state = tests::state(owner.0);
        let original = state.model.windows[0].1.clone();
        state.key(VK_END);
        assert_eq!(state.model.selected, Some(state.model.rows.len() + 1));
        state.key(VK_UP);
        assert_eq!(state.model.selected, Some(state.model.rows.len()));
        state.key(VK_DELETE);
        assert!(state.command.is_none() && !state.done);
        state.key(key);
        assert!(state.done);
        assert!(matches!(state.command, Some(menu::Command::RestoreAll)));
        assert_eq!(state.model.windows[0].1, original);
    }
}
