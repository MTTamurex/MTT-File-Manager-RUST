/// Trim the process working set after large one-shot indexing operations.
///
/// The in-memory index remains intact. This only asks Windows to remove cold
/// resident pages from the process working set, which is the number shown in
/// Task Manager's "Memory" column. Set `MTT_SEARCH_DISABLE_WS_TRIM=1` to disable.
pub(crate) fn trim_working_set(reason: &str) {
    if trim_disabled() {
        return;
    }

    unsafe {
        libmimalloc_sys::mi_collect(true);
    }

    #[cfg(target_os = "windows")]
    unsafe {
        use windows::Win32::System::Memory::{
            SetProcessWorkingSetSizeEx, SETPROCESSWORKINGSETSIZEEX_FLAGS,
        };
        use windows::Win32::System::Threading::GetCurrentProcess;

        let process = GetCurrentProcess();
        match SetProcessWorkingSetSizeEx(
            process,
            usize::MAX,
            usize::MAX,
            SETPROCESSWORKINGSETSIZEEX_FLAGS(0),
        ) {
            Ok(()) => eprintln!("[MEM] Trimmed working set after {}", reason),
            Err(error) => eprintln!("[MEM] Working set trim failed after {}: {}", reason, error),
        }
    }
}

pub(crate) fn trim_working_set_delayed(reason: String, delay: std::time::Duration) {
    if trim_disabled() {
        return;
    }

    let spawn_result = std::thread::Builder::new()
        .name("working-set-trim".to_string())
        .spawn(move || {
            std::thread::sleep(delay);
            trim_working_set(&reason);
        });

    if let Err(error) = spawn_result {
        eprintln!("[MEM] Failed to spawn delayed working set trim: {}", error);
    }
}

fn trim_disabled() -> bool {
    match std::env::var("MTT_SEARCH_DISABLE_WS_TRIM") {
        Ok(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}
