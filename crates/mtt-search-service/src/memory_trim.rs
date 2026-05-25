/// Trim the process working set after large one-shot indexing operations.
///
/// The in-memory index remains intact. This only asks Windows to remove cold
/// resident pages from the process working set, which is the number shown in
/// Task Manager's "Memory" column. Set `MTT_SEARCH_DISABLE_WS_TRIM=1` to disable.
pub(crate) fn trim_working_set(reason: &str) {
    if trim_disabled() {
        return;
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

fn trim_disabled() -> bool {
    match std::env::var("MTT_SEARCH_DISABLE_WS_TRIM") {
        Ok(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}
