//! Launch Command Prompt without interpreting the target folder as command text.
use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows::Win32::System::Threading::CREATE_NEW_CONSOLE;
use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SHELLEXECUTEINFOW};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

pub const HELPER_FLAG: &str = "--open-cmd-helper";
const DIRECTORY_COMMAND: &str = "/d /e:on /v:off /k pushd \"%MTT_CMD_DIRECTORY%\"";

fn system_directory() -> io::Result<PathBuf> {
    let mut buffer = vec![0u16; 32_768];
    let len = unsafe { GetSystemDirectoryW(Some(&mut buffer)) } as usize;
    if len == 0 {
        return Err(io::Error::last_os_error());
    }
    if len >= buffer.len() {
        return Err(io::Error::other("System directory exceeds buffer"));
    }
    Ok(PathBuf::from(OsString::from_wide(&buffer[..len])))
}

fn validate_directory(path: &Path) -> io::Result<()> {
    if !path.is_absolute()
        || path
            .as_os_str()
            .encode_wide()
            .any(|unit| unit < 32 || unit == b'"' as u16)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid Command Prompt directory",
        ));
    }
    Ok(())
}

fn is_network(path: &Path) -> bool {
    matches!(path.components().next(), Some(std::path::Component::Prefix(prefix))
        if matches!(prefix.kind(), std::path::Prefix::UNC(..) | std::path::Prefix::VerbatimUNC(..)))
}

fn helper_parameters(path: &Path) -> io::Result<Vec<u16>> {
    validate_directory(path)?;
    // The target is a Windows argv argument, not CMD syntax. Quotes are rejected
    // above; trailing backslashes must be doubled before the closing argv quote.
    let mut parameters: Vec<u16> = format!("{HELPER_FLAG} \"").encode_utf16().collect();
    let units: Vec<_> = path.as_os_str().encode_wide().collect();
    let trailing_slashes = units
        .iter()
        .rev()
        .take_while(|unit| **unit == b'\\' as u16)
        .count();
    parameters.extend(units);
    parameters.extend(std::iter::repeat_n(b'\\' as u16, trailing_slashes));
    parameters.extend([b'"' as u16, 0]);
    Ok(parameters)
}

fn elevated(path: &Path, system: &Path, hwnd: Option<HWND>) -> io::Result<()> {
    let executable = std::env::current_exe()?;
    let program: Vec<u16> = executable
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let args = helper_parameters(path)?;
    // Start the helper from a trusted directory, never the user-selected folder.
    let directory: Vec<u16> = system.as_os_str().encode_wide().chain(Some(0)).collect();
    let verb: Vec<u16> = "runas\0".encode_utf16().collect();
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_FLAG_NO_UI,
        hwnd: hwnd.unwrap_or_default(),
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(program.as_ptr()),
        lpParameters: PCWSTR(args.as_ptr()),
        lpDirectory: PCWSTR(directory.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    match unsafe { ShellExecuteExW(&mut info) } {
        Ok(()) => Ok(()),
        Err(error) if error.code().0 as u32 == 0x800704c7 => Ok(()), // UAC cancelled
        Err(error) => Err(io::Error::other(error.to_string())),
    }
}

pub fn open(path: &Path, admin: bool, hwnd: Option<HWND>) -> io::Result<()> {
    validate_directory(path)?;
    let system = system_directory()?;
    let cmd = system.join("cmd.exe");
    if admin {
        elevated(path, &system, hwnd)
    } else {
        let mut command = Command::new(cmd);
        command.creation_flags(CREATE_NEW_CONSOLE.0);
        if is_network(path) {
            // Environment expansion happens once, inside quotes, with delayed expansion off.
            command
                .env("MTT_CMD_DIRECTORY", path)
                .raw_arg(DIRECTORY_COMMAND)
                .current_dir(&system);
        } else {
            command.args(["/d", "/v:off", "/k"]).current_dir(path);
        }
        command.spawn().map(|_| ())
    }
}

fn helper_target(args: impl IntoIterator<Item = OsString>) -> io::Result<PathBuf> {
    let mut args = args.into_iter();
    let path = args.next().map(PathBuf::from).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Missing Command Prompt directory",
        )
    })?;
    if args.next().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Unexpected helper arguments",
        ));
    }
    validate_directory(&path)?;
    Ok(path)
}

fn helper_command(path: &Path, system: &Path) -> Command {
    let mut command = Command::new(system.join("cmd.exe"));
    // Set this environment variable only on the child process, not globally.
    // pushd runs after CMD startup, so elevation cannot reset the final directory.
    command
        .creation_flags(CREATE_NEW_CONSOLE.0)
        .env("MTT_CMD_DIRECTORY", path)
        .raw_arg(DIRECTORY_COMMAND)
        .current_dir(system);
    command
}

