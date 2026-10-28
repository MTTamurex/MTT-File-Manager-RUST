use super::super::{
    model::{Entry, LocationKind, Snapshot, Workspace},
    tests::NativeWindow,
};
use super::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_DOWN, VK_ESCAPE, VK_RETURN, VK_RIGHT};

pub(super) fn state(owner: HWND) -> State {
    let model = Model::new(vec![(
        WindowRef::capture(owner).unwrap(),
        expandable_snapshot(),
    )]);
    let anchor = POINT { x: 500, y: 500 };
    let layout = Layout::new(owner, anchor, model.content_height()).unwrap();
    let resources = Resources::new(layout.dpi).unwrap();
    State {
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
    }
}

fn expandable_snapshot() -> Snapshot {
    Snapshot {
        workspaces: vec![Workspace {
            tab: Entry {
                id: 7,
                path: "Example".into(),
                icon: LocationKind::Folder,
            },
            panels: Some([
                vec![Entry {
                    id: 11,
                    path: "Example".into(),
                    icon: LocationKind::Folder,
                }],
                Vec::new(),
            ]),
        }],
        active_path: "Example".into(),
        ..Default::default()
    }
}

#[test]
fn hidden_popup_is_a_tool_window_not_another_main_instance() {
    let owner = NativeWindow::new();
    let mut state = state(owner.0);
    let hwnd = create(&mut state).unwrap();
    unsafe {
        assert!(!IsWindowVisible(hwnd).as_bool());
        assert_ne!(
            GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0,
            0
        );
        assert!(GetPropW(hwnd, w!("MTT.FileManager.MainWindow")).is_invalid());
        SendMessageW(
            hwnd,
            WM_KEYDOWN,
            Some(WPARAM(VK_DOWN.0 as usize)),
            Some(LPARAM(0)),
        );
        assert_eq!(state.model.selected, Some(0));
        SendMessageW(
            hwnd,
            WM_KEYDOWN,
            Some(WPARAM(VK_RIGHT.0 as usize)),
            Some(LPARAM(0)),
        );
        assert_eq!(state.model.rows[0].expanded, Some(true));
        SendMessageW(
            hwnd,
            WM_KEYDOWN,
            Some(WPARAM(VK_RETURN.0 as usize)),
            Some(LPARAM(0)),
        );
        assert!(!state.done);
        assert_eq!(state.model.rows[0].expanded, Some(false));
        assert!(state.command.is_none());
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        DestroyWindow(hwnd).unwrap();
    }
}

#[test]
fn escape_outside_click_and_focus_loss_close_without_a_command() {
    let owner = NativeWindow::new();
    let mut state = state(owner.0);
    let hwnd = create(&mut state).unwrap();
    unsafe {
        for (message, wparam, lparam) in [
            (WM_KEYDOWN, WPARAM(VK_ESCAPE.0 as usize), LPARAM(0)),
            (WM_LBUTTONDOWN, WPARAM(0), LPARAM(0xffff)),
            (WM_ACTIVATE, WPARAM(WA_INACTIVE as usize), LPARAM(0)),
        ] {
            state.done = false;
            SendMessageW(hwnd, message, Some(wparam), Some(lparam));
            assert!(state.done);
            assert!(state.command.is_none());
        }
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        DestroyWindow(hwnd).unwrap();
    }
}

#[test]
fn footer_is_fixed_and_expansion_click_does_not_restore_the_instance() {
    let owner = NativeWindow::new();
    let mut state = state(owner.0);
    let index = state
        .hit(POINT {
            x: 20,
            y: state.layout.height - 20,
        })
        .unwrap();
    assert_eq!(index, state.model.rows.len());
    state.activate(0, true);
    assert!(!state.done);
    assert!(state.command.is_none());
    state.activate(state.model.rows.len(), false);
    assert!(matches!(state.command, Some(menu::Command::ExitAll)));
}

#[test]
fn closed_instances_cannot_receive_popup_commands() {
    let owner = NativeWindow::new();
    let target = NativeWindow::new();
    let reference = WindowRef::capture(target.0).unwrap();
    drop(target);
    while super::super::take_action().is_some() {}
    menu::execute(owner.0, &menu::Command::Open(reference));
    assert!(super::super::take_action().is_none());
}

#[test]
fn popup_fonts_and_bitmaps_are_released_when_resources_drop() {
    use windows::Win32::Graphics::Gdi::{GetObjectW, HGDIOBJ, LOGFONTW};
    let resources = Resources::new(96).unwrap();
    let handles: Vec<HGDIOBJ> = vec![
        resources.font.0.into(),
        resources.bold.0.into(),
        resources.app.as_ref().unwrap().0.into(),
        resources
            .icons
            .get(super::super::model::LocationKind::Folder)
            .unwrap()
            .into(),
    ];
    drop(resources);
    for handle in handles {
        let mut object = LOGFONTW::default();
        assert_eq!(
            unsafe {
                GetObjectW(
                    handle,
                    std::mem::size_of::<LOGFONTW>() as i32,
                    Some((&mut object as *mut LOGFONTW).cast()),
                )
            },
            0
        );
    }
}

#[test]
fn popup_regular_and_bold_fonts_use_the_same_small_size_at_every_dpi() {
    use windows::Win32::Graphics::Gdi::{GetObjectW, LOGFONTW};
    for dpi in [96, 144, 192] {
        let resources = Resources::new(dpi).unwrap();
        for (font, weight) in [(resources.font.0, 400), (resources.bold.0, 600)] {
            let mut description = LOGFONTW::default();
            assert_ne!(
                unsafe {
                    GetObjectW(
                        font.into(),
                        std::mem::size_of::<LOGFONTW>() as i32,
                        Some((&mut description as *mut LOGFONTW).cast()),
                    )
                },
                0
            );
            assert_eq!(description.lfHeight, -(12 * dpi as i32 / 96));
            assert_eq!(description.lfWeight, weight);
        }
    }
}

#[test]
fn pointer_movement_only_highlights_and_click_toggles_the_instance() {
    let owner = NativeWindow::new();
    for x in [16, 100, 200] {
        let mut state = state(owner.0);
        let position = POINT {
            x,
            y: layout::HEADER + 20,
        };
        state.pointer_move(position);
        assert_eq!(state.model.rows[0].expanded, Some(false));
        let hit = state.mouse_hit(position).unwrap();
        state.click(hit);
        assert_eq!(state.model.rows[0].expanded, Some(true));
        assert!(!state.done);
        assert!(state.command.is_none());
        state.click(hit);
        assert_eq!(state.model.rows[0].expanded, Some(false));
        assert!(state.command.is_none());
    }
}
