use crate::tabs::TabManager;
use crate::ui::icon_loader::IconLoader;
use crate::ui::svg_icons::SvgIconManager;
use eframe::egui::{self, Color32, CornerRadius, Stroke, Vec2};

use super::{drag_dwell, TabBarAction};

#[allow(clippy::too_many_arguments)]
pub(super) fn render_tabs(
    ui: &mut egui::Ui,
    tab_manager: &TabManager,
    svg_icons: &mut SvgIconManager,
    computer_icon: Option<&egui::TextureHandle>,
    icon_loader: &mut IconLoader,
    media_owner_id: Option<usize>,
    is_playing: bool,
    is_muted: bool,
    is_item_dragging: bool,
    ideal_tab_width: f32,
    tab_height: f32,
    tab_padding: f32,
    close_btn_size: f32,
    active_bg: Color32,
    inactive_bg: Color32,
    hover_bg: Color32,
    text_color: Color32,
    inactive_text: Color32,
    is_dark: bool,
) -> TabBarAction {
    let mut action = TabBarAction::None;

    for (idx, tab) in tab_manager.tabs.iter().enumerate() {
        let is_active = idx == tab_manager.active_tab;

        let sense = if is_item_dragging {
            egui::Sense::click_and_drag()
        } else {
            egui::Sense::click()
        };
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(ideal_tab_width, tab_height), sense);

        if response.clicked() {
            action = TabBarAction::SwitchTab(idx);
        }

        if response.middle_clicked() {
            action = TabBarAction::CloseTab(idx);
        }
        if drag_dwell::should_activate_tab_on_drag_hover(
            ui,
            idx,
            is_item_dragging,
            is_active,
            response.contains_pointer(),
        ) {
            action = TabBarAction::SwitchTab(idx);
        }

        let drag_hovering = is_item_dragging && !is_active && response.contains_pointer();
        let bg_color = if is_active {
            active_bg
        } else if drag_hovering {
            egui::Color32::from_rgba_unmultiplied(60, 130, 220, 90)
        } else if response.hovered() {
            hover_bg
        } else {
            inactive_bg
        };

        ui.painter().rect_filled(
            rect,
            CornerRadius {
                nw: 6,
                ne: 6,
                sw: 0,
                se: 0,
            },
            bg_color,
        );

        let content_rect = rect.shrink2(Vec2::new(tab_padding, 4.0));

        let icon_size = 16.0;
        let icon_pos = content_rect.min + Vec2::new(0.0, (content_rect.height() - icon_size) / 2.0);
        let icon_rect = egui::Rect::from_min_size(icon_pos, Vec2::splat(icon_size));

        let render_size = 32;
        let icon_name = if tab.is_computer_view {
            "home"
        } else {
            "folder"
        };
        let icon_color = if is_active {
            [30, 90, 180, 255]
        } else {
            [80, 80, 80, 255]
        };

        let native_icon = if tab.is_computer_view {
            computer_icon.cloned()
        } else if tab.path == "Lixeira" {
            icon_loader.ensure_recycle_bin_icon(ui.ctx())
        } else {
            icon_loader.get_or_load_folder_path_icon(ui.ctx(), &tab.path)
        };

        if let Some(texture) = native_icon {
            ui.painter().image(
                texture.id(),
                icon_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        } else if let Some(texture) =
            svg_icons.get_icon(ui.ctx(), icon_name, render_size, icon_color)
        {
            ui.painter().image(
                texture.id(),
                icon_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }

        let title_x = icon_pos.x + icon_size + 6.0;

        let speaker_btn_size = 14.0;
        let has_speaker = media_owner_id == Some(tab.id) && is_playing;
        let speaker_width = if has_speaker {
            speaker_btn_size + 8.0
        } else {
            0.0
        };

        let title_max_width =
            ideal_tab_width - icon_size - close_btn_size - speaker_width - tab_padding * 2.0 - 12.0;

        let font_id = egui::FontId::proportional(13.0);
        let title_color = if is_active { text_color } else { inactive_text };

        let full_text = tab.title.clone();
        let mut title_text = full_text.clone();

        let galley = ui
            .painter()
            .layout_no_wrap(title_text.clone(), font_id.clone(), title_color);

        if galley.rect.width() > title_max_width {
            let mut boundaries: Vec<usize> = full_text.char_indices().map(|(i, _)| i).collect();
            boundaries.push(full_text.len());
            let mut low = 0usize;
            let mut high = boundaries.len().saturating_sub(1);

            while low < high {
                let mid = (low + high + 1) / 2;
                let byte_idx = boundaries[mid];
                let test_text = format!("{}...", &full_text[..byte_idx]);
                let test_galley =
                    ui.painter()
                        .layout_no_wrap(test_text.clone(), font_id.clone(), title_color);

                if test_galley.rect.width() <= title_max_width {
                    low = mid;
                } else {
                    high = mid - 1;
                }
            }

            if low > 0 {
                let byte_idx = boundaries[low];
                title_text = format!("{}...", &full_text[..byte_idx]);
            } else {
                title_text = "...".to_string();
            }
        }

        ui.painter().text(
            egui::pos2(title_x, content_rect.center().y),
            egui::Align2::LEFT_CENTER,
            title_text,
            font_id,
            title_color,
        );

        if has_speaker {
            let speaker_x = rect.max.x - close_btn_size - tab_padding - speaker_btn_size - 4.0;
            let speaker_y = content_rect.center().y - speaker_btn_size / 2.0;
            let speaker_rect = egui::Rect::from_min_size(
                egui::pos2(speaker_x, speaker_y),
                Vec2::splat(speaker_btn_size),
            );

            let speaker_response = ui.interact(
                speaker_rect,
                egui::Id::new(format!("speaker_{}", idx)),
                egui::Sense::click(),
            );

            if speaker_response.clicked() {
                action = TabBarAction::ToggleMute(idx);
            }

            let icon_name = if is_muted { "vol_mute" } else { "vol_high" };
            let icon_color = if speaker_response.hovered() {
                if is_dark {
                    [255, 255, 255, 255]
                } else {
                    [0, 0, 0, 255]
                }
            } else if is_dark {
                [200, 200, 200, 255]
            } else {
                [80, 80, 80, 255]
            };

            if let Some(tex) = svg_icons.get_icon(ui.ctx(), icon_name, 32, icon_color) {
                ui.painter().image(
                    tex.id(),
                    speaker_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }

            speaker_response.on_hover_text(if is_muted {
                "Ativar Áudio"
            } else {
                "Mutar Áudio"
            });
        }

        let close_btn_x = rect.max.x - close_btn_size - tab_padding;
        let close_btn_y = content_rect.center().y - close_btn_size / 2.0;
        let close_btn_rect = egui::Rect::from_min_size(
            egui::pos2(close_btn_x, close_btn_y),
            Vec2::splat(close_btn_size),
        );

        let close_response = ui.interact(
            close_btn_rect,
            egui::Id::new(format!("close_{}", idx)),
            egui::Sense::click(),
        );

        if close_response.clicked() {
            action = TabBarAction::CloseTab(idx);
        }

        if close_response.hovered() {
            ui.painter()
                .rect_filled(close_btn_rect, CornerRadius::same(4), hover_bg);
        }

        let x_stroke = Stroke::new(
            1.5,
            if close_response.hovered() {
                text_color
            } else {
                inactive_text
            },
        );
        let x_center = close_btn_rect.center();
        let x_radius = close_btn_size * 0.25;
        ui.painter().line_segment(
            [
                x_center + Vec2::new(-x_radius, -x_radius),
                x_center + Vec2::new(x_radius, x_radius),
            ],
            x_stroke,
        );
        ui.painter().line_segment(
            [
                x_center + Vec2::new(x_radius, -x_radius),
                x_center + Vec2::new(-x_radius, x_radius),
            ],
            x_stroke,
        );
    }

    action
}
