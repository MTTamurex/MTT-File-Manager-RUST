use windows::Win32::Foundation::{CloseHandle, ERROR_NO_MORE_FILES};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};

pub(crate) fn elevated() -> bool {
    unsafe { windows::Win32::UI::Shell::IsUserAnAdmin().as_bool() }
}

/// Includes viewer subprocesses: they can also hold runtime DLLs open.
pub(crate) fn other_instances(excluded: u32) -> Result<bool, String> {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
            .map_err(|_| "Could not inspect running applications".to_string())?;
        let result = (|| {
            let mut entry = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            Process32FirstW(snapshot, &mut entry)
                .map_err(|_| "Could not inspect running applications".to_string())?;
            loop {
                let length = entry
                    .szExeFile
                    .iter()
                    .position(|c| *c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..length]);
                if entry.th32ProcessID != excluded
                    && name.eq_ignore_ascii_case("mtt-file-manager.exe")
                {
                    return Ok(true);
                }
                if let Err(error) = Process32NextW(snapshot, &mut entry) {
                    return if error.code()
                        == windows::core::HRESULT::from_win32(ERROR_NO_MORE_FILES.0)
                    {
                        Ok(false)
                    } else {
                        Err("Could not inspect running applications".into())
                    };
                }
            }
        })();
        let _ = CloseHandle(snapshot);
        result
    }
}
