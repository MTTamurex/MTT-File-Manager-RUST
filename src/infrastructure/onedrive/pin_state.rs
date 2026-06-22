use std::io;
use std::path::Path;
use std::process::Command;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinCommand {
    AlwaysKeepOnDevice,
    FreeUpSpace,
}

fn run_attrib(args: &[String], path: &Path) -> io::Result<()> {
    // SEC: Resolve attrib.exe via the GetSystemDirectoryW Win32 API rather
    // than trusting the SYSTEMROOT environment variable, which can be
    // overridden in the process environment by a parent process.
    // GetSystemDirectoryW returns the kernel-tracked system directory.
    let attrib_exe = system32_path("attrib.exe");
    let mut cmd = Command::new(attrib_exe);
    cmd.args(args);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let output = cmd.output()?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(io::Error::other(format!(
        "attrib failed for {:?} (args: {:?}): {}",
        path,
        args,
        stderr.trim()
    )))
}

pub fn set_pin_state(path: &Path, command: PinCommand) -> io::Result<()> {
    let path_str = path.to_string_lossy().to_string();

    // Cloud Files pin-state can be controlled via NTFS Cloud Files attributes:
    // +P pinned, -P unpinned, +U unpinned placeholder, -U clear unpinned.
    let (set_flag, clear_flag) = match command {
        PinCommand::AlwaysKeepOnDevice => ("+P", "-U"),
        PinCommand::FreeUpSpace => ("+U", "-P"),
    };

    // 1) Apply to the selected path itself.
    let direct_args = vec![
        set_flag.to_string(),
        clear_flag.to_string(),
        path_str.clone(),
    ];
    run_attrib(&direct_args, path)?;

    // 2) If it's a folder, apply recursively to children too (Explorer-like behavior).
    // Use "<folder>\*" with /S /D so files and subdirectories are covered.
    if super::fast_is_dir(path) {
        let mut pattern = path.to_path_buf();
        pattern.push("*");
        let recursive_path = pattern.to_string_lossy().to_string();
        let recursive_args = vec![
            set_flag.to_string(),
            clear_flag.to_string(),
            "/S".to_string(),
            "/D".to_string(),
            recursive_path,
        ];
        run_attrib(&recursive_args, path)?;
    }

    Ok(())
}

/// SEC: Resolves the absolute path of an executable inside the Windows system
/// directory using the kernel-tracked location, ignoring the (mutable)
/// SYSTEMROOT environment variable.
fn system32_path(exe: &str) -> std::path::PathBuf {
    use windows::Win32::System::SystemInformation::GetSystemDirectoryW;

    let mut buf = [0u16; 260];
    let len = unsafe { GetSystemDirectoryW(Some(&mut buf)) } as usize;
    let dir = if len == 0 || len > buf.len() {
        // Fallback: hardcoded — only reached if GetSystemDirectoryW fails,
        // which should not happen in normal Windows installs.
        std::path::PathBuf::from(r"C:\Windows\System32")
    } else {
        std::path::PathBuf::from(String::from_utf16_lossy(&buf[..len]))
    };
    dir.join(exe)
}
