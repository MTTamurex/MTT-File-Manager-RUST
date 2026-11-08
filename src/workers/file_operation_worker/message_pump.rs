//! Keep the worker's STA responsive to COM/OLE calls while waiting for work.

use super::FileOperationRequest;
use crossbeam_channel::{Receiver, RecvTimeoutError};
use std::time::Duration;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE, WM_QUIT,
};

const MAX_MESSAGES_PER_TURN: usize = 64;

pub(super) fn receive(receiver: &Receiver<FileOperationRequest>) -> Option<FileOperationRequest> {
    loop {
        let mut message = MSG::default();
        // A Shell window can keep posting messages. Bound each turn so queued
        // file requests and channel disconnection are still observed.
        for _ in 0..MAX_MESSAGES_PER_TURN {
            if !unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
                break;
            }
            if message.message == WM_QUIT {
                return None;
            }
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(request) => return Some(request),
            Err(RecvTimeoutError::Disconnected) => return None,
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_transfer_message_pump_exits_on_disconnected_channel() {
        let (sender, receiver) = crossbeam_channel::unbounded();
        drop(sender);
        assert!(receive(&receiver).is_none());
    }

    #[test]
    fn shell_transfer_message_pump_returns_work_without_executing_it() {
        let (sender, receiver) = crossbeam_channel::unbounded();
        sender
            .send(FileOperationRequest::CreateShortcuts {
                paths: vec![],
                dest_folder: std::path::PathBuf::new(),
            })
            .unwrap();
        assert!(matches!(
            receive(&receiver),
            Some(FileOperationRequest::CreateShortcuts { .. })
        ));
    }

    #[test]
    fn shell_transfer_message_pump_bounds_messages_before_checking_work() {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::System::Threading::GetCurrentThreadId;
        use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_APP};

        const TEST_MESSAGE: u32 = WM_APP + 0x321;
        let mut message = MSG::default();
        // Ensure this test thread has a Win32 message queue before posting.
        let _ = unsafe { PeekMessageW(&mut message, None, TEST_MESSAGE, TEST_MESSAGE, PM_REMOVE) };
        for _ in 0..=MAX_MESSAGES_PER_TURN {
            unsafe { PostThreadMessageW(GetCurrentThreadId(), TEST_MESSAGE, WPARAM(0), LPARAM(0)) }
                .unwrap();
        }
        let (sender, receiver) = crossbeam_channel::unbounded();
        sender
            .send(FileOperationRequest::CreateShortcuts {
                paths: vec![],
                dest_folder: std::path::PathBuf::new(),
            })
            .unwrap();
        assert!(receive(&receiver).is_some());
        let remaining =
            unsafe { PeekMessageW(&mut message, None, TEST_MESSAGE, TEST_MESSAGE, PM_REMOVE) };
        while unsafe { PeekMessageW(&mut message, None, TEST_MESSAGE, TEST_MESSAGE, PM_REMOVE) }
            .as_bool()
        {}
        assert!(
            remaining.as_bool(),
            "message draining must yield to queued work"
        );
    }
}
