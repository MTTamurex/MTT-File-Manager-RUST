use eframe::egui;
use rust_i18n::t;

use super::XlsxViewerApp;

const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 4.0;

impl XlsxViewerApp {
    pub(super) fn change_zoom(&mut self, multiplier: f32) {
        self.zoom = (self.zoom * multiplier).clamp(MIN_ZOOM, MAX_ZOOM);
        self.texture = None;
        self.latest_render_key = None;
        self.render_error = None;
    }

    pub(super) fn handle_keyboard(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (zoom_in, zoom_out, reset) = ctx.input(|input| {
            let control = input.modifiers.ctrl || input.modifiers.command;
            (
                control
                    && (input.key_pressed(egui::Key::Plus) || input.key_pressed(egui::Key::Equals)),
                control && input.key_pressed(egui::Key::Minus),
                control && input.key_pressed(egui::Key::Num0),
            )
        });

        if zoom_in {
            self.change_zoom(1.2);
        } else if zoom_out {
            self.change_zoom(1.0 / 1.2);
        } else if reset {
            self.zoom = 1.0;
            self.texture = None;
            self.latest_render_key = None;
            self.render_error = None;
        }
    }

    pub(super) fn show_toolbar_frames(&mut self, root_ui: &mut egui::Ui) {
        let (bar_fill, separator) =
            crate::ui::theme::viewer_bar_colors(root_ui.visuals().dark_mode);
        egui::Panel::top("xlsx_toolbar")
            .frame(
                egui::Frame::new()
                    .fill(bar_fill)
                    .inner_margin(egui::Margin::symmetric(10, 6)),
            )
            .show(root_ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let filename = self
                        .file_path
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_default();
                    ui.add_sized(
                        [260.0, ui.spacing().interact_size.y],
                        egui::Label::new(egui::RichText::new(&filename).strong().size(13.0))
                            .truncate(),
                    )
                    .on_hover_text(filename);
                    ui.separator();
                    if ui
                        .button("−")
                        .on_hover_text(t!("xlsxviewer.zoom_out").to_string())
                        .clicked()
                    {
                        self.change_zoom(1.0 / 1.2);
                    }
                    if ui
                        .button(format!("{:.0}%", self.zoom * 100.0))
                        .on_hover_text(t!("xlsxviewer.zoom_reset").to_string())
                        .clicked()
                    {
                        self.zoom = 1.0;
                        self.texture = None;
                        self.latest_render_key = None;
                        self.render_error = None;
                    }
                    if ui
                        .button("+")
                        .on_hover_text(t!("xlsxviewer.zoom_in").to_string())
                        .clicked()
                    {
                        self.change_zoom(1.2);
                    }
                    ui.separator();
                    let search_hint = if self.search_active {
                        t!("xlsxviewer.search_close")
                    } else {
                        t!("xlsxviewer.search_hint")
                    };
                    if ui
                        .button(t!("xlsxviewer.search_button").to_string())
                        .on_hover_text(search_hint)
                        .clicked()
                    {
                        self.toggle_search();
                    }
                });
                let inner = ui.max_rect();
                ui.painter().hline(
                    inner.x_range(),
                    inner.max.y + 6.0,
                    egui::Stroke::new(1.0, separator),
                );
            });
        egui::Panel::top("xlsx_sheet_tabs")
            .frame(
                egui::Frame::new()
                    .fill(bar_fill)
                    .inner_margin(egui::Margin::symmetric(8, 4)),
            )
            .show(root_ui, |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt("xlsx_sheet_tabs_scroll")
                    .max_height(ui.spacing().interact_size.y + 4.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for index in 0..self.sheet_names.len() {
                                let name = self.sheet_names[index].clone();
                                if ui
                                    .selectable_label(self.selected_sheet == index, &name)
                                    .on_hover_text(&name)
                                    .clicked()
                                {
                                    self.select_sheet(index);
                                }
                            }
                        });
                    });
                let inner = ui.max_rect();
                ui.painter().hline(
                    inner.x_range(),
                    inner.max.y + 4.0,
                    egui::Stroke::new(1.0, separator),
                );
            });
    }
}
