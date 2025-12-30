//! Windows system information functions
//! Follows .cursorrules: single responsibility, < 300 lines

use crate::infrastructure::windows_api::{
    Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
    Win32::System::Threading::GetCurrentProcess,
};

/// Gets the current process RAM usage (RSS/Working Set).
pub fn get_ram_usage() -> u64 {
    unsafe {
        let mut counters = PROCESS_MEMORY_COUNTERS::default();
        if K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ).as_bool() {
            counters.WorkingSetSize as u64
        } else {
            0
        }
    }
}
