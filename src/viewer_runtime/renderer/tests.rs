use super::*;
use std::cell::Cell;
use std::rc::Rc;

struct TestApp;

impl eframe::App for TestApp {
    fn ui(&mut self, _ui: &mut eframe::egui::Ui, _frame: &mut eframe::Frame) {}
}

fn options() -> eframe::NativeOptions {
    super::super::build_viewer_native_options(
        eframe::egui::ViewportBuilder::default()
            .with_title("Viewer")
            .with_inner_size([800.0, 600.0])
            .with_visible(false),
    )
}

#[test]
fn glow_runner_preserves_window_and_one_shot_payload() {
    let constructed = Rc::new(Cell::new(0));
    let mut calls = 0;
    let result = run_with(
        "Viewer",
        options(),
        app_creator(Rc::clone(&constructed)),
        BackendSelection {
            candidates: vec![GraphicsBackend::Glow],
            inherited: true,
        },
        |_, options, create| {
            calls += 1;
            assert_eq!(options.renderer, eframe::Renderer::Glow);
            assert!(options.run_and_return);
            assert!(!options.persist_window);
            assert_eq!(options.viewport.title.as_deref(), Some("Viewer"));
            assert_eq!(options.viewport.visible, Some(false));
            assert_eq!(
                options.viewport.inner_size,
                Some(eframe::egui::vec2(800.0, 600.0))
            );
            let cc = eframe::CreationContext::_new_kittest(eframe::egui::Context::default());
            create(&cc).map_err(eframe::Error::AppCreation)?;
            Ok(())
        },
    );
    assert!(result.is_ok());
    assert_eq!(calls, 1);
    assert_eq!(constructed.get(), 1);
}

fn app_creator(count: Rc<Cell<usize>>) -> eframe::AppCreator<'static> {
    // A channel receiver is deliberately non-Clone, like the image viewer's IPC
    // receiver. The runtime must pass its one-shot payload through unchanged.
    let (_sender, receiver) = std::sync::mpsc::channel::<()>();
    Box::new(move |_cc| {
        drop(receiver);
        count.set(count.get() + 1);
        Ok(Box::new(TestApp))
    })
}

#[test]
fn wgpu_initialization_error_is_returned_without_retry() {
    let constructed = Rc::new(Cell::new(0));
    let mut calls = 0;
    let result = run_with(
        "Viewer",
        options(),
        app_creator(Rc::clone(&constructed)),
        BackendSelection {
            candidates: vec![GraphicsBackend::Dx12, GraphicsBackend::Glow],
            inherited: true,
        },
        |_, options, _| {
            calls += 1;
            assert_eq!(options.renderer, eframe::Renderer::Wgpu);
            Err(eframe::Error::Wgpu(
                eframe::egui_wgpu::WgpuError::NoSurfaceFormatsAvailable,
            ))
        },
    );
    assert!(matches!(result, Err(eframe::Error::Wgpu(_))));
    assert_eq!(calls, 1);
    assert_eq!(constructed.get(), 0);
}

#[test]
fn wgpu_configuration_keeps_viewers_lightweight_and_uses_dx12_on_windows() {
    let options = options_for_backend(options(), GraphicsBackend::Dx12);
    assert_eq!(options.renderer, eframe::Renderer::Wgpu);
    assert_eq!(options.multisampling, 0);
    assert_eq!(options.depth_buffer, 0);
    assert_eq!(options.stencil_buffer, 0);
    let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = options.wgpu_options.wgpu_setup else {
        panic!("standalone viewer must create its own graphics setup");
    };
    assert_eq!(
        setup.power_preference,
        eframe::wgpu::PowerPreference::LowPower
    );
    assert_eq!(
        setup.instance_descriptor.backends,
        eframe::wgpu::Backends::DX12
    );
    #[cfg(target_os = "windows")]
    {
        assert_eq!(
            setup.instance_descriptor.backends,
            eframe::wgpu::Backends::DX12
        );
        assert_eq!(
            setup
                .instance_descriptor
                .backend_options
                .dx12
                .presentation_system,
            eframe::wgpu::Dx12SwapchainKind::DxgiFromVisual
        );
    }
}

