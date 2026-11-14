//! Hide standalone DX12 windows after accepted exit, before renderer teardown.

use eframe::{egui, App};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
use winit::platform::windows::WindowExtWindows;

pub(super) fn wrap<'app>(
    app: Box<dyn App + 'app>,
    cc: &eframe::CreationContext<'_>,
) -> Box<dyn App + 'app> {
    if let Some(window) = cc.winit_window() {
        window.set_present_before_resize(true);
    }
    let hwnd = cc
        .window_handle()
        .ok()
        .and_then(|handle| match handle.as_raw() {
            RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as _)),
            _ => None,
        });
    Box::new(Dx12WindowApp { app, hwnd })
}

struct Dx12WindowApp<'app> {
    app: Box<dyn App + 'app>,
    // Borrowed handle only: do not retain an Arc<Window> across renderer teardown.
    hwnd: Option<HWND>,
}

fn exit_app(app: &mut dyn App, gl: Option<&eframe::glow::Context>, hide: impl FnOnce()) {
    // on_exit runs only after close acceptance (and persistence). Never hide in
    // response to WM_CLOSE: image edits and active operations may cancel it.
    hide();
    app.on_exit(gl);
}

impl App for Dx12WindowApp<'_> {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app.logic(ctx, frame);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui(ui, frame);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.app.save(storage);
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        let hwnd = self.hwnd;
        exit_app(self.app.as_mut(), gl, || {
            if let Some(hwnd) = hwnd {
                // Synchronous: queued viewport commands are no longer processed
                // once eframe starts destroying the painter and its surfaces.
                let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
            }
        });
    }

    fn auto_save_interval(&self) -> std::time::Duration {
        self.app.auto_save_interval()
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        self.app.clear_color(visuals)
    }

    fn persist_egui_memory(&self) -> bool {
        self.app.persist_egui_memory()
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        self.app.raw_input_hook(ctx, input);
    }
}

#[cfg(test)]
mod tests;