/// Dedicated startup mode: launches only CMD, without GUI or preference initialization.
/// Elevation is requested by `open`; this entry point cannot elevate itself.
pub fn run_helper(args: impl IntoIterator<Item = OsString>) -> i32 {
    let result = (|| -> io::Result<()> {
        let path = helper_target(args)?;
        let system = system_directory()?;
        helper_command(&path, &system).spawn().map(|_| ())
    })();
    match result {
        Ok(()) => 0,
        Err(error) => error.raw_os_error().unwrap_or(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_absolute_paths_without_rejecting_shell_metacharacters() {
        for path in [
            r"C:\",
            r"D:\Folder with spaces",
            r"D:\A&B (test) %PATH% !value! ^",
            r"D:\例",
            r"\\server\share\Folder",
        ] {
            assert!(validate_directory(Path::new(path)).is_ok());
        }
        for path in [
            "relative",
            "C:relative",
            "C:\\bad\0path",
            "C:\\bad\"path",
            "C:\\bad\npath",
        ] {
            assert!(validate_directory(Path::new(path)).is_err());
        }
    }

    #[test]
    fn recognizes_network_paths_without_misclassifying_extended_local_paths() {
        assert!(is_network(Path::new(r"\\server\share\Folder")));
        assert!(is_network(Path::new(r"\\?\UNC\server\share\Folder")));
        assert!(!is_network(Path::new(r"\\?\C:\Folder")));
        assert!(!is_network(Path::new(r"C:\Folder")));
    }

    #[test]
    #[ignore = "launches CMD; run manually after reviewing local antivirus restrictions"]
    fn cmd_expands_the_directory_environment_value_only_once() {
        use windows::Win32::System::Threading::CREATE_NO_WINDOW;
        let temp = tempfile::tempdir().unwrap();
        let name = "Folder & (test) %PATH% !value! ^ 例";
        let directory = temp.path().join(name);
        std::fs::create_dir(&directory).unwrap();
        let system = system_directory().unwrap();
        let output = Command::new(system.join("cmd.exe"))
            .creation_flags(CREATE_NO_WINDOW.0)
            .env("MTT_CMD_DIRECTORY", &directory)
            .raw_arg("/d /e:on /v:off /u /c pushd \"%MTT_CMD_DIRECTORY%\" && cd")
            .current_dir(system)
            .output()
            .unwrap();
        assert!(output.status.success());
        let units: Vec<_> = output
            .stdout
            .as_chunks::<2>()
            .0
            .iter()
            .map(|bytes| u16::from_le_bytes(*bytes))
            .collect();
        let printed = String::from_utf16(&units).unwrap();
        assert!(printed.trim_end().ends_with(name));
    }

    #[test]
    fn helper_accepts_exactly_one_absolute_directory() {
        for path in [r"C:\", r"D:\Folder with spaces", r"\\server\share\Folder"] {
            assert_eq!(helper_target([path.into()]).unwrap(), Path::new(path));
        }
        assert!(helper_target(Vec::<OsString>::new()).is_err());
        assert!(helper_target(["relative".into()]).is_err());
        assert!(helper_target([r"C:\Folder".into(), "/c".into()]).is_err());
    }

    #[test]
    fn helper_argument_quotes_drive_roots_and_trailing_slashes() {
        for (path, expected) in [
            (r"C:\", format!("{HELPER_FLAG} \"C:\\\\\"\0")),
            (
                r"D:\Folder with spaces\",
                format!("{HELPER_FLAG} \"D:\\Folder with spaces\\\\\"\0"),
            ),
        ] {
            assert_eq!(
                helper_parameters(Path::new(path)).unwrap(),
                expected.encode_utf16().collect::<Vec<_>>()
            );
        }
        assert!(helper_parameters(Path::new("C:\\bad\"path")).is_err());
    }

    #[test]
    fn helper_passes_directory_as_child_environment_data_and_uses_fixed_cmd_command() {
        let system = Path::new(r"C:\System");
        for path in [
            r"D:\Folder & (test) %PATH% !value! ^ 例",
            r"\\server\share\Folder",
        ] {
            let path = Path::new(path);
            let command = helper_command(path, system);
            assert_eq!(command.get_program(), system.join("cmd.exe"));
            assert_eq!(command.get_current_dir(), Some(system));
            assert!(command
                .get_envs()
                .any(|(key, value)| key == "MTT_CMD_DIRECTORY" && value == Some(path.as_os_str())));
            assert_eq!(
                command.get_args().collect::<Vec<_>>(),
                [std::ffi::OsStr::new(DIRECTORY_COMMAND)]
            );
        }
    }

    #[test]
    fn helper_directory_roundtrips_through_windows_argument_parser() {
        use std::os::windows::ffi::OsStringExt;
        use windows::Win32::Foundation::{LocalFree, HLOCAL};
        use windows::Win32::UI::Shell::CommandLineToArgvW;

        for path in [
            r"C:\",
            r"D:\Folder with spaces\",
            r"D:\Folder & %PATH% !value! 例",
            r"\\server\share\Folder\",
        ] {
            let mut command_line: Vec<u16> = "helper.exe ".encode_utf16().collect();
            command_line.extend(helper_parameters(Path::new(path)).unwrap());
            let mut count = 0;
            let argv = unsafe { CommandLineToArgvW(PCWSTR(command_line.as_ptr()), &mut count) };
            assert!(!argv.is_null());
            let arguments: Vec<_> = (0..count as usize)
                .map(|index| unsafe { OsString::from_wide((*argv.add(index)).as_wide()) })
                .collect();
            unsafe {
                let _ = LocalFree(Some(HLOCAL(argv.cast())));
            }
            assert_eq!(arguments.len(), 3);
            assert_eq!(arguments[1], HELPER_FLAG);
            assert_eq!(
                helper_target(arguments.into_iter().skip(2)).unwrap(),
                Path::new(path)
            );
        }
    }
}
