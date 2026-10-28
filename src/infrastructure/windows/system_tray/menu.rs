use super::{
    commands, coordinator, menu_icons::MenuIcons, menu_text::MenuText, model::Target,
    native_popup::Menu, window_ref::WindowRef,
};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, PostMessageW, SetForegroundWindow, TrackPopupMenu, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, WM_NULL,
};

#[derive(Clone)]
pub(super) enum Command {
    Open(WindowRef),
    Activate(WindowRef, Target),
    Exit(WindowRef),
    ExitAll,
}

pub(super) fn show(hwnd: HWND) {
    if !super::popup::show(hwnd) {
        show_native(hwnd);
    }
}

fn show_native(hwnd: HWND) {
    let mut point = POINT::default();
    if unsafe { GetCursorPos(&mut point) }.is_err() {
        return;
    }
    let text = MenuText::new(hwnd, point);
    let icons = MenuIcons::new(text.icon_size());
    let Some(root) = Menu::new(&text, &icons) else {
        return;
    };
    let mut commands = Vec::new();
    let windows = coordinator::windows();
    let window_count = windows.len();
    let paths: Vec<_> = windows
        .iter()
        .map(|(_, snapshot)| {
            if snapshot.active_path.is_empty() {
                rust_i18n::t!("tray.window").into_owned()
            } else {
                snapshot.active_path.clone()
            }
        })
        .collect();
    let window_labels = text.window_labels(paths.iter().map(String::as_str));
    for ((window, snapshot), window_label) in windows.into_iter().zip(window_labels) {
        let Some(group) = Menu::new(&text, &icons) else {
            continue;
        };
        if let Some(time) = snapshot.hidden_at {
            group.information(&format!("{} {}", rust_i18n::t!("tray.hidden_at"), time));
            group.separator();
        }
        group.command(
            &mut commands,
            Command::Open(window),
            &rust_i18n::t!("tray.open"),
        );
        let workspace_labels =
            text.siblings(snapshot.workspaces.iter().map(|w| w.tab.path.as_str()));
        for (workspace, label) in snapshot.workspaces.into_iter().zip(workspace_labels) {
            let target = Target {
                workspace: workspace.tab.id,
                panel: None,
            };
            if let Some(panels) = workspace.panels {
                let Some(outer) = Menu::new(&text, &icons) else {
                    continue;
                };
                outer.command(
                    &mut commands,
                    Command::Activate(window, target),
                    &rust_i18n::t!("tray.open"),
                );
                for (right, entries) in panels.into_iter().enumerate() {
                    let Some(panel) = Menu::new(&text, &icons) else {
                        continue;
                    };
                    let labels = text.siblings(entries.iter().map(|entry| entry.path.as_str()));
                    for (entry, label) in entries.into_iter().zip(labels) {
                        panel.location_command(
                            &mut commands,
                            Command::Activate(
                                window,
                                Target {
                                    workspace: workspace.tab.id,
                                    panel: Some((right == 1, entry.id)),
                                },
                            ),
                            &label,
                            entry.icon,
                        );
                    }
                    outer.submenu(
                        panel,
                        &if right == 0 {
                            rust_i18n::t!("tray.left_panel")
                        } else {
                            rust_i18n::t!("tray.right_panel")
                        },
                        None,
                    );
                }
                group.submenu(outer, &label, Some(workspace.tab.icon));
            } else {
                group.location_command(
                    &mut commands,
                    Command::Activate(window, target),
                    &label,
                    workspace.tab.icon,
                );
            }
        }
        group.window_exit(&mut commands, window, window_count);
        root.submenu(group, &window_label, Some(snapshot.active_icon));
    }
    root.separator();
    root.command(&mut commands, Command::ExitAll, &rust_i18n::t!("tray.exit"));
    let command = unsafe {
        let _ = SetForegroundWindow(hwnd);
        let command = TrackPopupMenu(
            root.handle,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            None,
            hwnd,
            None,
        )
        .0 as usize;
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        command
    };
    let Some(command) = command.checked_sub(1).and_then(|i| commands.get(i)) else {
        return;
    };
    execute(hwnd, command);
}

pub(super) fn execute(hwnd: HWND, command: &Command) {
    if let Command::Open(window) | Command::Activate(window, _) | Command::Exit(window) = command {
        if !window.is_current() {
            return;
        }
    }
    let accepted = match command {
        Command::Open(window) => super::super::app_instances::restore_main_window(window.hwnd),
        Command::Activate(window, target) => commands::activate(hwnd, *window, target.clone()),
        Command::Exit(window) => commands::exit_window(hwnd, *window),
        Command::ExitAll => commands::exit_all(hwnd),
    };
    if !accepted {
        commands::warn_failure(hwnd, matches!(command, Command::ExitAll));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::w;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, GetMenuItemCount, GetMenuItemID, GetMenuState, SetPropW,
        MF_BYPOSITION, MF_GRAYED, WINDOW_EX_STYLE, WS_OVERLAPPEDWINDOW,
    };

    #[test]
    fn timestamp_information_is_disabled_and_does_not_shift_command_ids() {
        let text = MenuText::new(HWND::default(), POINT::default());
        let icons = MenuIcons::new(text.icon_size());
        let menu = Menu::new(&text, &icons).unwrap();
        menu.information("Sent to tray: 2026-01-01 12:30:00");
        menu.separator();
        let mut commands = Vec::new();
        menu.command(&mut commands, Command::ExitAll, "Exit application");
        assert_eq!(commands.len(), 1);
        unsafe {
            assert_eq!(GetMenuItemCount(Some(menu.handle)), 3);
            assert_ne!(GetMenuState(menu.handle, 0, MF_BYPOSITION) & MF_GRAYED.0, 0);
            assert_eq!(GetMenuItemID(menu.handle, 2), 1);
        }
    }

    #[test]
    fn individual_exit_and_its_separator_only_appear_with_multiple_windows() {
        struct TestWindow(HWND);
        impl Drop for TestWindow {
            fn drop(&mut self) {
                unsafe {
                    let _ = DestroyWindow(self.0);
                }
            }
        }
        let native = TestWindow(unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("Tray menu test"),
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
        });
        unsafe {
            SetPropW(
                native.0,
                w!("MTT.FileManager.MainWindow"),
                Some(HANDLE(std::ptr::dangling_mut::<u8>().cast())),
            )
            .unwrap();
        }
        let window = WindowRef::capture(native.0).unwrap();
        let text = MenuText::new(native.0, POINT::default());
        let icons = MenuIcons::new(text.icon_size());
        for count in [1, 2, 3] {
            let group = Menu::new(&text, &icons).unwrap();
            let mut commands = Vec::new();
            group.command(&mut commands, Command::Open(window), "Open");
            group.window_exit(&mut commands, window, count);
            assert_eq!(
                unsafe { GetMenuItemCount(Some(group.handle)) },
                if count == 1 { 1 } else { 3 }
            );
            assert_eq!(commands.len(), if count == 1 { 1 } else { 2 });
            if count > 1 {
                assert!(
                    matches!(commands.last(), Some(Command::Exit(target)) if *target == window)
                );
                assert_eq!(unsafe { GetMenuItemID(group.handle, 2) }, 2);
            }
            let root = Menu::new(&text, &icons).unwrap();
            root.submenu(group, "Window", None);
            root.command(&mut commands, Command::ExitAll, "Exit application");
            assert!(matches!(commands.last(), Some(Command::ExitAll)));
            assert_eq!(unsafe { GetMenuItemCount(Some(root.handle)) }, 2);
        }
    }
}
