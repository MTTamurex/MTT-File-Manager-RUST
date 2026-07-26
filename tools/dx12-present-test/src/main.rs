use std::{error::Error, io, sync::Arc, time::Instant};

use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalSize},
    event::{ElementState, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

mod minimize;

type AnyError = Box<dyn Error>;

#[derive(Clone, Copy)]
enum PresentationMode {
    Hwnd,
    Visual,
}

impl PresentationMode {
    fn from_arg(argument: &str) -> Result<Self, String> {
        match argument {
            "--hwnd" => Ok(Self::Hwnd),
            "--visual" => Ok(Self::Visual),
            _ => Err(format!("unknown presentation mode: {argument}")),
        }
    }

    fn from_args() -> Result<Option<(Self, minimize::MinimizeFlow, bool)>, String> {
        let mut args = std::env::args().skip(1);
        let mode = match args.next().as_deref() {
            Some("--help" | "-h") => {
                Self::print_usage();
                return Ok(None);
            }
            Some(argument) => Self::from_arg(argument)?,
            None => return Err("a presentation mode is required".to_owned()),
        };

        let mut minimize_flow = minimize::MinimizeFlow::Winit;
        let mut maximized = false;
        while let Some(argument) = args.next() {
            match argument.as_str() {
                "--minimize" => {
                    let value = args
                        .next()
                        .ok_or_else(|| "--minimize requires a value".to_owned())?;
                    minimize_flow = minimize::MinimizeFlow::from_arg(&value)?;
                }
                "--maximized" => maximized = true,
                _ => return Err(format!("unexpected argument: {argument}")),
            }
        }

        Ok(Some((mode, minimize_flow, maximized)))
    }

    fn print_usage() {
        eprintln!(
            "Usage: dx12-present-test <--hwnd|--visual> [--minimize <winit|native|preview>] [--maximized]"
        );
    }

    fn label(self) -> &'static str {
        match self {
            Self::Hwnd => "DxgiFromHwnd",
            Self::Visual => "DxgiFromVisual",
        }
    }

    fn swapchain_kind(self) -> wgpu::Dx12SwapchainKind {
        match self {
            Self::Hwnd => wgpu::Dx12SwapchainKind::DxgiFromHwnd,
            Self::Visual => wgpu::Dx12SwapchainKind::DxgiFromVisual,
        }
    }

    fn clear_color(self, elapsed: f64) -> wgpu::Color {
        let pulse = elapsed.sin() * 0.12 + 0.12;
        match self {
            Self::Hwnd => wgpu::Color {
                r: 0.68 + pulse,
                g: 0.04,
                b: 0.12,
                a: 1.0,
            },
            Self::Visual => wgpu::Color {
                r: 0.02,
                g: 0.35 + pulse,
                b: 0.78,
                a: 1.0,
            },
        }
    }
}

struct GpuState {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    mode: PresentationMode,
    started_at: Instant,
}

impl GpuState {
    async fn new(window: Arc<Window>, mode: PresentationMode) -> Result<Self, AnyError> {
        let mut instance_descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        instance_descriptor.backends = wgpu::Backends::DX12;
        instance_descriptor.backend_options.dx12.presentation_system = mode.swapchain_kind();
        let instance = wgpu::Instance::new(instance_descriptor);
        let surface = instance.create_surface(Arc::clone(&window))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .await?;
        let adapter_info = adapter.get_info();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("DX12 presentation test device"),
                ..Default::default()
            })
            .await?;

        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| io::Error::other("DX12 adapter cannot present to this surface"))?;
        config.present_mode = wgpu::PresentMode::Fifo;
        config.desired_maximum_frame_latency = 2;
        surface.configure(&device, &config);

        println!("Presentation: {}", mode.label());
        println!(
            "Adapter: {} ({:?})",
            adapter_info.name, adapter_info.backend
        );
        println!("Controls: M or Space minimizes; Esc exits");

        Ok(Self {
            window,
            surface,
            device,
            queue,
            config,
            mode,
            started_at: Instant::now(),
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }

        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
    }

    fn render(&mut self) {
        let (output, reconfigure_after_present) = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(output) => (output, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(output) => (output, true),
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return,
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                eprintln!("surface lost");
                return;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                eprintln!("surface validation error");
                return;
            }
        };

        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("DX12 presentation test encoder"),
            });
        let color_attachments = [Some(wgpu::RenderPassColorAttachment {
            view: &view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(
                    self.mode
                        .clear_color(self.started_at.elapsed().as_secs_f64()),
                ),
                store: wgpu::StoreOp::Store,
            },
        })];
        {
            let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("DX12 presentation test clear"),
                color_attachments: &color_attachments,
                ..Default::default()
            });
        }

        self.queue.submit([encoder.finish()]);
        output.present();

        if reconfigure_after_present {
            self.surface.configure(&self.device, &self.config);
        }
    }
}

struct App {
    mode: PresentationMode,
    minimize_flow: minimize::MinimizeFlow,
    maximized: bool,
    gpu: Option<GpuState>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_some() {
            return;
        }

        let attributes = Window::default_attributes()
            .with_title(format!(
                "DX12 presentation test: {} / {} / {}",
                self.mode.label(),
                self.minimize_flow.label(),
                if self.maximized {
                    "maximized"
                } else {
                    "normal"
                }
            ))
            .with_inner_size(LogicalSize::new(960, 600))
            .with_maximized(self.maximized)
            .with_decorations(false);
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                eprintln!("failed to create window: {error}");
                event_loop.exit();
                return;
            }
        };

        match pollster::block_on(GpuState::new(window, self.mode)) {
            Ok(gpu) => {
                if let Err(error) = self.minimize_flow.install(&gpu.window) {
                    eprintln!("failed to install minimize flow: {error}");
                    event_loop.exit();
                    return;
                }
                println!("Minimize flow: {}", self.minimize_flow.label());
                println!(
                    "Window state: {}",
                    if self.maximized {
                        "maximized"
                    } else {
                        "normal"
                    }
                );
                gpu.window.request_redraw();
                self.gpu = Some(gpu);
            }
            Err(error) => {
                eprintln!("failed to initialize DX12: {error}");
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        if gpu.window.id() != window_id {
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => gpu.resize(size),
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !event.repeat =>
            {
                match event.physical_key {
                    PhysicalKey::Code(KeyCode::Escape) => event_loop.exit(),
                    PhysicalKey::Code(KeyCode::KeyM | KeyCode::Space) => {
                        self.minimize_flow.minimize(&gpu.window);
                    }
                    _ => {}
                }
            }
            WindowEvent::RedrawRequested => {
                gpu.render();
                gpu.window.request_redraw();
            }
            _ => {}
        }
    }
}

fn main() -> Result<(), AnyError> {
    let (mode, minimize_flow, maximized) = match PresentationMode::from_args() {
        Ok(Some(options)) => options,
        Ok(None) => return Ok(()),
        Err(error) => {
            eprintln!("error: {error}");
            PresentationMode::print_usage();
            std::process::exit(2);
        }
    };

    let event_loop = EventLoop::new()?;
    event_loop.run_app(&mut App {
        mode,
        minimize_flow,
        maximized,
        gpu: None,
    })?;
    Ok(())
}
