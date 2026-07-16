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

    let tooltip_layer = egui::LayerId::new(egui::Order::Tooltip, response.id.with("tooltip"));
    egui::show_tooltip_at(
        ui.ctx(),
        tooltip_layer,
        response.id,
        mouse_pos,
        |ui: &mut egui::Ui| {
            ui.set_max_width(300.0);
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(result_name).strong());
                ui.separator();
                render_thumbnail(ui, thumbnail.as_ref());
                ui.horizontal(|ui| {
                    ui.label(t!("file_info.type"));
                    ui.label(file_type);
                });
                if !is_directory {
                    ui.horizontal(|ui| {
                        ui.label(t!("file_info.size"));
                        ui.label(size_text.as_deref().unwrap_or("-"));
                    });
                }
                ui.horizontal(|ui| {
                    ui.label(t!("file_info.date_modified"));
                    ui.label(crate::infrastructure::windows::format_date(modified_ts));
                });
                if let Some(created_ts) = app.global_search.created_ts_for_index(source_index) {
                    ui.horizontal(|ui| {
                        ui.label(t!("file_info.date_created"));
                        ui.label(crate::infrastructure::windows::format_date(created_ts));
                    });
                }
                render_tags(ui, app, tag_ids);
            });
        },
    );
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

fn render_thumbnail(ui: &mut egui::Ui, texture: Option<&egui::TextureHandle>) {
    let Some(texture) = texture else {
        return;
    };
    let texture_size = texture.size_vec2();
    let scale = (280.0 / texture_size.x)
        .min(180.0 / texture_size.y)
        .min(1.0);
    let display_size = egui::vec2(texture_size.x * scale, texture_size.y * scale);
    ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
        ui.add(egui::Image::new(texture).fit_to_exact_size(display_size));
    });
    ui.add_space(4.0);
}

fn render_tags(ui: &mut egui::Ui, app: &ImageViewerApp, tag_ids: &[i64]) {
    if tag_ids.is_empty() {
        return;
    }
    ui.horizontal(|ui| {
        ui.label(if tag_ids.len() == 1 {
            t!("file_info.tag")
        } else {
            t!("file_info.tags")
        });
        for tag_id in tag_ids {
            if let Some(tag) = app.tag_definitions.get(tag_id) {
                let (dot_rect, _) =
                    ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter()
                    .circle_filled(dot_rect.center(), 3.5, tag.color.to_color32());
                ui.label(&tag.name);
                ui.add_space(6.0);
            }
        }
    });
}
