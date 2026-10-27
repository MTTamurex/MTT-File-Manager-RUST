use super::for_shell_open;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[test]
fn files_use_their_containing_directory() {
    for path in [
        r"C:\Documents\config.ffs_gui",
        r"C:\Folder with spaces\file.txt",
        r"C:\Δοκιμή\文件.txt",
        r"C:\file.txt",
        r"\\server\share\folder\file.txt",
        r"\\?\C:\Documents\file.txt",
    ] {
        let path = Path::new(path);
        assert_eq!(for_shell_open(path), path.parent());
    }
}

#[test]
fn shortcuts_relative_paths_and_virtual_objects_keep_shell_defaults() {
    for path in [
        r"C:\Documents\app.lnk",
        r"C:\Documents\app.LNK",
        r"file.txt",
        r"folder\file.txt",
        r"C:file.txt",
        r"shell:AppsFolder\item",
        r"::{645FF040-5081-101B-9F08-00AA002F954E}\item",
        r"\\?\::{645FF040-5081-101B-9F08-00AA002F954E}\item",
    ] {
        assert_eq!(for_shell_open(Path::new(path)), None);
    }
}

#[test]
fn existing_directories_keep_shell_defaults() {
    let root = tempfile::tempdir().unwrap();
    assert_eq!(for_shell_open(root.path()), None);
    assert_eq!(for_shell_open(Path::new(r"C:\")), None);
}

fn create_probe(directory: &Path, marker: &str) -> PathBuf {
    std::fs::create_dir_all(directory).unwrap();
    let script = directory.join("probe.bat");
    // Only read a marker in the inherited CWD and write beside this disposable
    // script. No associations, user files or global environment are modified.
    std::fs::write(
        &script,
        format!(
            "@echo off\r\nset \"RESULT=unexpected\"\r\nif exist \".\\{marker}\" set \"RESULT=expected\"\r\n> \"%~dp0result.txt\" echo %RESULT%\r\n"
        ),
    )
    .unwrap();
    script
}

fn wait_for_probe(directory: &Path) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(result) = std::fs::read_to_string(directory.join("result.txt")) {
            if result.ends_with('\n') {
                assert_eq!(result.trim(), "expected", "child received the wrong CWD");
                return;
            }
        }
        assert!(Instant::now() < deadline, "Shell launch probe timed out");
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn shell_open_gives_each_script_its_own_directory_without_changing_process_cwd() {
    let _com =
        crate::infrastructure::windows::recycle_bin::ComApartmentGuard::init_sta_best_effort();
    let original_cwd = std::env::current_dir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("Folder with spaces");
    let second = root.path().join("Δοκιμή 文件");
    let first_script = create_probe(&first, "first.marker");
    let second_script = create_probe(&second, "second.marker");
    std::fs::write(first.join("first.marker"), b"").unwrap();
    std::fs::write(second.join("second.marker"), b"").unwrap();

    // Exercise the application's validation/canonicalization too, including
    // the verbatim paths it passes to the Windows Shell layer.
    crate::application::file_operations::open_with_shell(&first_script, None).unwrap();
    crate::application::file_operations::open_with_shell(&second_script, None).unwrap();

    wait_for_probe(&first);
    wait_for_probe(&second);
    assert_eq!(std::env::current_dir().unwrap(), original_cwd);
}

#[test]
fn shell_open_preserves_shortcut_start_in_directory() {
    use windows::core::{Interface, PCWSTR};
    use windows::Win32::System::Com::{CoCreateInstance, IPersistFile, CLSCTX_INPROC_SERVER};
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

    let _com =
        crate::infrastructure::windows::recycle_bin::ComApartmentGuard::init_sta_best_effort();
    let root = tempfile::tempdir().unwrap();
    let script_dir = root.path().join("script");
    let working_dir = root.path().join("working directory");
    std::fs::create_dir(&working_dir).unwrap();
    std::fs::write(working_dir.join("shortcut.marker"), b"").unwrap();
    let script = create_probe(&script_dir, "shortcut.marker");
    let shortcut = root.path().join("probe.lnk");
    let wide =
        |path: &Path| -> Vec<u16> { path.as_os_str().encode_wide().chain(Some(0)).collect() };
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).unwrap();
        link.SetPath(PCWSTR(wide(&script).as_ptr())).unwrap();
        link.SetWorkingDirectory(PCWSTR(wide(&working_dir).as_ptr()))
            .unwrap();
        let persist: IPersistFile = link.cast().unwrap();
        persist
            .Save(PCWSTR(wide(&shortcut).as_ptr()), false)
            .unwrap();
    }

    crate::infrastructure::windows::shell_operations::open_with_shell(&shortcut, None).unwrap();
    wait_for_probe(&script_dir);
}