#[test]
fn vulkan_configuration_excludes_other_backends() {
    let options = options_for_backend(options(), GraphicsBackend::Vulkan);
    assert_eq!(options.renderer, eframe::Renderer::Wgpu);
    let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = options.wgpu_options.wgpu_setup else {
        panic!("viewer must create a graphics setup");
    };
    assert_eq!(
        setup.instance_descriptor.backends,
        eframe::wgpu::Backends::VULKAN
    );
}

#[test]
fn independent_launch_falls_back_before_creating_application() {
    let constructed = Rc::new(Cell::new(0));
    let mut calls = 0;
    let result = run_with(
        "Viewer",
        options(),
        app_creator(Rc::clone(&constructed)),
        BackendSelection {
            candidates: vec![
                GraphicsBackend::Dx12,
                GraphicsBackend::Vulkan,
                GraphicsBackend::Glow,
            ],
            inherited: false,
        },
        |_, options, create| {
            calls += 1;
            if calls < 3 {
                assert_eq!(options.renderer, eframe::Renderer::Wgpu);
                return Err(eframe::Error::Wgpu(
                    eframe::egui_wgpu::WgpuError::NoSurfaceFormatsAvailable,
                ));
            }
            assert_eq!(options.renderer, eframe::Renderer::Glow);
            let cc = eframe::CreationContext::_new_kittest(eframe::egui::Context::default());
            create(&cc).map_err(eframe::Error::AppCreation)?;
            Ok(())
        },
    );
    assert!(result.is_ok());
    assert_eq!(calls, 3);
    assert_eq!(constructed.get(), 1);
}

#[test]
fn runtime_graphics_errors_do_not_reopen_application() {
    let constructed = Rc::new(Cell::new(0));
    let mut calls = 0;
    let result = run_with(
        "Viewer",
        options(),
        app_creator(Rc::clone(&constructed)),
        BackendSelection {
            candidates: vec![GraphicsBackend::Glow, GraphicsBackend::Dx12],
            inherited: false,
        },
        |_, _, create| {
            calls += 1;
            let cc = eframe::CreationContext::_new_kittest(eframe::egui::Context::default());
            create(&cc).map_err(eframe::Error::AppCreation)?;
            Err(eframe::Error::Wgpu(
                eframe::egui_wgpu::WgpuError::NoSurfaceFormatsAvailable,
            ))
        },
    );
    assert!(result.is_err());
    assert_eq!(calls, 1);
    assert_eq!(constructed.get(), 1);
}

#[test]
fn app_creation_error_does_not_trigger_fallback() {
    let mut calls = 0;
    let result = run_with(
        "Viewer",
        options(),
        Box::new(|_| Err(std::io::Error::other("file error").into())),
        BackendSelection {
            candidates: vec![GraphicsBackend::Glow, GraphicsBackend::Dx12],
            inherited: false,
        },
        |_, _, create| {
            calls += 1;
            let cc = eframe::CreationContext::_new_kittest(eframe::egui::Context::default());
            create(&cc).map_err(eframe::Error::AppCreation)?;
            Ok(())
        },
    );
    assert!(matches!(result, Err(eframe::Error::AppCreation(_))));
    assert_eq!(calls, 1);
}

#[test]
fn unexpected_initialized_backend_is_rejected_before_app_creation() {
    let constructed = Rc::new(Cell::new(0));
    let mut calls = 0;
    let result = run_with(
        "Viewer",
        options(),
        app_creator(Rc::clone(&constructed)),
        BackendSelection {
            candidates: vec![GraphicsBackend::Dx12, GraphicsBackend::Glow],
            inherited: false,
        },
        |_, _, create| {
            calls += 1;
            // A context without WGPU models Glow, not the requested DX12.
            let cc = eframe::CreationContext::_new_kittest(eframe::egui::Context::default());
            create(&cc).map_err(eframe::Error::AppCreation)?;
            Ok(())
        },
    );
    assert!(matches!(result, Err(eframe::Error::AppCreation(_))));
    assert_eq!(calls, 1);
    assert_eq!(constructed.get(), 0);
}
