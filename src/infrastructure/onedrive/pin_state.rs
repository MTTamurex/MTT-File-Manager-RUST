use std::io;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinCommand {
    AlwaysKeepOnDevice,
    FreeUpSpace,
}

pub fn set_pin_state(path: &Path, command: PinCommand) -> io::Result<()> {
    let path_str = path.to_string_lossy().to_string();

    // OneDrive pin-state can be controlled via NTFS Cloud Files attributes:
    // +P pinned, -P unpinned, +U unpinned placeholder, -U clear unpinned.
    let args: [&str; 3] = match command {
        PinCommand::AlwaysKeepOnDevice => ["+P", "-U", &path_str],
        PinCommand::FreeUpSpace => ["+U", "-P", &path_str],
    };

    let output = Command::new("attrib").args(args).output()?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(io::Error::new(
        io::ErrorKind::Other,
        format!("attrib failed for {:?}: {}", path, stderr.trim()),
    ))
}

