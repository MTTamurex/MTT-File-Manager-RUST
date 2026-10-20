#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[path = "mtt-updater/install.rs"]
mod install;
#[path = "../infrastructure/updater/platform.rs"]
mod platform;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{
    OpenProcess, WaitForSingleObject, INFINITE, PROCESS_SYNCHRONIZE,
};
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONWARNING, MB_OK};

fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.encode_wide().chain(Some(0)).collect()
}

fn warn(message: &str) {
    let message = wide(std::ffi::OsStr::new(message));
    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR(message.as_ptr()),
            w!("MTT Update"),
            MB_OK | MB_ICONWARNING,
        );
    }
}

fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 4 || platform::elevated() {
        return;
    }
    let Some(pid) = args[1].to_str().and_then(|s| s.parse::<u32>().ok()) else {
        return;
    };
    let target = std::path::PathBuf::from(&args[2]);
    if !target.is_absolute()
        || target
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .is_none_or(|name| !name.eq_ignore_ascii_case("mtt-file-manager.exe"))
    {
        return;
    }
    let Some(digest) = args[3]
        .to_str()
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
    else {
        return;
    };
    let Ok(helper) = std::env::current_exe() else {
        return;
    };
    let Some(directory) = helper.parent() else {
        return;
    };
    // Open the parent before it exits, avoiding PID reuse while waiting.
    let parent = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) };
    let Ok(parent) = parent else {
        return;
    };
    if std::fs::write(directory.join("ready"), b"ready").is_err() {
        unsafe {
            let _ = CloseHandle(parent);
        }
        return;
    }
    // MTT owns a cancellable guard until authorization. A slow parent exit is
    // not an installation failure; never start setup before the parent exits.
    let waited = unsafe { WaitForSingleObject(parent, INFINITE) };
    unsafe {
        let _ = CloseHandle(parent);
    }
    if waited != WAIT_OBJECT_0 {
        return;
    }
    if !directory.join("install").is_file() {
        let _ = std::fs::remove_file(directory.join("setup.exe"));
        let _ = std::fs::remove_file(directory.join("ready"));
        return;
    }
    let setup = directory.join("setup.exe");
    let can_restart = match install::run(&setup, digest) {
        Ok(()) => true,
        Err(error) => {
            warn(error.message());
            error.can_restart()
        }
    };
    // Do not trigger another automatic installation after failure/cancel/restart.
    if can_restart {
        let mut command = std::process::Command::new(&target);
        if let Some(parent) = target.parent() {
            command.current_dir(parent);
        }
        if command.env("MTT_SKIP_STARTUP_UPDATE", "1").spawn().is_err() {
            warn("Could not reopen MTT. Start the application manually.");
        }
        let _ = std::fs::remove_file(setup);
    }
    // Only the downloaded installer is removed; the running helper is left for temp cleanup.
    let _ = std::fs::remove_file(directory.join("ready"));
    let _ = std::fs::remove_file(directory.join("install"));
}
