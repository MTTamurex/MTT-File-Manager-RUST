//! Shared standalone-viewer renderer configuration.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use super::backend::{self, BackendSelection, GraphicsBackend};

/// Child windows inherit one exact backend. Only independent launches may fall
/// back, and only before application creation has started.
pub fn run_viewer_native(
    app_name: &str,
    options: eframe::NativeOptions,
    app_creator: eframe::AppCreator<'_>,
) -> eframe::Result<()> {
    let selection = backend::selection()
        .map_err(|error| eframe::Error::AppCreation(std::io::Error::other(error).into()))?;
    run_with(
        app_name,
        options,
        app_creator,
        selection,
        |name, options, creator| eframe::run_native(name, options, creator),
    )
}

fn run_with<'app>(
    app_name: &str,
    mut options: eframe::NativeOptions,
    app_creator: eframe::AppCreator<'app>,
    selection: BackendSelection,
    mut run: impl FnMut(&str, eframe::NativeOptions, eframe::AppCreator<'app>) -> eframe::Result<()>,
) -> eframe::Result<()> {
    options.run_and_return = true;
    let creator = Rc::new(RefCell::new(Some(app_creator)));
    for (index, requested) in selection.candidates.iter().copied().enumerate() {
        log::info!(
            "[VIEWER-RENDERER] requested={} inherited={}",
            requested.key(),
            selection.inherited
        );
        let pending = Rc::clone(&creator);
        let attempt = || {
            run(
                app_name,
                options_for_backend(options.clone(), requested),
                Box::new(move |cc| {
                    let create = pending
                        .borrow_mut()
                        .take()
                        .expect("app creator is called once");
                    let actual =
                        GraphicsBackend::from_context(cc).map_err(std::io::Error::other)?;
                    if actual != requested {
                        return Err(std::io::Error::other(
                            "viewer initialized an unexpected graphics backend",
                        )
                        .into());
                    }
                    backend::register_context(cc);
                    log::info!("[VIEWER-RENDERER] initialized={}", actual.key());
                    let app = create(cc)?;
                    #[cfg(target_os = "windows")]
                    let app = if actual == GraphicsBackend::Dx12 {
                        super::dx12_window::wrap(app, cc)
                    } else {
                        app
                    };
                    Ok(app)
                }),
            )
        };
        #[cfg(target_os = "windows")]
        let result = if requested == GraphicsBackend::Dx12 {
            winit::platform::windows::with_direct_composition_window(attempt)
        } else {
            attempt()
        };
        #[cfg(not(target_os = "windows"))]
        let result = attempt();
        match result {
            Err(error)
                if !selection.inherited
                    && index + 1 < selection.candidates.len()
                    && should_retry(&error, creator.borrow().is_some()) =>
            {
                log::warn!("[VIEWER-RENDERER] backend={} initialization failed; trying next backend: {error}", requested.key());
            }
            result => {
                if let Err(error) = &result {
                    log::error!(
                        "[VIEWER-RENDERER] backend={} failed: {error}",
                        requested.key()
                    );
                }
                return result;
            }
        }
    }
    unreachable!("backend selection always contains at least one candidate")
}

fn should_retry(error: &eframe::Error, creator_available: bool) -> bool {
    creator_available
        && matches!(
            error,
            eframe::Error::Wgpu(_)
                | eframe::Error::OpenGL(_)
                | eframe::Error::Glutin(_)
                | eframe::Error::NoGlutinConfigs(_, _)
                | eframe::Error::Winit(_)
        )
}

fn options_for_backend(
    mut options: eframe::NativeOptions,
    backend: GraphicsBackend,
) -> eframe::NativeOptions {
    if backend == GraphicsBackend::Glow {
        options.renderer = eframe::Renderer::Glow;
        return options;
    }
    let mut setup = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = match backend {
        GraphicsBackend::Dx12 => eframe::wgpu::Backends::DX12,
        GraphicsBackend::Vulkan => eframe::wgpu::Backends::VULKAN,
        GraphicsBackend::Glow => unreachable!(),
    };
    #[cfg(target_os = "windows")]
    if backend == GraphicsBackend::Dx12 {
        // The opt-in winit live-resize patch prepares and presents this visual
        // at the prospective client size before the OS exposes the new area.
        setup
            .instance_descriptor
            .backend_options
            .dx12
            .presentation_system = eframe::wgpu::Dx12SwapchainKind::DxgiFromVisual;
    }
    setup.power_preference = eframe::wgpu::PowerPreference::LowPower;
    let default_descriptor = Arc::clone(&setup.device_descriptor);
    setup.device_descriptor = Arc::new(move |adapter| eframe::wgpu::DeviceDescriptor {
        memory_hints: eframe::wgpu::MemoryHints::MemoryUsage,
        ..default_descriptor(adapter)
    });
    options.renderer = eframe::Renderer::Wgpu;
    options.wgpu_options = eframe::egui_wgpu::WgpuConfiguration {
        wgpu_setup: eframe::egui_wgpu::WgpuSetup::CreateNew(setup),
        ..Default::default()
    };
    options
}

#[cfg(test)]
mod tests;
