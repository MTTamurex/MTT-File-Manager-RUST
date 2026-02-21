use std::io;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinCommand {
    AlwaysKeepOnDevice,
    FreeUpSpace,
}

fn run_attrib(args: &[String], path: &Path) -> io::Result<()> {
    let output = Command::new("attrib").args(args).output()?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(io::Error::new(
        io::ErrorKind::Other,
        format!(
            "attrib failed for {:?} (args: {:?}): {}",
            path,
            args,
            stderr.trim()
        ),
    ))
}

pub fn set_pin_state(path: &Path, command: PinCommand) -> io::Result<()> {
    let path_str = path.to_string_lossy().to_string();

    // OneDrive pin-state can be controlled via NTFS Cloud Files attributes:
    // +P pinned, -P unpinned, +U unpinned placeholder, -U clear unpinned.
    let (set_flag, clear_flag) = match command {
        PinCommand::AlwaysKeepOnDevice => ("+P", "-U"),
        PinCommand::FreeUpSpace => ("+U", "-P"),
    };

    // 1) Apply to the selected path itself.
    let direct_args = vec![set_flag.to_string(), clear_flag.to_string(), path_str.clone()];
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

