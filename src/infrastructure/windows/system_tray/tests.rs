use super::model::{menu_label, Entry, LocationKind, Packet, Snapshot, Target, Workspace};
use super::*;
use windows::core::w;
use windows::Win32::Foundation::{HANDLE, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, DispatchMessageW, GetMessageW, PostMessageW, SetPropW, MSG,
    WINDOW_EX_STYLE, WM_COPYDATA, WS_OVERLAPPEDWINDOW,
};

pub(super) struct NativeWindow(pub(super) HWND);
impl NativeWindow {
    pub(super) fn new() -> Self {
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("Tray protocol test"),
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
        };
        unsafe {
            SetPropW(
                hwnd,
                w!("MTT.FileManager.MainWindow"),
                Some(HANDLE(std::ptr::dangling_mut::<u8>().cast())),
            )
            .unwrap();
        }
        Self(hwnd)
    }
}
impl Drop for NativeWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}

#[test]
fn tray_window_references_reject_destroyed_or_exiting_windows() {
    let window = NativeWindow::new();
    let reference = window_ref::WindowRef::capture(window.0).unwrap();
    assert!(reference.is_current());
    unsafe {
        SetPropW(
            window.0,
            w!("MTT.FileManager.Exiting"),
            Some(HANDLE(std::ptr::dangling_mut::<u8>().cast())),
        )
        .unwrap();
    }
    assert!(!reference.is_current());
    drop(window);
    assert!(!reference.is_current());
}

#[test]
fn native_protocol_rejects_unknown_senders_and_invalid_payloads() {
    let sender = NativeWindow::new();
    let target = Target {
        workspace: 9,
        panel: Some((true, 4)),
    };
    let bytes = serde_json::to_vec(&Packet::Activate(target.clone())).unwrap();
    let mut data = COPYDATASTRUCT {
        dwData: 0x4D545438,
        cbData: bytes.len() as u32,
        lpData: bytes.as_ptr().cast_mut().cast(),
    };
    let receive = |sender, data: &COPYDATASTRUCT| unsafe {
        receive_packet(sender, LPARAM(data as *const _ as isize))
    };
    assert_eq!(receive(HWND::default(), &data), Some(false));
    data.cbData = u32::MAX;
    assert_eq!(receive(sender.0, &data), Some(false));
    data.cbData = bytes.len() as u32;
    assert_eq!(receive(sender.0, &data), Some(true));
    assert_eq!(take_action(), Some(TrayAction::Activate(target)));
    data.lpData = std::ptr::null_mut();
    assert_eq!(receive(sender.0, &data), Some(false));
    let malformed = b"not a tray packet";
    data.lpData = malformed.as_ptr().cast_mut().cast();
    data.cbData = malformed.len() as u32;
    assert_eq!(receive(sender.0, &data), Some(false));
    data.dwData = 0;
    assert_eq!(receive(sender.0, &data), None); // leave unrelated WM_COPYDATA alone
}

#[test]
fn native_protocol_delivers_activation_to_another_ui_thread() {
    use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    const DONE: u32 = 0x8000 + 0x7f;
    extern "system" fn receiver(
        hwnd: HWND,
        msg: u32,
        wp: WPARAM,
        lp: LPARAM,
        _: usize,
        _: usize,
    ) -> LRESULT {
        if msg == WM_COPYDATA {
            if let Some(accepted) = unsafe { receive_packet(HWND(wp.0 as *mut _), lp) } {
                return LRESULT(isize::from(accepted));
            }
        }
        unsafe { DefSubclassProc(hwnd, msg, wp, lp) }
    }
    let sender = NativeWindow::new();
    let (tx, rx) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let receiver_window = NativeWindow::new();
        assert!(unsafe { SetWindowSubclass(receiver_window.0, Some(receiver), 88, 0) }.as_bool());
        tx.send(receiver_window.0 .0 as usize).unwrap();
        let mut msg = MSG::default();
        while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
            if msg.message == DONE {
                break;
            }
            unsafe {
                DispatchMessageW(&msg);
            }
        }
        take_action()
    });
    let receiver = HWND(rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap() as *mut _);
    let target = Target {
        workspace: 3,
        panel: Some((false, 17)),
    };
    let delivered = protocol::send(sender.0, receiver, &Packet::Activate(target.clone()));
    unsafe {
        PostMessageW(Some(receiver), DONE, WPARAM(0), LPARAM(0)).unwrap();
    }
    let action = thread.join().unwrap();
    assert!(delivered);
    assert_eq!(action, Some(TrayAction::Activate(target)));
}

