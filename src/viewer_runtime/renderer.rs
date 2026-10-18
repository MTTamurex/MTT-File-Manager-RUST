//! Glow-first startup with a single WGPU fallback before app creation.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// Preserve the one-shot app creator across renderer initialization attempts.
/// No retry is made after app creation starts: file errors and runtime failures
/// must not reopen the viewer or duplicate its workers and IPC receivers.
pub fn run_viewer_native(
    app_name: &str,
    options: eframe::NativeOptions,
    app_creator: eframe::AppCreator<'_>,
) -> eframe::Result<()> {
    run_with(app_name, options, app_creator, |name, options, creator| {
        eframe::run_native(name, options, creator)
    })
}

fn run_with<'app>(
    app_name: &str,
    mut options: eframe::NativeOptions,
    app_creator: eframe::AppCreator<'app>,
    mut run: impl FnMut(&str, eframe::NativeOptions, eframe::AppCreator<'app>) -> eframe::Result<()>,
) -> eframe::Result<()> {
    options.renderer = eframe::Renderer::Glow;
    // eframe must return initialization failures instead of exiting the process.
    options.run_and_return = true;
    let fallback_options = options.clone();
    let creator = Rc::new(RefCell::new(Some(app_creator)));

    log::info!("[VIEWER-RENDERER] attempt=glow");
    let result = run(
        app_name,
        options,
        pending_creator(Rc::clone(&creator), "glow"),
    );
    match result {
        Err(error) if should_fallback(&error, creator.borrow().is_some()) => {
            log::warn!("[VIEWER-RENDERER] Glow initialization failed; trying WGPU: {error}");
            log::info!(
                "[VIEWER-RENDERER] attempt=wgpu backends={:?}",
                fallback_backends()
            );
            let result = run(
                app_name,
                wgpu_options(fallback_options),
                pending_creator(creator, "wgpu"),
            );
            if let Err(error) = &result {
                log::error!("[VIEWER-RENDERER] WGPU fallback failed: {error}");
            }
            result
        }
        result => result,
    }
}

fn pending_creator<'app>(
    creator: Rc<RefCell<Option<eframe::AppCreator<'app>>>>,
    renderer: &'static str,
) -> eframe::AppCreator<'app> {
    Box::new(move |cc| {
        let create = creator
            .borrow_mut()
            .take()
            .expect("app creator is called only once");
        let app = create(cc)?;
        log::info!("[VIEWER-RENDERER] initialized={renderer}");
        Ok(app)
    })
}

fn should_fallback(error: &eframe::Error, creator_available: bool) -> bool {
    creator_available
        && matches!(
            error,
            eframe::Error::OpenGL(_)
                | eframe::Error::Glutin(_)
                | eframe::Error::NoGlutinConfigs(_, _)
                | eframe::Error::Winit(_)
        )
}

fn fallback_backends() -> eframe::wgpu::Backends {
    #[cfg(target_os = "windows")]
    {
        eframe::wgpu::Backends::DX12
    }
    #[cfg(not(target_os = "windows"))]
    {
        eframe::wgpu::Backends::PRIMARY
    }
}

fn wgpu_options(mut options: eframe::NativeOptions) -> eframe::NativeOptions {
    let mut setup = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = fallback_backends();
    #[cfg(target_os = "windows")]
    {
        // Same presentation path as the main window, including VM drivers.
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
