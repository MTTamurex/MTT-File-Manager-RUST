use crate::app::global_search_state::TooltipRequest;
use crate::app::state::ImageViewerApp;
use eframe::egui;
use rust_i18n::t;

use super::super::actions;

const TOOLTIP_DELAY_SECS: f32 = crate::ui::views::common::TOOLTIP_DELAY_SECS;

#[allow(clippy::too_many_arguments)]
pub(super) fn render(
    ui: &mut egui::Ui,
    app: &mut ImageViewerApp,
    response: &egui::Response,
    source_index: usize,
    full_path: &str,
    result_name: &str,
    is_directory: bool,
    size: u64,
    file_type: &str,
    tag_ids: &[i64],
    enabled: bool,
) {
    let hover_id = egui::Id::new("global_search_hover_start").with(full_path);
    if !response.hovered() || !enabled {
        ui.ctx().data_mut(|data| data.remove::<f64>(hover_id));
        return;
    }

    let current_time = ui.input(|input| input.time);
    let hover_start_time = ui
        .ctx()
        .data_mut(|data| *data.get_temp_mut_or_insert_with(hover_id, || current_time));
    let hover_duration = (current_time - hover_start_time) as f32;
    if hover_duration < TOOLTIP_DELAY_SECS {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f32(
                TOOLTIP_DELAY_SECS - hover_duration + 0.01,
            ));
        return;
    }

    let modified_ts = resolve_modified_timestamp(app, source_index, full_path);
    let size_text = actions::resolve_result_size(app, full_path, is_directory, size)
        .map(crate::infrastructure::windows::format_size);
    let thumbnail = resolve_thumbnail(app, full_path, is_directory);
    let Some(mouse_pos) = ui.input(|input| input.pointer.hover_pos()) else {
        return;
    };

    let tooltip_id = response.id.with("tooltip");
    let tooltip_layer = egui::LayerId::new(egui::Order::Tooltip, tooltip_id);
    let tooltip_response =
        crate::ui::video_overlay::without_shadow_if_needed(ui.ctx(), tooltip_id, || {
            egui::Tooltip::always_open(ui.ctx().clone(), tooltip_layer, response.id, mouse_pos)
                .show(|ui: &mut egui::Ui| {
                    ui.set_max_width(300.0);
                    ui.vertical(|ui| {
                        crate::ui::views::file_tooltip::header(ui, result_name);
                        crate::ui::views::file_tooltip::media_preview_from_option(
                            ui,
                            thumbnail.as_ref(),
                        );
                        crate::ui::views::file_tooltip::info_row(
                            ui,
                            &t!("file_info.type"),
                            file_type,
                        );
                        if !is_directory {
                            crate::ui::views::file_tooltip::info_row(
                                ui,
                                &t!("file_info.size"),
                                size_text.as_deref().unwrap_or("-"),
                            );
                        }
                        crate::ui::views::file_tooltip::info_row(
                            ui,
                            &t!("file_info.date_modified"),
                            &crate::infrastructure::windows::format_date(modified_ts),
                        );
                        if let Some(created_ts) =
                            app.global_search.created_ts_for_index(source_index)
                        {
                            crate::ui::views::file_tooltip::info_row(
                                ui,
                                &t!("file_info.date_created"),
                                &crate::infrastructure::windows::format_date(created_ts),
                            );
                        }
                        render_tags(ui, app, tag_ids);
                    });
                })
        });
    if let Some(tooltip_response) = tooltip_response {
        crate::ui::video_overlay::register_rect(
            ui.ctx(),
            tooltip_id,
            tooltip_response.response.rect,
        );
    }
}

fn resolve_modified_timestamp(
    app: &mut ImageViewerApp,
    source_index: usize,
    full_path: &str,
) -> u64 {
    if let Some(&cached_ts) = app.global_search.metadata_cache.get(full_path) {
        return cached_ts;
    }
    if let Some(cached_ts) = app.global_search.sort_modified_ts_for_index(source_index) {
        return cached_ts;
    }
    if app
        .global_search
        .attach_tooltip_to_sort_metadata_request(full_path)
    {
        return 0;
    }
    if app
        .global_search
        .tooltip_metadata_inflight
        .contains(full_path)
    {
        return 0;
    }

    app.global_search
        .tooltip_metadata_inflight
        .insert(full_path.to_string());
    let _ = app
        .global_search
        .tooltip_sender
        .send(TooltipRequest::Metadata(full_path.to_string()));
    0
}

fn resolve_thumbnail(
    app: &mut ImageViewerApp,
    full_path: &str,
    is_directory: bool,
) -> Option<egui::TextureHandle> {
    if is_directory {
        return None;
    }
    let path = std::path::PathBuf::from(full_path);
    let is_media = path.extension().is_some_and(|ext| {
        crate::infrastructure::windows::is_media_extension(&ext.to_string_lossy())
    });
    if !is_media {
        return None;
    }
    if let Some(texture) = app.cache_manager.get_thumbnail(&path) {
        return Some(texture.clone());
    }
    if let Some(texture) = app.global_search.tooltip_texture_cache.get(full_path) {
        return Some(texture.clone());
    }
    if app
        .global_search
        .tooltip_thumbnail_inflight
        .contains(full_path)
    {
        return None;
    }

    app.global_search
        .tooltip_thumbnail_inflight
        .insert(full_path.to_string());
    let _ = app
        .global_search
        .tooltip_sender
        .send(TooltipRequest::Thumbnail(full_path.to_string()));
    None
}

fn render_tags(ui: &mut egui::Ui, app: &ImageViewerApp, tag_ids: &[i64]) {
    if tag_ids.is_empty() {
        return;
    }
    let dark_mode = ui.visuals().dark_mode;
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(110.0, 18.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(
                    egui::RichText::new(if tag_ids.len() == 1 {
                        t!("file_info.tag")
                    } else {
                        t!("file_info.tags")
                    })
                    .size(12.0)
                    .color(crate::ui::theme::secondary_text_color(dark_mode)),
                );
            },
        );
        for tag_id in tag_ids {
            if let Some(tag) = app.tag_definitions.get(tag_id) {
                let (tag_icon_rect, _) =
                    ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                crate::ui::tag_icon::paint_filled(
                    ui.painter(),
                    tag_icon_rect.center(),
                    9.0,
                    tag.color.to_color32(),
                );
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new(tag.name.clone())
                        .size(12.0)
                        .color(crate::ui::theme::text_color(dark_mode)),
                );
                ui.add_space(6.0);
            }
        }
    });
}
