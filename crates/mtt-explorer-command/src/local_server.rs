use std::sync::atomic::Ordering;

use windows::Win32::System::Com::{
    CoInitializeEx, CoRegisterClassObject, CoRevokeClassObject, CoUninitialize, IClassFactory,
    CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, REGCLS_MULTIPLEUSE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MsgWaitForMultipleObjects, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
    QS_ALLINPUT,
};

use super::folder_delegate::CLSID_MTT_FOLDER_DELEGATE;
use super::{
    current_tick_milliseconds, ClassFactory, ClassKind, COMMANDS_SERVED, LAST_COMMAND_ACTIVITY,
    LIVE_SERVER_COMMANDS, SERVER_LOCKS,
};

const IDLE_WAIT_MILLISECONDS: u32 = 1000;
const INITIAL_ACTIVATION_TIMEOUT_MILLISECONDS: u32 = 60000;
const SERVER_GRACE_MILLISECONDS: u32 = 5000;

fn server_can_shutdown_at(
    started_at: u32,
    now: u32,
    commands_served: u32,
    last_activity: u32,
    live_commands: u32,
    server_locks: u32,
) -> bool {
    if live_commands != 0 || server_locks != 0 {
        return false;
    }
    let (reference, timeout) = if commands_served == 0 {
        (started_at, INITIAL_ACTIVATION_TIMEOUT_MILLISECONDS)
    } else {
        (last_activity, SERVER_GRACE_MILLISECONDS)
    };
    now.wrapping_sub(reference) >= timeout
}

fn server_can_shutdown(started_at: u32) -> bool {
    server_can_shutdown_at(
        started_at,
        current_tick_milliseconds(),
        COMMANDS_SERVED.load(Ordering::SeqCst),
        LAST_COMMAND_ACTIVITY.load(Ordering::SeqCst),
        LIVE_SERVER_COMMANDS.load(Ordering::SeqCst),
        SERVER_LOCKS.load(Ordering::SeqCst),
    )
}

pub fn run_local_server() -> i32 {
    if unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_err() {
        return 1;
    }

    let factory: IClassFactory = ClassFactory::new(ClassKind::FolderDelegate).into();
    let registration = unsafe {
        CoRegisterClassObject(
            &CLSID_MTT_FOLDER_DELEGATE,
            &factory,
            CLSCTX_LOCAL_SERVER,
            REGCLS_MULTIPLEUSE,
        )
    };
    let Ok(registration) = registration else {
        unsafe { CoUninitialize() };
        return 1;
    };

    let started_at = current_tick_milliseconds();
    let mut message = MSG::default();
    loop {
        unsafe { MsgWaitForMultipleObjects(None, false, IDLE_WAIT_MILLISECONDS, QS_ALLINPUT) };
        let mut has_message =
            unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() };
        while has_message {
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            has_message = unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() };
        }
        if server_can_shutdown(started_at) {
            break;
        }
    }

    unsafe {
        let _ = CoRevokeClassObject(registration);
        CoUninitialize();
    }
    0
}

#[cfg(test)]
mod tests {
    use super::server_can_shutdown_at;

    #[test]
    fn initial_activation_has_a_full_startup_window() {
        assert!(!server_can_shutdown_at(10, 60_009, 0, 0, 0, 0));
        assert!(server_can_shutdown_at(10, 60_010, 0, 0, 0, 0));
    }

    #[test]
    fn active_com_objects_and_locks_keep_the_server_alive() {
        assert!(!server_can_shutdown_at(10, 70_000, 1, 10, 1, 0));
        assert!(!server_can_shutdown_at(10, 70_000, 1, 10, 0, 1));
    }

    #[test]
    fn idle_server_shuts_down_after_the_grace_period() {
        assert!(!server_can_shutdown_at(10, 5_099, 1, 100, 0, 0));
        assert!(server_can_shutdown_at(10, 5_100, 1, 100, 0, 0));
    }
}
