use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use windows_service::service::{
    ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode,
    ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_dispatcher;
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

const SERVICE_NAME: &str = "MTTFileManagerSearch";
const SERVICE_DISPLAY_NAME: &str = "MTT File Manager Search Indexer";

/// Install the service into the Windows Service Control Manager.
pub fn install_service() {
    let manager =
        ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CREATE_SERVICE);

    let manager = match manager {
        Ok(m) => m,
        Err(e) => {
            eprintln!("[SERVICE] Failed to open Service Manager: {}", e);
            eprintln!("[SERVICE] Are you running as Administrator?");
            return;
        }
    };

    let exe_path = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("[SERVICE] Cannot get executable path: {}", error);
            return;
        }
    };
    eprintln!("[SERVICE] Executable path: {}", exe_path.display());
    if let Err(error) = validate_service_install_path(&exe_path) {
        eprintln!(
            "[SERVICE] Refusing to install insecure service path: {}",
            error
        );
        eprintln!(
            "[SERVICE] Move the service executable under the system Program Files directory, or set \
             MTT_SEARCH_ALLOW_UNSAFE_SERVICE_INSTALL=1 only for an intentional admin/dev install."
        );
        return;
    }
    if let Err(error) = crate::index_db::reset_storage_for_install() {
        eprintln!(
            "[SERVICE] Refusing to install without securely resetting the index cache: {}",
            error
        );
        return;
    }

    let service_info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(SERVICE_DISPLAY_NAME),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe_path.clone(),
        launch_arguments: vec![],
        dependencies: vec![],
        // USN journal indexing requires elevated privileges; keep LocalSystem runtime.
        account_name: None,
        account_password: None,
    };

    match manager.create_service(&service_info, ServiceAccess::CHANGE_CONFIG) {
        Ok(_) => {
            eprintln!("[SERVICE] '{}' installed successfully.", SERVICE_NAME);
            eprintln!("[SERVICE] Start it with: sc.exe start {}", SERVICE_NAME);
        }
        Err(e) => {
            eprintln!("[SERVICE] Failed to install service: {:?}", e);
            eprintln!("[SERVICE] Hint: try manually with:");
            eprintln!(
                "  sc create {} binPath= \"{}\" start= auto",
                SERVICE_NAME,
                exe_path.display()
            );
        }
    }
}

fn validate_service_install_path(exe_path: &Path) -> Result<(), String> {
    if env_flag_enabled("MTT_SEARCH_ALLOW_UNSAFE_SERVICE_INSTALL") {
        eprintln!(
            "[SERVICE] WARNING: MTT_SEARCH_ALLOW_UNSAFE_SERVICE_INSTALL=1; path hardening bypassed"
        );
        return Ok(());
    }

    let normalized = normalize_path_for_policy(exe_path);
    if normalized.contains("\\target\\") || normalized.ends_with("\\target") {
        return Err(format!(
            "service executable is under a build target directory: {}",
            exe_path.display()
        ));
    }

    for variable in ["USERPROFILE", "TEMP", "TMP"] {
        if path_starts_with_env_path(&normalized, variable) {
            return Err(format!(
                "service executable is under %{}%: {}",
                variable,
                exe_path.display()
            ));
        }
    }

    validate_windows_path_security(exe_path)?;

    Ok(())
}

fn validate_windows_path_security(exe_path: &Path) -> Result<(), String> {
    let program_files = trusted_program_files_path()?;
    let canonical_exe = std::fs::canonicalize(exe_path)
        .map_err(|error| format!("cannot canonicalize {}: {}", exe_path.display(), error))?;
    let canonical_program_files = std::fs::canonicalize(&program_files).map_err(|error| {
        format!(
            "cannot canonicalize trusted Program Files path {}: {}",
            program_files.display(),
            error
        )
    })?;

    let expected_install_dir = canonical_program_files.join("MTT File Manager");
    let actual_install_dir = canonical_exe
        .parent()
        .ok_or_else(|| "service executable has no parent directory".to_string())?;
    if !paths_equivalent_case_insensitive(actual_install_dir, &expected_install_dir) {
        return Err(format!(
            "service executable is outside the trusted Program Files product directory: {}",
            exe_path.display()
        ));
    }

    let mut component = Some(exe_path);
    while let Some(path) = component {
        validate_not_reparse_point(path)?;
        if paths_equivalent_case_insensitive(path, &program_files) {
            break;
        }
        component = path.parent();
    }

    Ok(())
}

fn trusted_program_files_path() -> Result<PathBuf, String> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{
        FOLDERID_ProgramFiles, SHGetKnownFolderPath, KF_FLAG_DONT_VERIFY,
    };

    let raw = unsafe {
        SHGetKnownFolderPath(&FOLDERID_ProgramFiles, KF_FLAG_DONT_VERIFY, None)
            .map_err(|error| format!("SHGetKnownFolderPath(ProgramFiles) failed: {}", error))?
    };
    let path = PathBuf::from(OsString::from_wide(unsafe { raw.as_wide() }));
    unsafe {
        CoTaskMemFree(Some(raw.0.cast()));
    }
    Ok(path)
}

#[cfg(test)]
fn path_is_same_or_descendant(path: &Path, root: &Path) -> bool {
    let path = normalize_path_text(path);
    let root = normalize_path_text(root);
    path == root || path.starts_with(&format!("{}\\", root.trim_end_matches('\\')))
}

