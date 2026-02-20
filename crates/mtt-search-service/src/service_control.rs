use std::ffi::OsString;
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

    let exe_path = std::env::current_exe().expect("Cannot get executable path");
    eprintln!("[SERVICE] Executable path: {}", exe_path.display());

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
