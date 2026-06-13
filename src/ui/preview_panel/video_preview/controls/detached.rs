use super::{add_icon_button, icon_color};
use crate::ui::components::MediaPreview;
use crate::ui::svg_icons::SvgIconManager;
use eframe::egui;

/// Draw audio normalizer button
pub(super) fn draw_audio_normalizer(
    ui: &mut egui::Ui,
    preview: &mut MediaPreview,
    svg_manager: &mut SvgIconManager,
) {
    let normalizer_enabled = preview.is_audio_normalizer_enabled();
    let icon_color_val = icon_color(ui.visuals().dark_mode);
    let normalizer_color = if normalizer_enabled {
        [118, 185, 0, 255]
    } else {
        icon_color_val
    };

    if let Some(tex) = svg_manager.get_icon(ui.ctx(), "headphones", 48, normalizer_color) {
        let tooltip = if normalizer_enabled {
            "Normalizador: Ativo"
        } else {
            "Normalizador: Inativo"
        };
        if add_icon_button(ui, &tex, 18.0, tooltip, ui.visuals().dark_mode) {
            preview.toggle_audio_normalizer();
        }
    } else {
        // Fallback: text button if icon doesn't load
        let label = if normalizer_enabled { "N+" } else { "N" };
        if ui.small_button(label).clicked() {
            preview.toggle_audio_normalizer();
        }
    }

    ui.add_space(4.0);
}

/// Draw detached-only buttons (fullscreen, VSR)
pub(super) fn draw_detached_buttons(
    ui: &mut egui::Ui,
    preview: &mut MediaPreview,
    svg_manager: &mut SvgIconManager,
) {
    let icon_color_val = icon_color(ui.visuals().dark_mode);

    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        // Fullscreen Button
        let is_fullscreen = preview.is_maximized();
        let fs_icon_name = if is_fullscreen {
            "minimize"
        } else {
            "maximize"
        };
        let fs_tooltip = if is_fullscreen {
            "Sair da Tela Cheia (ESC)"
        } else {
            "Tela Cheia"
        };

        if let Some(tex) = svg_manager.get_icon(ui.ctx(), fs_icon_name, 48, icon_color_val) {
            if add_icon_button(ui, &tex, 18.0, fs_tooltip, ui.visuals().dark_mode) {
                if !is_fullscreen {
                    // Entering fullscreen - only set flags here.
                    // The actual ViewportCommand::Fullscreen(true) is sent
                    // from render_fullscreen_video() on the next frame.
                    let was_maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                    preview.set_prev_app_maximized(was_maximized);
                    preview.set_fullscreen_applied(false);
                    preview.toggle_maximized();
                } else {
                    // Exiting fullscreen
                    preview.set_fullscreen_applied(false);
                    preview.set_forced_size(None);
                    preview.reset_last_rect();
                    preview.toggle_maximized();
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                    if preview.prev_app_maximized() {
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Maximized(true));
                    }
                }
            }
        }

        ui.add_space(4.0);

        if preview.is_rtx_supported() {
            // VSR Button (NVIDIA Video Super Resolution)
            let is_vsr = preview.is_vsr_enabled();
            let label = if is_vsr { "VSR On" } else { "VSR Off" };

            // Custom style for ON state (NVIDIA Green), Standard style for OFF state
            let btn = if is_vsr {
                egui::Button::new(
                    egui::RichText::new(label)
                        .strong()
                        .size(10.0)
                        .color(egui::Color32::WHITE),
                )
                .fill(egui::Color32::from_rgb(118, 185, 0))
            } else {
                egui::Button::new(egui::RichText::new(label).size(10.0))
                    .fill(egui::Color32::TRANSPARENT)
            };

            if ui
                .add(btn)
                .on_hover_text(if is_vsr {
                    "Desativar NVIDIA VSR Upscaling"
                } else {
                    "Ativar NVIDIA VSR (AI Upscaling)"
                })
                .clicked()
            {
                if let Err(e) = preview.toggle_vsr() {
                    log::error!("toggling VSR: {}", e);
                }
            }
        }
    });
}
