//! Interactive, copy-only probe for the destination folder's Shell drop target.
//! This example is deliberately disconnected from the application's file workers.

#[path = "shell_transfer_probe/input.rs"]
mod input;
#[path = "shell_transfer_probe/observation.rs"]
mod observation;
#[path = "shell_transfer_probe/observed_data_object.rs"]
mod observed_data_object;
#[path = "shell_transfer_probe/transfer.rs"]
mod transfer;

use std::io::{self, Write};
use windows::core::w;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::{OleInitialize, OleUninitialize};
use windows::Win32::UI::WindowsAndMessaging::*;

struct OleGuard;

impl Drop for OleGuard {
    fn drop(&mut self) {
        unsafe { OleUninitialize() };
    }
}

struct ProbeWindow(HWND);

impl Drop for ProbeWindow {
    fn drop(&mut self) {
        unsafe {
            if IsWindow(Some(self.0)).as_bool() {
                let _ = DestroyWindow(self.0);
            }
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_DESTROY {
        unsafe { PostQuitMessage(0) };
        LRESULT(0)
    } else {
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }
}

fn create_window() -> Result<ProbeWindow, String> {
    unsafe {
        let instance = HINSTANCE(
            GetModuleHandleW(None)
                .map_err(|_| "GetModuleHandleW failed")?
                .0,
        );
        let class = w!("MttShellTransferProbe");
        let window_class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW).map_err(|_| "LoadCursorW failed")?,
            ..Default::default()
        };
        if RegisterClassW(&window_class) == 0 {
            return Err("RegisterClassW failed".into());
        }
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            w!("MTT Shell transfer probe - keep open until the transfer ends"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            760,
            180,
            None,
            None,
            Some(instance),
            None,
        )
        .map_err(|_| "CreateWindowExW failed")?;
        let window = ProbeWindow(hwnd);
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!("Copy-only Shell probe. See the console for diagnostic stages.\nKeep this window open until the external transfer has finished.\nThen close this window to exit. Closing it is NOT a transfer cancellation."),
            WS_CHILD | WS_VISIBLE,
            16,
            16,
            710,
            90,
            Some(hwnd),
            None,
            Some(instance),
            None,
        )
        .map_err(|_| "Could not create the diagnostic label")?;
        let _ = ShowWindow(hwnd, SW_SHOW);
        Ok(window)
    }
}

fn run() -> Result<(), String> {
    let Some(input) = input::parse(std::env::args_os().skip(1))? else {
        println!("Usage: cargo run --example shell_transfer_probe -- [--observe] <destination> <source> [source ...]");
        println!("Use existing absolute filesystem paths and disposable test data only.");
        println!("No clipboard changes, moves, deletions, CLI delegation, or automatic retries.");
        println!("--observe logs transfer feedback and offers async negotiation through a diagnostic IDataObject wrapper.");
        return Ok(());
    };
    let input = input.validate()?;
    if input.observe {
        println!("Observation mode: diagnostic IDataObject wrapper, async negotiation enabled.");
        println!("This mode may change Shell timing; compare it with the default unwrapped mode.");
    }
    println!("COPY-ONLY PROBE: {} source item(s).", input.sources.len());
    println!("This starts a REAL copy. The selected handler may offer to replace existing files.");
    println!("Use disposable data. Cancel any unexpected replacement prompt.");
    print!("Type COPY to continue, or press Enter to exit: ");
    io::stdout().flush().map_err(|_| "Could not flush stdout")?;
    let mut confirmation = String::new();
    io::stdin()
        .read_line(&mut confirmation)
        .map_err(|_| "Could not read confirmation")?;
    if confirmation.trim() != "COPY" {
        println!("Not started.");
        return Ok(());
    }

    unsafe { OleInitialize(None) }.map_err(|error| format!("OleInitialize: {}", error.code()))?;
    let _ole = OleGuard;
    let window = create_window()?;
    let session = transfer::CopySession::prepare(&input, window.0)?;
    let outcome = session.dispatch();
    match &outcome {
        Ok(effect) => println!("Drop returned: {effect:?}. This is NOT proof of copy completion."),
        Err(error) => eprintln!("{error}. No retry will be attempted."),
    }
    println!("Observe the chosen handler, cancellation, and destination contents manually.");
    println!("Keep the probe window open until the transfer ends; close it afterwards.");

    // Keep the STA, data object, destination and owner window alive and pump
    // messages even if a third-party handler delegates work asynchronously.
    let mut message = MSG::default();
    while unsafe { IsWindow(Some(window.0)) }.as_bool() {
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) }.0;
        if result == -1 {
            return Err("GetMessageW failed; external transfer outcome is unknown".into());
        }
        if result == 0 {
            break;
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    session.print_observation_summary();
    drop(session);
    outcome.map(|_| ())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Shell transfer probe: {error}");
        std::process::exit(1);
    }
}
