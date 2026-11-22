use super::SERVICE_NAME;
use std::ffi::OsString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_dispatcher;

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
                shutdown_clone.store(true, Ordering::Release);
                crate::startup_trace::event(None, "shutdown_requested", &[]);
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        });
    let status_handle = match status_handle {
        Ok(handle) => handle,
        Err(error) => {
            eprintln!("[SERVICE] Failed to register control handler: {error}");
            return;
        }
    };

    let _ = status_handle.set_service_status(status(ServiceState::Running, 0));
    let mut checkpoint = 0u32;
    crate::run_indexer(shutdown, || {
        checkpoint = checkpoint.saturating_add(1);
        let _ = status_handle.set_service_status(status(ServiceState::StopPending, checkpoint));
    });
    // run_indexer does not return while discovery/indexer threads are still saving.
    crate::startup_trace::event(None, "service_stopped", &[]);
    let _ = status_handle.set_service_status(status(ServiceState::Stopped, 0));
}

fn status(state: ServiceState, checkpoint: u32) -> ServiceStatus {
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: if state == ServiceState::Running {
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
        } else {
            ServiceControlAccept::empty()
        },
        exit_code: ServiceExitCode::Win32(0),
        checkpoint,
        wait_hint: if state == ServiceState::StopPending {
            Duration::from_secs(30)
        } else {
            Duration::ZERO
        },
        process_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_shutdown_reports_progress_without_claiming_to_be_stopped() {
        let pending = status(ServiceState::StopPending, 2);
        assert_eq!(pending.current_state, ServiceState::StopPending);
        assert_eq!(pending.checkpoint, 2);
        assert!(!pending.wait_hint.is_zero());
        assert!(pending.controls_accepted.is_empty());
        assert_eq!(status(ServiceState::Stopped, 0).checkpoint, 0);
    }
}
