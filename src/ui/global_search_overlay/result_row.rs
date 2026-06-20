use crate::app::global_search_state::TooltipRequest;
use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::IconSize;
use crate::ui::theme;
use eframe::egui;
use rust_i18n::t;

use super::actions::{self, ResultAction};

pub(super) const ROW_HEIGHT: f32 = 46.0;
pub(super) const ICON_SIZE: f32 = 18.0;
use crate::ui::views::common::TOOLTIP_DELAY_SECS;
const ACTION_BTN_WIDTH: f32 = 52.0;
const ACTION_BTN_HEIGHT: f32 = 22.0;
const ACTION_BTN_GAP: f32 = 4.0;

#[inline]
fn cache_key_for_icon(path: &std::path::Path, size: IconSize) -> String {
    format!("{}_{:?}", path.to_string_lossy(), size)
}

#[inline]
fn lookup_icon_with_size_guard(
    app: &mut ImageViewerApp,
    ctx: &egui::Context,
    path: &std::path::Path,
    is_dir: bool,
) -> Option<egui::TextureHandle> {
    if let Some(icon) =
        app.item_icon_loader
            .get_or_load_icon_sized(ctx, path, IconSize::Large, is_dir, false)
    {
        return Some(icon);
    }

    let small_key = cache_key_for_icon(path, IconSize::Small);
    app.item_icon_loader.icon_cache.get(&small_key).cloned()
}

#[inline]
fn lookup_cached_icon_only(
    app: &mut ImageViewerApp,
    path: &std::path::Path,
    is_dir: bool,
) -> Option<egui::TextureHandle> {
    if !is_dir {
        if let Some(ext) = path.extension().and_then(|ext| ext.to_str()) {
            let ext_key = format!(
                "{}_{:?}",
                crate::infrastructure::windows::icons::canonical_icon_ext(
                    &ext.to_ascii_lowercase()
                ),
                IconSize::Large
            );
            if let Some(icon) = app.item_icon_loader.extension_cache.get(&ext_key) {
                return Some(icon.clone());
            }

            let small_ext_key = format!(
                "{}_{:?}",
                crate::infrastructure::windows::icons::canonical_icon_ext(
                    &ext.to_ascii_lowercase()
                ),
                IconSize::Small
            );
            if let Some(icon) = app.item_icon_loader.extension_cache.get(&small_ext_key) {
                return Some(icon.clone());
            }
        }
    }

    let large_key = cache_key_for_icon(path, IconSize::Large);
    if let Some(icon) = app.item_icon_loader.icon_cache.get(&large_key) {
        return Some(icon.clone());
    }

    let small_key = cache_key_for_icon(path, IconSize::Small);
    app.item_icon_loader.icon_cache.get(&small_key).cloned()
}

#[inline]
fn measure_text_width(
    ui: &egui::Ui,
    text: &str,
    font_id: &egui::FontId,
    color: egui::Color32,
) -> f32 {
    ui.fonts(|fonts| {
        fonts
            .layout_no_wrap(text.to_string(), font_id.clone(), color)
            .rect
            .width()
    })
}

fn paint_action_button(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id: egui::Id,
    label: &str,
) -> egui::Response {
    let response = ui.interact(rect, id, egui::Sense::click());
    let visuals = if response.is_pointer_button_down_on() {
        &ui.visuals().widgets.active
    } else if response.hovered() {
        &ui.visuals().widgets.hovered
    } else {
        &ui.visuals().widgets.inactive
    };

    ui.painter().rect_filled(rect, 4.0, visuals.bg_fill);
    ui.painter()
        .rect_stroke(rect, 4.0, visuals.bg_stroke, egui::StrokeKind::Inside);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(11.0),
        visuals.text_color(),
    );

    response
}