#[test]
fn menu_labels_preserve_paths_without_native_accelerators_or_columns() {
    assert_eq!(menu_label("C:\\Documents\\A&B"), "C:\\Documents\\A&&B");
    assert_eq!(menu_label("folder\tname\n\0"), "folder name  ");
}

#[test]
fn protocol_round_trips_workspace_and_both_panel_tabs() {
    let snapshot = Snapshot {
        workspaces: vec![Workspace {
            tab: Entry {
                id: 7,
                path: "C:\\Documents".into(),
                icon: LocationKind::Folder,
            },
            panels: Some([
                vec![Entry {
                    id: 3,
                    path: "C:\\Documents".into(),
                    icon: LocationKind::Folder,
                }],
                vec![Entry {
                    id: 4,
                    path: "D:\\Videos".into(),
                    icon: LocationKind::Folder,
                }],
            ]),
        }],
        active_path: "D:\\Videos".into(),
        active_icon: LocationKind::Folder,
        hidden_at: Some("2026-01-01 12:30:00".into()),
    };
    let bytes = serde_json::to_vec(&Packet::Snapshot(snapshot.clone())).unwrap();
    let Packet::Snapshot(decoded) = serde_json::from_slice(&bytes).unwrap() else {
        panic!("unexpected packet")
    };
    assert_eq!(snapshot, decoded);
    let target = Target {
        workspace: 7,
        panel: Some((true, 4)),
    };
    let bytes = serde_json::to_vec(&Packet::Activate(target.clone())).unwrap();
    let Packet::Activate(decoded) = serde_json::from_slice(&bytes).unwrap() else {
        panic!("unexpected packet")
    };
    assert_eq!(target, decoded);
}

#[test]
fn explicit_exit_takes_priority_over_queued_open_and_activation() {
    while take_action().is_some() {}
    queue_action(TrayAction::Open);
    queue_action(TrayAction::Activate(Target {
        workspace: 1,
        panel: None,
    }));
    queue_action(TrayAction::Exit);
    queue_action(TrayAction::Open);
    assert_eq!(take_action(), Some(TrayAction::Exit));
    assert_eq!(take_action(), None);
}

#[test]
fn visible_window_clears_hidden_timestamp_but_snapshot_updates_preserve_it() {
    coordinator::mark_hidden();
    coordinator::publish(Snapshot::default());
    // Inspect through a test-only accessor without creating a shell icon.
    assert!(coordinator::test_snapshot().hidden_at.is_some());
    coordinator::mark_visible();
    assert!(coordinator::test_snapshot().hidden_at.is_none());
}

