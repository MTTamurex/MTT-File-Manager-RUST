use super::{folder_command, legacy_folder_command};
use std::path::{Path, PathBuf};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::UI::Shell::CommandLineToArgvW;

fn parse_command(command: &str) -> Vec<String> {
    let wide: Vec<u16> = command.encode_utf16().chain(Some(0)).collect();
    let mut count = 0;
    unsafe {
        let argv = CommandLineToArgvW(PCWSTR(wide.as_ptr()), &mut count);
        assert!(!argv.is_null());
        let args = std::slice::from_raw_parts(argv, count as usize)
            .iter()
            .map(|arg| arg.to_string().unwrap())
            .collect();
        let _ = LocalFree(Some(HLOCAL(argv.cast())));
        args
    }
}

#[test]
fn shell_folder_commands_preserve_roots_spaces_unicode_and_trailing_separators() {
    let executable = Path::new(r"C:\Program Files\Example\mtt-file-manager.exe");
    for target in [
        r"C:\",
        r"D:\",
        r"C:\Example Folder",
        r"C:\Example Folder\",
        r"C:\Pasta com acentuação\",
        r"\\server\share",
        r"\\server\share\",
        r"\\server\share\Example Folder\",
    ] {
        let command = folder_command(executable).replace("%1", target);
        let args = parse_command(&command);
        assert_eq!(args.len(), 3, "{target}");
        assert_eq!(args[0], executable.to_string_lossy());
        assert_eq!(args[1], "--open-folder");
        // This is the lexical normalization performed by the startup receiver.
        let received: PathBuf = Path::new(&args[2]).components().collect();
        let expected: PathBuf = Path::new(target).components().collect();
        assert_eq!(received, expected, "{target}");
    }
}

#[test]
fn old_shell_command_reproduces_the_invalid_drive_root_argument() {
    let executable = Path::new(r"C:\Example\mtt-file-manager.exe");
    let args = parse_command(&legacy_folder_command(executable).replace("%1", r"C:\"));
    assert_eq!(args[2], "C:\"");
}
