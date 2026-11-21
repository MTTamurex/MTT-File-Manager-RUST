use super::TextViewerApp;
use eframe::egui;

impl eframe::App for TextViewerApp {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if crate::viewer_runtime::poll_viewer_fonts(ctx) {
            self.markdown_document.invalidate_layout();
        }
        self.run_idle_working_set_trim(ctx);

        if self.dark_mode.take().is_some() {
            crate::ui::theme::apply_viewer_visuals(ctx, crate::viewer_runtime::saved_theme_mode());

            use raw_window_handle::HasWindowHandle;
            if let Ok(handle) = frame.window_handle() {
                if let raw_window_handle::RawWindowHandle::Win32(handle) = handle.as_raw() {
                    let hwnd = windows::Win32::Foundation::HWND(handle.hwnd.get() as _);
                    self.native_hwnd = Some(handle.hwnd.get());
                    crate::infrastructure::windows::center_window_on_primary_monitor(hwnd);
                    crate::ui::theme::apply_native_title_bar(ctx, hwnd);
                }
            }

            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx_owned = ui.ctx().clone();
        let ctx = &ctx_owned;
        if let Some(mode) =
            crate::viewer_runtime::poll_saved_theme_change(ctx, &mut self.last_theme_poll)
        {
            crate::ui::theme::apply_viewer_visuals(ctx, mode);
            if let Some(raw) = self.native_hwnd {
                crate::ui::theme::apply_native_title_bar(
                    ctx,
                    windows::Win32::Foundation::HWND(raw as *mut _),
                );
            }
        }
        crate::viewer_runtime::schedule_saved_theme_poll(ctx, self.last_theme_poll);
        ui.set_style(ctx.global_style());
        self.syntax_highlighting.update(
            ctx,
            &self.file_path,
            std::sync::Arc::clone(&self.content),
            self.is_markdown,
            ui.visuals().dark_mode,
        );
        self.syntax_highlighting.poll(ctx);
        self.handle_search_shortcuts(ctx);
        self.handle_keyboard(ctx);

        let (toolbar_frame, separator) = super::toolbar::top_bar_frame(ui);
        egui::Panel::top("text_toolbar")
            .frame(toolbar_frame)
            .show(ui, |ui| {
                self.show_toolbar(ui);
                super::toolbar::paint_bar_separator(ui, separator);
            });

        if self.search_is_open() {
            let (search_frame, separator) = super::toolbar::top_bar_frame(ui);
            egui::Panel::top("text_search")
                .frame(search_frame)
                .show(ui, |ui| {
                    self.show_search_bar(ui);
                    super::toolbar::paint_bar_separator(ui, separator);
                });
        }
        if self.goto_open {
            let (goto_frame, separator) = super::toolbar::top_bar_frame(ui);
            egui::Panel::top("text_goto")
                .frame(goto_frame)
                .show(ui, |ui| {
                    self.show_goto_bar(ui);
                    super::toolbar::paint_bar_separator(ui, separator);
                });
        }

        egui::CentralPanel::default().show(ui, |ui| {
            self.show_content(ui);
        });
    }
}
