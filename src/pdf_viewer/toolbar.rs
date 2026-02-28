//! Toolbar UI for the PDF viewer.
//!
//! Implements the top toolbar with page navigation, zoom controls, and rotation
//! buttons. Methods are defined as `impl PdfViewerApp` extensions.

use eframe::egui;

use super::viewer_app::{PdfViewerApp, ZoomMode};

impl PdfViewerApp {
    /// Render the toolbar contents inside a horizontal layout.
    pub(super) fn show_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;

            // ── Page navigation ──────────────────────────────────────────
            self.toolbar_navigation(ui);

            ui.separator();

            // ── Zoom controls ────────────────────────────────────────────
            self.toolbar_zoom(ui);

            ui.separator();

            // ── Rotation ─────────────────────────────────────────────────
            self.toolbar_rotation(ui);
        });
    }

    // ── Sections ─────────────────────────────────────────────────────────

    fn toolbar_navigation(&mut self, ui: &mut egui::Ui) {
        if ui
            .button("⏮")
            .on_hover_text("First page  (Ctrl+Home)")
            .clicked()
        {
            self.go_to_page(0);
        }

        if ui
            .button("◀")
            .on_hover_text("Previous page  (PgUp)")
            .clicked()
        {
            self.prev_page();
        }

        // Editable page number
        let response = ui.add(
            egui::TextEdit::singleline(&mut self.page_input)
                .desired_width(40.0)
                .horizontal_align(egui::Align::Center),
        );
        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            if let Ok(page) = self.page_input.parse::<u32>() {
                self.go_to_page(page.saturating_sub(1));
            }
        }

        ui.label(format!("/ {}", self.total_pages));

        if ui
            .button("▶")
            .on_hover_text("Next page  (PgDn)")
            .clicked()
        {
            self.next_page();
        }

        if ui
            .button("⏭")
            .on_hover_text("Last page  (Ctrl+End)")
            .clicked()
        {
            self.go_to_page(self.total_pages.saturating_sub(1));
        }
    }

    fn toolbar_zoom(&mut self, ui: &mut egui::Ui) {
        if ui
            .button("➖")
            .on_hover_text("Zoom out  (Ctrl+−)")
            .clicked()
        {
            self.zoom_out();
        }

        // Current zoom indicator — always show actual percentage
        let zoom_label = format!("{:.0}%", self.effective_zoom_pct);
        ui.label(
            egui::RichText::new(&zoom_label)
                .strong()
                .size(13.0),
        )
        .on_hover_text("Current zoom level");

        if ui
            .button("➕")
            .on_hover_text("Zoom in  (Ctrl++)")
            .clicked()
        {
            self.zoom_in();
        }

        if ui
            .selectable_label(self.zoom_mode == ZoomMode::FitWidth, "Fit Width")
            .clicked()
        {
            self.zoom_mode = ZoomMode::FitWidth;
            self.on_view_changed();
        }

        if ui
            .selectable_label(self.zoom_mode == ZoomMode::FitPage, "Fit Page")
            .clicked()
        {
            self.zoom_mode = ZoomMode::FitPage;
            self.on_view_changed();
        }
    }

    fn toolbar_rotation(&mut self, ui: &mut egui::Ui) {
        if ui
            .button("↺")
            .on_hover_text("Rotate counter-clockwise  (Shift+R)")
            .clicked()
        {
            self.rotate_ccw();
        }

        ui.label(
            egui::RichText::new(format!("{}°", self.rotation))
                .strong()
                .size(13.0),
        );

        if ui
            .button("↻")
            .on_hover_text("Rotate clockwise  (R)")
            .clicked()
        {
            self.rotate_cw();
        }
    }
}