#[cfg(test)]
fn normalize_path_text(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

fn paths_equivalent_case_insensitive(left: &Path, right: &Path) -> bool {
    normalize_path_for_policy(left) == normalize_path_for_policy(right)
}

fn validate_not_reparse_point(path: &Path) -> Result<(), String> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        OPEN_EXISTING,
    };

    let path_wide: Vec<u16> = OsStr::new(path.as_os_str())
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let handle = unsafe {
        CreateFileW(
            PCWSTR(path_wide.as_ptr()),
            0x0080, // FILE_READ_ATTRIBUTES
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(
                FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0,
            ),
            None,
        )
    }
    .map_err(|error| format!("cannot securely open {}: {}", path.display(), error))?;

    struct OwnedHandle(HANDLE);
    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
    let handle = OwnedHandle(handle);

    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(handle.0, &mut information) }
        .map_err(|error| format!("cannot inspect {}: {}", path.display(), error))?;
    if information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
        return Err(format!(
            "path component is a reparse point: {}",
            path.display()
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_containment_is_case_insensitive_and_separator_aware() {
        let root = Path::new(r"C:\Program Files");
        assert!(path_is_same_or_descendant(
            Path::new(r"c:\PROGRAM FILES\MTT File Manager\service.exe"),
            root
        ));
        assert!(!path_is_same_or_descendant(
            Path::new(r"C:\Program Files Malicious\service.exe"),
            root
        ));
    }

    #[test]
    fn trusted_program_files_known_folder_resolves() {
        let path = trusted_program_files_path().expect("Program Files Known Folder should resolve");
        assert!(path.is_absolute());
        assert!(path.is_dir());
    }

    #[test]
    fn current_test_binary_is_rejected_outside_program_files() {
        if env_flag_enabled("MTT_SEARCH_ALLOW_UNSAFE_SERVICE_INSTALL") {
            return;
        }
        let current = std::env::current_exe().expect("current test executable path");
        let error = validate_service_install_path(&current).unwrap_err();
        assert!(error.contains("target directory") || error.contains("outside the trusted"));
    }

    #[test]
    fn reparse_point_is_rejected_when_symlink_creation_is_available() {
        use std::os::windows::fs::symlink_file;

        let directory = std::env::temp_dir().join(format!(
            "mtt-service-path-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        std::fs::create_dir(&directory).expect("temporary directory");
        let target = directory.join("target.exe");
        let link = directory.join("service.exe");
        std::fs::write(&target, b"test").expect("create symlink target");
        if symlink_file(&target, &link).is_err() {
            let _ = std::fs::remove_file(&target);
            let _ = std::fs::remove_dir(&directory);
            return;
        }

        let error = validate_not_reparse_point(&link).unwrap_err();
        let _ = std::fs::remove_file(&link);
        let _ = std::fs::remove_file(&target);
        let _ = std::fs::remove_dir(&directory);
        assert!(error.contains("reparse point"));
    }
}

fn path_starts_with_env_path(path: &str, variable: &str) -> bool {
    let Ok(value) = std::env::var(variable) else {
        return false;
    };
    if value.trim().is_empty() {
        return false;
    }
    let env_path = normalize_path_for_policy(Path::new(&value));
    path == env_path || path.starts_with(&format!("{}\\", env_path.trim_end_matches('\\')))
}

fn normalize_path_for_policy(path: &Path) -> String {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    canonical
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase()
}

fn env_flag_enabled(name: &str) -> bool {
    match std::env::var(name) {
        Ok(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}

/// Uninstall the service from the Windows Service Control Manager.
pub fn uninstall_service() {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT);

    let manager = match manager {
        Ok(m) => m,
        Err(e) => {
            eprintln!("[SERVICE] Failed to open Service Manager: {}", e);
            eprintln!("[SERVICE] Are you running as Administrator?");
            return;
        }
    };

    let service = match manager.open_service(
        SERVICE_NAME,
        ServiceAccess::STOP | ServiceAccess::DELETE | ServiceAccess::QUERY_STATUS,
    ) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[SERVICE] Failed to open service '{}': {}", SERVICE_NAME, e);
            return;
        }
    };

    // Try to stop the service
    let _ = service.stop();
    eprintln!("[SERVICE] Stopping service...");
    std::thread::sleep(Duration::from_secs(2));

    match service.delete() {
        Ok(_) => eprintln!("[SERVICE] '{}' uninstalled successfully.", SERVICE_NAME),
        Err(e) => eprintln!("[SERVICE] Failed to delete service: {}", e),
    }
}

/// Run as a Windows Service (called by SCM dispatcher).
pub fn run_as_service() -> Result<(), String> {
    service_dispatcher::start(SERVICE_NAME, service_main)
        .map_err(|e| format!("Service dispatcher error: {}", e))
}

windows_service::define_windows_service!(service_main, handle_service_main);

fn handle_service_main(_args: Vec<OsString>) {
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = shutdown.clone();

    let status_handle =
        service_control_handler::register(SERVICE_NAME, move |control| match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                shutdown_clone.store(true, Ordering::Relaxed);
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        });

    let status_handle = match status_handle {
        Ok(h) => h,
        Err(e) => {
            eprintln!("[SERVICE] Failed to register control handler: {}", e);
            return;
        }
    };

    // Report "Running"
    let _ = status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    });

    // Run the indexer (blocks until shutdown)
    crate::run_indexer(shutdown);

    // Report "Stopped"
    let _ = status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    });
}
