//! Toolbar UI for the PDF viewer.
//!
//! Implements the top toolbar with page navigation, zoom controls, and rotation
//! buttons. Methods are defined as `impl PdfViewerApp` extensions.

use eframe::egui;
use rust_i18n::t;

use super::viewer_app::{PdfPageLayout, PdfViewerApp, ZoomMode};

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

            // ── Page layout ──────────────────────────────────────────────
            self.toolbar_page_layout(ui);

            ui.separator();

            // ── Rotation ─────────────────────────────────────────────────
            self.toolbar_rotation(ui);

            ui.separator();

            // ── Selection ────────────────────────────────────────────────
            self.toolbar_selection(ui);

            ui.separator();

            // ── Search ───────────────────────────────────────────────────
            self.toolbar_search(ui);
        });
    }

    // ── Sections ─────────────────────────────────────────────────────────

    fn toolbar_navigation(&mut self, ui: &mut egui::Ui) {
        if ui
            .button("⏮")
            .on_hover_text(t!("pdfviewer.first_page"))
            .clicked()
        {
            self.go_to_page(0);
        }

        if ui
            .button("◀")
            .on_hover_text(t!("pdfviewer.previous_page"))
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
        self.page_input_has_focus = response.has_focus();
        if response.changed() {
            self.page_input.retain(|ch| ch.is_ascii_digit());
        }

        let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));
        if response.has_focus() && enter_pressed {
            self.commit_page_input();
            response.request_focus();
        } else if response.lost_focus() {
            self.commit_page_input();
        }

        ui.label(format!("/ {}", self.total_pages));

        if ui
            .button("▶")
            .on_hover_text(t!("pdfviewer.next_page"))
            .clicked()
        {
            self.next_page();
        }

        if ui
            .button("⏭")
            .on_hover_text(t!("pdfviewer.last_page"))
            .clicked()
        {
            self.go_to_page(self.total_pages.saturating_sub(1));
        }
    }

    fn commit_page_input(&mut self) {
        if let Ok(page) = self.page_input.parse::<u32>() {
            self.go_to_page(page.saturating_sub(1));
        } else {
            self.page_input = format!("{}", self.current_page + 1);
        }
    }

    fn toolbar_zoom(&mut self, ui: &mut egui::Ui) {
        if ui
            .button("➖")
            .on_hover_text(t!("pdfviewer.zoom_out"))
            .clicked()
        {
            self.zoom_out();
        }

        // Current zoom indicator — always show actual percentage
        let zoom_label = format!("{:.0}%", self.effective_zoom_pct);
        ui.label(egui::RichText::new(&zoom_label).strong().size(13.0))
            .on_hover_text(t!("pdfviewer.current_zoom"));

        if ui
            .button("➕")
            .on_hover_text(t!("pdfviewer.zoom_in"))
            .clicked()
        {
            self.zoom_in();
        }

        if ui
            .selectable_label(
                self.zoom_mode == ZoomMode::FitWidth,
                t!("pdfviewer.fit_width"),
            )
            .clicked()
        {
            self.zoom_mode = ZoomMode::FitWidth;
            self.on_view_changed();
        }

        if ui
            .selectable_label(
                self.zoom_mode == ZoomMode::FitPage,
                t!("pdfviewer.fit_page"),
            )
            .clicked()
        {
            self.zoom_mode = ZoomMode::FitPage;
            self.on_view_changed();
        }
    }

    fn toolbar_rotation(&mut self, ui: &mut egui::Ui) {
        if ui
            .button("↺")
            .on_hover_text(t!("pdfviewer.rotate_ccw"))
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
            .on_hover_text(t!("pdfviewer.rotate_cw"))
            .clicked()
        {
            self.rotate_cw();
        }
    }

    fn toolbar_page_layout(&mut self, ui: &mut egui::Ui) {
        if ui
            .selectable_label(
                self.page_layout == PdfPageLayout::OnePage,
                t!("pdfviewer.view_single_short"),
            )
            .on_hover_text(t!("pdfviewer.view_single"))
            .clicked()
        {
            self.set_page_layout(PdfPageLayout::OnePage);
        }

        if ui
            .selectable_label(
                self.page_layout == PdfPageLayout::TwoPage,
                t!("pdfviewer.view_two_page_short"),
            )
            .on_hover_text(t!("pdfviewer.view_two_page"))
            .clicked()
        {
            self.set_page_layout(PdfPageLayout::TwoPage);
        }
    }

    fn toolbar_selection(&mut self, ui: &mut egui::Ui) {
        let button = ui.add_enabled(
            self.has_text_selection(),
            egui::Button::new(t!("pdfviewer.copy_selection")),
        );

        if button.clicked() {
            if let Err(err) = self.copy_selected_text() {
                log::warn!("[PDF-VIEWER] copy selection failed: {err}");
            }
        }

        button.on_hover_text(t!("pdfviewer.copy_selection_hint"));

        ui.label(self.selection_summary());
    }

    fn toolbar_search(&mut self, ui: &mut egui::Ui) {
        let label = if self.search_active {
            t!("pdfviewer.search_close")
        } else {
            t!("pdfviewer.search_hint")
        };
        if ui
            .button(t!("pdfviewer.search_button").to_string())
            .on_hover_text(label.clone())
            .clicked()
        {
            self.toggle_search();
        }
    }
}
