//! Backend identity and per-child inheritance; never mutate the process environment.

use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

const BACKEND_ENV: &str = "MTT_VIEWER_BACKEND";
static ACTIVE_BACKEND: OnceLock<GraphicsBackend> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GraphicsBackend {
    Dx12,
    Vulkan,
    Glow,
}

impl GraphicsBackend {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "dx12" => Some(Self::Dx12),
            "vulkan" => Some(Self::Vulkan),
            "glow" => Some(Self::Glow),
            _ => None,
        }
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Dx12 => "dx12",
            Self::Vulkan => "vulkan",
            Self::Glow => "glow",
        }
    }

    pub(crate) fn from_context(cc: &eframe::CreationContext<'_>) -> Result<Self, String> {
        match cc
            .wgpu_render_state
            .as_ref()
            .map(|state| state.adapter.get_info().backend)
        {
            Some(eframe::wgpu::Backend::Dx12) => Ok(Self::Dx12),
            Some(eframe::wgpu::Backend::Vulkan) => Ok(Self::Vulkan),
            None => Ok(Self::Glow),
            Some(_) => Err("unsupported standalone-viewer graphics backend".into()),
        }
    }
}

/// Record the initialized backend, not the preference for the next restart.
pub(crate) fn register_context(cc: &eframe::CreationContext<'_>) {
    match GraphicsBackend::from_context(cc) {
        Ok(backend) => {
            let _ = ACTIVE_BACKEND.set(backend);
        }
        Err(error) => log::error!("[VIEWER-RENDERER] {error}"),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct BackendSelection {
    pub(super) candidates: Vec<GraphicsBackend>,
    pub(super) inherited: bool,
}

fn select(
    active: Option<GraphicsBackend>,
    inherited: Option<&str>,
    preference: Option<&str>,
) -> Result<BackendSelection, String> {
    if let Some(backend) = active {
        return Ok(BackendSelection {
            candidates: vec![backend],
            inherited: true,
        });
    }
    if let Some(value) = inherited {
        let backend = GraphicsBackend::parse(value)
            .ok_or_else(|| "invalid inherited viewer backend".to_string())?;
        return Ok(BackendSelection {
            candidates: vec![backend],
            inherited: true,
        });
    }
    use GraphicsBackend::*;
    let candidates = match preference {
        Some("glow" | "gl") => vec![Glow],
        Some("vulkan") => vec![Vulkan, Glow],
        _ => vec![Dx12, Vulkan, Glow],
    };
    Ok(BackendSelection {
        candidates,
        inherited: false,
    })
}

pub(super) fn selection() -> Result<BackendSelection, String> {
    if let Some(backend) = ACTIVE_BACKEND.get().copied() {
        return select(Some(backend), None, None);
    }
    if let Some(inherited) = std::env::var_os(BACKEND_ENV) {
        let inherited = inherited
            .to_str()
            .ok_or("invalid inherited viewer backend")?;
        return select(None, Some(inherited), None);
    }
    let preference = super::read_pref_readonly("gpu_backend");
    select(None, None, preference.as_deref())
}

pub(crate) fn requested_backend() -> Result<GraphicsBackend, String> {
    Ok(selection()?.candidates[0])
}

/// Every viewer launch goes through here so a saved preference cannot override
/// a backend selected by the manager's startup fallback.
pub(crate) fn viewer_command(executable: impl AsRef<Path>) -> std::io::Result<Command> {
    let backend = requested_backend().map_err(std::io::Error::other)?;
    Ok(command_with_backend(executable.as_ref(), backend))
}

fn command_with_backend(executable: &Path, backend: GraphicsBackend) -> Command {
    let mut command = Command::new(executable);
    command.env(BACKEND_ENV, backend.key());
    command
}

#[cfg(test)]
mod tests;