pub(super) fn render_result_row(
    ui: &mut egui::Ui,
    app: &mut ImageViewerApp,
    ctx: &egui::Context,
    source_idx: usize,
    row_rect: egui::Rect,
    hover_color: egui::Color32,
    icon_request_budget: &mut usize,
    open_folder_label: &str,
    open_file_label: &str,
    activate_result: &mut Option<ResultAction>,
) {
    let Some((full_path, result_name, is_dir, size)) =
        app.global_search.results.get(source_idx).map(|result| {
            (
                result.full_path.clone(),
                result.name.clone(),
                result.is_dir,
                result.size,
            )
        })
    else {
        return;
    };

    let row_resp = ui.interact(
        row_rect,
        ui.id().with(("global_search_row", source_idx)),
        egui::Sense::click(),
    );

    if row_resp.clicked() {
        app.global_search.selected_index = Some(source_idx);
    }

    let dark_mode = ui.visuals().dark_mode;
    let is_selected = app.global_search.selected_index == Some(source_idx);
    if is_selected {
        ui.painter()
            .rect_filled(row_rect, 4.0, theme::selection_color(dark_mode));
    } else if row_resp.hovered() {
        ui.painter().rect_filled(row_rect, 4.0, hover_color);
    }

    let separator_color = ui.visuals().widgets.noninteractive.bg_stroke.color;
    ui.painter().hline(
        row_rect.x_range(),
        row_rect.bottom(),
        egui::Stroke::new(1.0, separator_color),
    );

    let path = std::path::Path::new(&full_path);
    let row_has_priority = is_selected || row_resp.hovered();
    let row_may_spend_budget = *icon_request_budget > 0;
    let icon_tex = {
        let tex = if row_has_priority || row_may_spend_budget {
            lookup_icon_with_size_guard(app, ctx, path, is_dir)
        } else {
            lookup_cached_icon_only(app, path, is_dir)
        };
        if tex.is_none()
            && !is_dir
            && (row_has_priority || row_may_spend_budget)
            && !app.loading_icons.contains(path)
            && app.failed_icons.peek(path).is_none()
        {
            app.request_icon_load(path.to_path_buf());
            if !row_has_priority && row_may_spend_budget {
                *icon_request_budget = icon_request_budget.saturating_sub(1);
            }
        }
        tex
    };

    let file_type = actions::file_type_label(&full_path, is_dir);
    let meta_text = actions::format_result_meta(&file_type);
    let text_color = if is_selected {
        theme::selection_text_color(dark_mode)
    } else {
        ui.visuals().text_color()
    };
    let secondary_color = if is_selected {
        theme::selection_text_color(dark_mode)
    } else {
        egui::Color32::from_gray(120)
    };
    let meta_color = if is_selected {
        theme::selection_text_color(dark_mode)
    } else {
        egui::Color32::from_gray(140)
    };

    let content_rect = row_rect.shrink2(egui::vec2(8.0, 4.0));
    let button_size = egui::vec2(ACTION_BTN_WIDTH, ACTION_BTN_HEIGHT);
    let buttons_top = content_rect.center().y - button_size.y * 0.5;
    let folder_button_rect = egui::Rect::from_min_size(
        egui::pos2(content_rect.right() - button_size.x, buttons_top),
        button_size,
    );
    let open_button_rect = egui::Rect::from_min_size(
        egui::pos2(
            folder_button_rect.left() - ACTION_BTN_GAP - button_size.x,
            buttons_top,
        ),
        button_size,
    );

    let folder_button_resp = paint_action_button(
        ui,
        folder_button_rect,
        ui.id().with(("global_search_open_folder", source_idx)),
        open_folder_label,
    );
    if folder_button_resp.clicked() {
        *activate_result = Some(ResultAction::OpenFolder(full_path.clone(), is_dir));
    }

    let open_button_resp = paint_action_button(
        ui,
        open_button_rect,
        ui.id().with(("global_search_open_file", source_idx)),
        open_file_label,
    );
    if open_button_resp.clicked() {
        *activate_result = Some(ResultAction::OpenFile(full_path.clone(), is_dir));
    }

    let icon_rect = egui::Rect::from_min_size(
        egui::pos2(
            content_rect.left(),
            content_rect.center().y - ICON_SIZE * 0.5,
        ),
        egui::vec2(ICON_SIZE, ICON_SIZE),
    );
    if let Some(icon) = icon_tex {
        ui.painter().image(
            icon.id(),
            icon_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    }

    // Tag dot (mirrors list-view behavior: first tag's color, single dot
    // between the icon and the file name, with a 12 px name offset).
    // Collect into owned data so the immutable borrow of `app` does not
    // outlive this block.
    let tag_ids: Vec<i64> = {
        let path = std::path::Path::new(&full_path);
        crate::domain::file_tag::tag_ids_for_path(&app.tag_assignments, path)
            .map(|ids| ids.to_vec())
            .unwrap_or_default()
    };
    let first_tag_color: Option<egui::Color32> = tag_ids
        .iter()
        .find_map(|id| app.tag_definitions.get(id).map(|t| t.color.to_color32()));
    let tag_name_offset = if first_tag_color.is_some() { 12.0 } else { 0.0 };
    if let Some(color) = first_tag_color {
        ui.painter().circle_filled(
            egui::pos2(icon_rect.right() + 4.0, content_rect.center().y),
            3.2,
            color,
        );
    }

    let name_font = egui::FontId::proportional(13.0);
    let meta_font = egui::FontId::proportional(10.0);
    let text_left = icon_rect.right() + 8.0 + tag_name_offset;
    let text_right = open_button_rect.left() - 8.0;
    let text_max_w = (text_right - text_left).max(60.0);
    let display_name = crate::ui::views::list_view::truncate_text_for_column(
        &result_name,
        text_max_w,
        &name_font,
        ui,
    );
    ui.painter().text(
        egui::pos2(text_left, content_rect.top()),
        egui::Align2::LEFT_TOP,
        display_name,
        name_font.clone(),
        text_color,
    );

    let meta_y = (content_rect.bottom() - meta_font.size).max(content_rect.top() + 16.0);
    let meta_width = measure_text_width(ui, &meta_text, &meta_font, meta_color);
    ui.painter().text(
        egui::pos2(text_left, meta_y),
        egui::Align2::LEFT_TOP,
        &meta_text,
        meta_font.clone(),
        meta_color,
    );

    let path_left = text_left + meta_width + 6.0;
    let path_max_w = (text_right - path_left).max(0.0);
    if path_max_w > 8.0 {
        let display_path = crate::ui::views::list_view::truncate_text_for_column(
            &full_path, path_max_w, &meta_font, ui,
        );
        ui.painter().text(
            egui::pos2(path_left, meta_y),
            egui::Align2::LEFT_TOP,
            display_path,
            meta_font.clone(),
            secondary_color,
        );
    }

    // Tooltip with debounce.
    if row_resp.hovered() {
        let current_time = ui.input(|i| i.time);
        let hover_id = egui::Id::new("global_search_hover_start").with(&full_path);
        let hover_start_time = ui
            .ctx()
            .data_mut(|d| *d.get_temp_mut_or_insert_with(hover_id, || current_time));
        let hover_duration = (current_time - hover_start_time) as f32;

        if hover_duration < TOOLTIP_DELAY_SECS {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f32(
                    TOOLTIP_DELAY_SECS - hover_duration + 0.01,
                ));
        }

        if hover_duration >= TOOLTIP_DELAY_SECS {
            // --- Async metadata (P0-02): request background load on cache miss ---
            let modified_ts = if let Some(&cached_ts) =
                app.global_search.metadata_cache.get(&full_path)
            {
                cached_ts
            } else if let Some(cached_ts) = app.global_search.sort_modified_ts_for_index(source_idx)
            {
                cached_ts
            } else if app
                .global_search
                .attach_tooltip_to_sort_metadata_request(&full_path)
            {
                0
            } else if !app
                .global_search
                .tooltip_metadata_inflight
                .contains(&full_path)
            {
                app.global_search
                    .tooltip_metadata_inflight
                    .insert(full_path.clone());
                let _ = app
                    .global_search
                    .tooltip_sender
                    .send(TooltipRequest::Metadata(full_path.clone()));
                0
            } else {
                0
            };

            let size_opt = actions::resolve_result_size(app, &full_path, is_dir, size);
            let size_text = size_opt.map(crate::infrastructure::windows::format_size);

            // --- Async thumbnail (P0-03): request background decode on cache miss ---
            let thumb_tex: Option<egui::TextureHandle> = if !is_dir {
                let p = std::path::PathBuf::from(&full_path);
                let is_media = p
                    .extension()
                    .map(|ext| {
                        crate::infrastructure::windows::is_media_extension(&ext.to_string_lossy())
                    })
                    .unwrap_or(false);
                if is_media {
                    if let Some(tex) = app.cache_manager.get_thumbnail(&p) {
                        Some(tex.clone())
                    } else if let Some(tex) =
                        app.global_search.tooltip_texture_cache.get(&full_path)
                    {
                        Some(tex.clone())
                    } else if !app
                        .global_search
                        .tooltip_thumbnail_inflight
                        .contains(&full_path)
                    {
                        app.global_search
                            .tooltip_thumbnail_inflight
                            .insert(full_path.clone());
                        let _ = app
                            .global_search
                            .tooltip_sender
                            .send(TooltipRequest::Thumbnail(full_path.clone()));
                        None
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            if let Some(mouse_pos) = ui.input(|i| i.pointer.hover_pos()) {
                let tooltip_layer =
                    egui::LayerId::new(egui::Order::Tooltip, row_resp.id.with("tooltip"));
                egui::show_tooltip_at(
                    ui.ctx(),
                    tooltip_layer,
                    row_resp.id,
                    mouse_pos,
                    |ui: &mut egui::Ui| {
                        ui.set_max_width(300.0);
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(&result_name).strong());
                            ui.separator();
                            if let Some(tex) = &thumb_tex {
                                let tex_size = tex.size_vec2();
                                let max_w = 280.0_f32;
                                let max_h = 180.0_f32;
                                let scale = (max_w / tex_size.x).min(max_h / tex_size.y).min(1.0);
                                let display_size =
                                    egui::vec2(tex_size.x * scale, tex_size.y * scale);
                                ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                                    ui.add(egui::Image::new(tex).fit_to_exact_size(display_size));
                                });
                                ui.add_space(4.0);
                            }
                            ui.horizontal(|ui| {
                                ui.label(t!("file_info.type"));
                                ui.label(&file_type);
                            });
                            if !is_dir {
                                ui.horizontal(|ui| {
                                    ui.label(t!("file_info.size"));
                                    ui.label(size_text.as_deref().unwrap_or("-"));
                                });
                            }
                            ui.horizontal(|ui| {
                                ui.label(t!("file_info.date_modified"));
                                ui.label(crate::infrastructure::windows::format_date(modified_ts));
                            });
                            if let Some(created_ts) =
                                app.global_search.created_ts_for_index(source_idx)
                            {
                                ui.horizontal(|ui| {
                                    ui.label(t!("file_info.date_created"));
                                    ui.label(crate::infrastructure::windows::format_date(
                                        created_ts,
                                    ));
                                });
                            }
                            if !tag_ids.is_empty() {
                                ui.horizontal(|ui| {
                                    ui.label(if tag_ids.len() == 1 {
                                        t!("file_info.tag")
                                    } else {
                                        t!("file_info.tags")
                                    });
                                    for tag_id in &tag_ids {
                                        if let Some(tag) = app.tag_definitions.get(tag_id) {
                                            let color = tag.color.to_color32();
                                            let (dot_rect, _) = ui.allocate_exact_size(
                                                egui::vec2(10.0, 10.0),
                                                egui::Sense::hover(),
                                            );
                                            ui.painter().circle_filled(
                                                dot_rect.center(),
                                                3.5,
                                                color,
                                            );
                                            ui.label(&tag.name);
                                            ui.add_space(6.0);
                                        }
                                    }
                                });
                            }
                        });
                    },
                );
            } // if let Some(mouse_pos)
        }
    } else {
        let hover_id = egui::Id::new("global_search_hover_start").with(&full_path);
        ui.ctx().data_mut(|d| d.remove::<f64>(hover_id));
    }

    if row_resp.double_clicked() {
        *activate_result = Some(ResultAction::OpenFolder(full_path, is_dir));
    }
}
