use crate::domain::file_entry::ViewMode;
use crate::ui::theme;
use eframe::egui;
use rust_i18n::t;

pub(crate) fn render(ui: &mut egui::Ui, view_mode: ViewMode, thumbnail_size: &mut f32) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        egui::Frame::NONE
            .inner_margin(egui::Margin {
                left: 0,
                right: 0,
                top: 0,
                bottom: 2,
            })
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.scope(|ui| {
                    let is_dark = ui.visuals().dark_mode;
                    let slider_track = if is_dark {
                        egui::Color32::from_gray(78)
                    } else {
                        egui::Color32::from_gray(228)
                    };
                    let slider_track_stroke = if is_dark {
                        egui::Stroke::new(1.0, egui::Color32::from_gray(98))
                    } else {
                        egui::Stroke::new(1.0, egui::Color32::from_gray(214))
                    };
                    let slider_fill = if is_dark {
                        egui::Color32::from_rgb(36, 170, 231)
                    } else {
                        egui::Color32::from_rgb(33, 176, 236)
                    };
                    let slider_knob = if is_dark {
                        egui::Color32::from_gray(220)
                    } else {
                        egui::Color32::from_gray(224)
                    };
                    let slider_knob_stroke = if is_dark {
                        egui::Stroke::new(1.0, egui::Color32::from_gray(150))
                    } else {
                        egui::Stroke::new(1.0, egui::Color32::from_gray(190))
                    };
                    let slider_knob_hover = if is_dark {
                        egui::Color32::from_gray(232)
                    } else {
                        egui::Color32::from_gray(234)
                    };

                    ui.visuals_mut().selection.bg_fill = slider_fill;

                    ui.visuals_mut().widgets.noninteractive.bg_fill = slider_track;
                    ui.visuals_mut().widgets.noninteractive.weak_bg_fill = slider_track;
                    ui.visuals_mut().widgets.noninteractive.fg_stroke = slider_knob_stroke;
                    ui.visuals_mut().widgets.noninteractive.bg_stroke = slider_track_stroke;

                    ui.visuals_mut().widgets.inactive.bg_fill = slider_track;
                    ui.visuals_mut().widgets.inactive.weak_bg_fill = slider_track;
                    ui.visuals_mut().widgets.inactive.fg_stroke = slider_knob_stroke;
                    ui.visuals_mut().widgets.inactive.bg_stroke = slider_track_stroke;

                    ui.visuals_mut().widgets.hovered.bg_fill = slider_knob_hover;
                    ui.visuals_mut().widgets.hovered.weak_bg_fill = slider_track;
                    ui.visuals_mut().widgets.hovered.fg_stroke = slider_knob_stroke;
                    ui.visuals_mut().widgets.hovered.bg_stroke = slider_track_stroke;

                    ui.visuals_mut().widgets.active.bg_fill = slider_knob;
                    ui.visuals_mut().widgets.active.weak_bg_fill = slider_track;
                    ui.visuals_mut().widgets.active.fg_stroke = slider_knob_stroke;
                    ui.visuals_mut().widgets.active.bg_stroke = slider_track_stroke;

                    let slider_enabled =
                        !matches!(view_mode, ViewMode::List | ViewMode::ColumnList);
                    ui.label(
                        egui::RichText::new(t!("status_bar.thumbnail_size").to_string())
                            .color(theme::text_color(ui.ctx())),
                    );
                    ui.spacing_mut().slider_width = 56.0;
                    ui.spacing_mut().interact_size.y = 14.0;
                    ui.add_enabled(
                        slider_enabled,
                        egui::Slider::new(thumbnail_size, theme::THUMBNAIL_MIN..=256.0)
                            .show_value(false)
                            .trailing_fill(true),
                    );
                });
            });
    });
}