#[test]
fn tray_targets_follow_ids_after_reordering_and_reject_closed_tabs() {
    let mut snapshot = Snapshot {
        workspaces: vec![
            Workspace {
                tab: Entry {
                    id: 1,
                    path: "C:\\Documents".into(),
                    icon: LocationKind::Folder,
                },
                panels: None,
            },
            Workspace {
                tab: Entry {
                    id: 2,
                    path: "C:\\Documents".into(),
                    icon: LocationKind::Folder,
                },
                panels: Some([
                    vec![Entry {
                        id: 8,
                        path: "D:\\Videos".into(),
                        icon: LocationKind::Folder,
                    }],
                    vec![
                        Entry {
                            id: 9,
                            path: "D:\\Videos".into(),
                            icon: LocationKind::Folder,
                        },
                        Entry {
                            id: 10,
                            path: "D:\\Videos".into(),
                            icon: LocationKind::Folder,
                        },
                    ],
                ]),
            },
        ],
        ..Default::default()
    };
    let target = Target {
        workspace: 2,
        panel: Some((true, 10)),
    };
    assert_eq!(snapshot.resolve(&target).unwrap().workspace_index, 1);
    snapshot.workspaces.swap(0, 1);
    let panels = snapshot.workspaces[0].panels.as_mut().unwrap();
    panels[1].swap(0, 1);
    let resolved = snapshot.resolve(&target).unwrap();
    assert_eq!(resolved.workspace_index, 0);
    assert_eq!(resolved.panel, Some((true, 0)));
    // The same ID in the opposite panel must never be selected as a fallback.
    assert!(snapshot
        .resolve(&Target {
            workspace: 2,
            panel: Some((false, 10))
        })
        .is_none());
    snapshot.workspaces[0].panels.as_mut().unwrap()[1].remove(0);
    assert!(snapshot.resolve(&target).is_none());
    snapshot.workspaces.remove(0);
    assert!(snapshot
        .resolve(&Target {
            workspace: 2,
            panel: None
        })
        .is_none());
}

#[test]
fn tray_panel_targets_are_invalid_when_split_mode_is_disabled() {
    let snapshot = Snapshot {
        workspaces: vec![Workspace {
            tab: Entry {
                id: 1,
                path: "C:\\Documents".into(),
                icon: LocationKind::Folder,
            },
            panels: None,
        }],
        ..Default::default()
    };
    assert!(snapshot
        .resolve(&Target {
            workspace: 1,
            panel: Some((false, 0))
        })
        .is_none());
    assert!(snapshot
        .resolve(&Target {
            workspace: 1,
            panel: None
        })
        .is_some());
}

#[test]
fn icon_metadata_uses_raw_locations_not_localized_labels() {
    use crate::domain::special_paths::{tag_view_path, COMPUTER_VIEW_ID, RECYCLE_BIN_VIEW_ID};
    assert_eq!(
        LocationKind::for_path(&tag_view_path(42)),
        LocationKind::Tag
    );
    assert_eq!(
        LocationKind::for_path(COMPUTER_VIEW_ID),
        LocationKind::Computer
    );
    assert_eq!(
        LocationKind::for_path(RECYCLE_BIN_VIEW_ID),
        LocationKind::RecycleBin
    );
    assert_eq!(
        LocationKind::for_path(r"C:\Documents"),
        LocationKind::Folder
    );
    let snapshot = Snapshot {
        workspaces: vec![Workspace {
            tab: Entry {
                id: 1,
                path: "Tag: Example".into(),
                icon: LocationKind::Tag,
            },
            panels: None,
        }],
        active_path: "Tag: Example".into(),
        active_icon: LocationKind::Tag,
        hidden_at: None,
    };
    let bytes = serde_json::to_vec(&Packet::Snapshot(snapshot.clone())).unwrap();
    let Packet::Snapshot(decoded) = serde_json::from_slice(&bytes).unwrap() else {
        panic!("unexpected packet")
    };
    assert_eq!(decoded, snapshot);
}

#[test]
fn older_snapshot_packets_default_to_folder_icons() {
    let bytes = br#"{"Snapshot":{"workspaces":[{"tab":{"id":1,"path":"Example"},"panels":null}],"active_path":"Example","hidden_at":null}}"#;
    let Packet::Snapshot(snapshot) = serde_json::from_slice(bytes).unwrap() else {
        panic!("unexpected packet")
    };
    assert_eq!(snapshot.active_icon, LocationKind::Folder);
    assert_eq!(snapshot.workspaces[0].tab.icon, LocationKind::Folder);
}
