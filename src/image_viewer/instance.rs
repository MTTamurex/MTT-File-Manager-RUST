//! Single image-viewer instance per graphics backend, acquired after renderer init.

use super::ipc;
use crate::viewer_runtime::backend::GraphicsBackend;
use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex};

pub(super) fn mutex_name(backend: GraphicsBackend) -> String {
    format!(
        "Local\\MTTFileManager_ImageViewer_SingleInstance_{}",
        backend.key()
    )
}

pub(super) struct SingleInstanceGuard {
    handle: HANDLE,
}

impl SingleInstanceGuard {
    fn try_acquire(backend: GraphicsBackend) -> std::io::Result<Option<Self>> {
        let wide: Vec<u16> = mutex_name(backend).encode_utf16().chain(Some(0)).collect();
        unsafe {
            let handle =
                CreateMutexW(None, true, PCWSTR(wide.as_ptr())).map_err(std::io::Error::other)?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(handle);
                Ok(None)
            } else {
                Ok(Some(Self { handle }))
            }
        }
    }
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.handle);
            let _ = CloseHandle(self.handle);
        }
    }
}

pub(super) fn start_or_forward(
    backend: GraphicsBackend,
    path: &Path,
) -> std::io::Result<Option<(SingleInstanceGuard, Receiver<PathBuf>)>> {
    if let Some(guard) = SingleInstanceGuard::try_acquire(backend)? {
        return Ok(Some((guard, ipc::start_open_request_server(backend))));
    }
    match ipc::send_open_request(path, backend).map_err(std::io::Error::other)? {
        true => Ok(None),
        false => Err(std::io::Error::other(
            "existing image viewer is unavailable",
        )),
    }
}

/// A duplicate process exits without ever revealing its startup window.
pub(super) struct ForwardedApp;

impl eframe::App for ForwardedApp {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_names_are_isolated_by_backend() {
        let backends = [
            GraphicsBackend::Dx12,
            GraphicsBackend::Vulkan,
            GraphicsBackend::Glow,
        ];
        for (i, backend) in backends.iter().copied().enumerate() {
            assert!(mutex_name(backend).ends_with(backend.key()));
            for other in &backends[i + 1..] {
                assert_ne!(mutex_name(backend), mutex_name(*other));
                assert_ne!(ipc::pipe_name(backend), ipc::pipe_name(*other));
            }
        }
    }
}
