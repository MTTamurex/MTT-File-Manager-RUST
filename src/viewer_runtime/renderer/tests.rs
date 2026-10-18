use super::*;
use std::cell::Cell;

struct TestApp;

impl eframe::App for TestApp {
    fn ui(&mut self, _ui: &mut eframe::egui::Ui, _frame: &mut eframe::Frame) {}
}

fn gl_error() -> eframe::Error {
    eframe::Error::OpenGL("egui_glow requires opengl 2.0+.".to_owned().into())
}

fn options() -> eframe::NativeOptions {
    super::super::build_viewer_native_options(
        eframe::egui::ViewportBuilder::default()
            .with_title("Viewer")
            .with_inner_size([800.0, 600.0])
            .with_visible(false),
    )
}

fn app_creator(count: Rc<Cell<usize>>) -> eframe::AppCreator<'static> {
    // A channel receiver is deliberately non-Clone, like the image viewer's IPC
    // receiver. Retrying must preserve this one-shot payload, not recreate it.
    let (_sender, receiver) = std::sync::mpsc::channel::<()>();
    Box::new(move |_cc| {
        drop(receiver);
        count.set(count.get() + 1);
        Ok(Box::new(TestApp))
    })
}

#[test]
fn working_glow_does_not_initialize_wgpu_or_recreate_the_app() {
    let constructed = Rc::new(Cell::new(0));
    let mut calls = 0;
    let mut native_options = options();
    native_options.run_and_return = false;
    let result = run_with(
        "Viewer",
        native_options,
        app_creator(Rc::clone(&constructed)),
        |_, options, create| {
            calls += 1;
            assert_eq!(options.renderer, eframe::Renderer::Glow);
            assert!(options.run_and_return);
            let cc = eframe::CreationContext::_new_kittest(eframe::egui::Context::default());
            create(&cc).map_err(eframe::Error::AppCreation)?;
            Ok(())
        },
    );
    assert!(result.is_ok());
    assert_eq!(calls, 1);
    assert_eq!(constructed.get(), 1);
}

#[test]
fn missing_opengl_retries_with_wgpu_and_preserves_one_shot_app_payload() {
    let constructed = Rc::new(Cell::new(0));
    let mut calls = 0;
    let result = run_with(
        "Viewer",
        options(),
        app_creator(Rc::clone(&constructed)),
        |_, options, create| {
            calls += 1;
            if calls == 1 {
                assert_eq!(options.renderer, eframe::Renderer::Glow);
                return Err(gl_error());
            }
            assert_eq!(options.renderer, eframe::Renderer::Wgpu);
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
    assert_eq!(calls, 2);
    assert_eq!(constructed.get(), 1);
}

#[test]
fn graphics_errors_after_app_creation_do_not_reopen_the_viewer() {
    let constructed = Rc::new(Cell::new(0));
    let mut calls = 0;
    let result = run_with(
        "Viewer",
        options(),
        app_creator(Rc::clone(&constructed)),
        |_, _, create| {
            calls += 1;
            let cc = eframe::CreationContext::_new_kittest(eframe::egui::Context::default());
            create(&cc).map_err(eframe::Error::AppCreation)?;
            Err(gl_error())
        },
    );
    assert!(matches!(result, Err(eframe::Error::OpenGL(_))));
    assert_eq!(calls, 1);
    assert_eq!(constructed.get(), 1);
}

#[test]
fn application_creation_errors_do_not_trigger_graphics_fallback() {
    let mut calls = 0;
    let result = run_with(
        "Viewer",
        options(),
        Box::new(|_| Err(std::io::Error::other("could not open file").into())),
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
fn failure_of_both_renderers_returns_the_final_error_without_looping() {
    let constructed = Rc::new(Cell::new(0));
    let mut calls = 0;
    let result = run_with(
        "Viewer",
        options(),
        app_creator(Rc::clone(&constructed)),
        |_, _, _| {
            calls += 1;
            if calls == 1 {
                Err(gl_error())
            } else {
                Err(eframe::Error::Wgpu(
                    eframe::egui_wgpu::WgpuError::NoSurfaceFormatsAvailable,
                ))
            }
        },
    );
    assert!(matches!(result, Err(eframe::Error::Wgpu(_))));
    assert_eq!(calls, 2);
    assert_eq!(constructed.get(), 0);
}

#[test]
fn fallback_requires_a_graphics_initialization_failure_and_an_unused_creator() {
    assert!(should_fallback(&gl_error(), true));
    assert!(!should_fallback(&gl_error(), false));
    let application_error = eframe::Error::AppCreation(std::io::Error::other("file error").into());
    assert!(!should_fallback(&application_error, true));
}

#[test]
fn fallback_configuration_keeps_viewers_lightweight_and_uses_dx12_on_windows() {
    let options = wgpu_options(options());
    assert_eq!(options.renderer, eframe::Renderer::Wgpu);
    assert_eq!(options.multisampling, 0);
    assert_eq!(options.depth_buffer, 0);
    assert_eq!(options.stencil_buffer, 0);
    let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = options.wgpu_options.wgpu_setup else {
        panic!("fallback must create its own graphics setup");
    };
    assert_eq!(
        setup.power_preference,
        eframe::wgpu::PowerPreference::LowPower
    );
    assert_eq!(setup.instance_descriptor.backends, fallback_backends());
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
