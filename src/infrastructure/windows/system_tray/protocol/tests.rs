use super::super::{
    coordinator,
    model::{Snapshot, Target},
    tests::NativeWindow,
    TrayAction,
};
use super::*;
use std::sync::mpsc;
use std::time::Duration;
use windows::Win32::Foundation::LRESULT;
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, PostMessageW, MSG, WM_CLOSE,
};

const DONE: u32 = 0x8000 + 0x7e;

extern "system" fn receiver(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _: usize,
    ignore_payload: usize,
) -> LRESULT {
    if msg == WM_COPYDATA {
        if ignore_payload != 0 {
            return LRESULT(1);
        }
        if let Some(accepted) = unsafe { receive(HWND(wp.0 as *mut _), lp) } {
            return LRESULT(isize::from(accepted));
        }
    }
    if msg == coordinator::EXIT_MESSAGE {
        return LRESULT(isize::from(coordinator::request_exit(hwnd)));
    }
    unsafe { DefSubclassProc(hwnd, msg, wp, lp) }
}

struct Peer {
    hwnd: HWND,
    release: Option<mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<Option<TrayAction>>>,
}

impl Peer {
    fn spawn(busy_for: Duration, ignore_payload: bool) -> Self {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let native = NativeWindow::new();
            assert!(unsafe {
                SetWindowSubclass(native.0, Some(receiver), 89, usize::from(ignore_payload))
            }
            .as_bool());
            ready_tx.send(native.0 .0 as usize).unwrap();
            // Simulate a temporarily busy UI thread without touching the app's
            // real windows. Drop always releases this wait, even on test failure.
            let released = release_rx.recv_timeout(busy_for).is_ok();
            let mut msg = MSG::default();
            while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
                if msg.message == DONE || msg.message == WM_CLOSE {
                    break;
                }
                unsafe {
                    DispatchMessageW(&msg);
                }
            }
            // Keep the HWND/thread alive until the sender has observed the
            // acknowledgement (SMTO_ERRORONEXIT would otherwise race teardown).
            if !released {
                let _ = release_rx.recv();
            }
            super::super::take_action()
        });
        let hwnd = HWND(ready_rx.recv_timeout(Duration::from_secs(5)).unwrap() as *mut _);
        Self {
            hwnd,
            release: Some(release_tx),
            thread: Some(thread),
        }
    }

    fn stop(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
        unsafe {
            let _ = PostMessageW(Some(self.hwnd), DONE, WPARAM(0), LPARAM(0));
        }
    }

    fn finish(&mut self) -> Option<TrayAction> {
        self.stop();
        self.thread.take().unwrap().join().unwrap()
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            self.stop();
            let _ = thread.join();
        }
    }
}

#[test]
fn activation_accepts_a_peer_busy_longer_than_the_snapshot_timeout() {
    let sender = NativeWindow::new();
    let mut peer = Peer::spawn(Duration::from_millis(400), false);
    let target = Target {
        workspace: 1,
        panel: Some((true, 2)),
    };
    assert!(send(sender.0, peer.hwnd, &Packet::Activate(target.clone())));
    assert_eq!(peer.finish(), Some(TrayAction::Activate(target)));
}

#[test]
fn timed_out_send_returns_failure_without_marking_the_peer_exiting() {
    let sender = NativeWindow::new();
    // Do not dereference a same-process payload after the sender has timed out.
    let mut peer = Peer::spawn(Duration::from_secs(10), true);
    let target = super::super::window_ref::WindowRef::capture(peer.hwnd).unwrap();
    assert!(!send_with_timeout(
        sender.0,
        peer.hwnd,
        &Packet::Snapshot(Snapshot::default()),
        30
    ));
    assert!(target.is_current());
    assert_eq!(peer.finish(), None);
}

#[test]
fn remote_exit_is_acknowledged_only_after_the_receiver_accepts_close() {
    let sender = NativeWindow::new();
    let mut peer = Peer::spawn(Duration::ZERO, false);
    assert!(request_exit(sender.0, peer.hwnd));
    assert_eq!(peer.finish(), Some(TrayAction::Exit));
}

#[test]
fn activation_is_not_acknowledged_when_the_action_queue_rejects_it() {
    let sender = NativeWindow::new();
    while super::super::take_action().is_some() {}
    super::super::queue_action(TrayAction::Exit);
    let bytes = serde_json::to_vec(&Packet::Activate(Target {
        workspace: 1,
        panel: None,
    }))
    .unwrap();
    let data = COPYDATASTRUCT {
        dwData: MAGIC,
        cbData: bytes.len() as u32,
        lpData: bytes.as_ptr().cast_mut().cast(),
    };
    assert_eq!(
        unsafe { receive(sender.0, LPARAM(&data as *const _ as isize)) },
        Some(false)
    );
    assert_eq!(super::super::take_action(), Some(TrayAction::Exit));
    assert_eq!(super::super::take_action(), None);
}
