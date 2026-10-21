use crate::domain::file_entry::ViewMode;
use crate::ui::theme;
use eframe::egui;
use rust_i18n::t;

pub(crate) fn compact_width(ui: &egui::Ui) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let labels_width: f32 = [
        t!("status_bar.thumbnail_size"),
        t!("settings.interface_scale"),
    ]
    .into_iter()
    .map(|text| {
        ui.painter()
            .layout_no_wrap(text.to_string(), font.clone(), theme::text_color(ui.ctx()))
            .size()
            .x
    })
    .sum();
    // Reserve both sliders, spacing, and half the centered selection group on each side.
    (2.0 * (labels_width + 112.0 + 32.0 + 46.0 + 16.0)).max(720.0)
}

pub(crate) fn render(
    ui: &mut egui::Ui,
    view_mode: ViewMode,
    thumbnail_size: &mut f32,
    interface_scale: &mut f32,
) {
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
                    ui.add_space(12.0);
                    ui.label(
                        egui::RichText::new(t!("settings.interface_scale").to_string())
                            .color(theme::text_color(ui.ctx())),
                    );
                    render_scale_slider(ui, interface_scale);
                });
            });
    });
}

fn render_scale_slider(ui: &mut egui::Ui, interface_scale: &mut f32) -> egui::Response {
    let draft_id = ui.make_persistent_id("interface_scale_percent");
    let draft = ui.data(|data| data.get_temp::<f32>(draft_id));
    let mut percent = draft.unwrap_or(*interface_scale * 100.0);
    let response = ui.add(
        egui::Slider::new(
            &mut percent,
            100.0..=crate::ui::interface_scale::MAX * 100.0,
        )
        .step_by(5.0)
        .show_value(false)
        .trailing_fill(true),
    );
    if response.is_pointer_button_down_on() {
        // Changing scale while dragging would move the status bar under the pointer.
        ui.data_mut(|data| data.insert_temp(draft_id, percent));
    } else {
        ui.data_mut(|data| data.remove::<f32>(draft_id));
        if response.changed() || draft.is_some() {
            *interface_scale = percent / 100.0;
        }
    }
    response
}

#[cfg(test)]
mod tests;
