use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::IconSize;
use crate::ui::theme;
use eframe::egui;

use super::actions::{self, ResultAction};

pub(super) const ROW_HEIGHT: f32 = 46.0;
pub(super) const ICON_SIZE: f32 = 18.0;
const ACTION_BTN_WIDTH: f32 = 52.0;
const ACTION_BTN_HEIGHT: f32 = 22.0;
const ACTION_BTN_GAP: f32 = 4.0;

mod interactions;
mod rename;
mod tooltip;

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

#[allow(clippy::too_many_arguments)]
pub(super) fn render_result_row(
    ui: &mut egui::Ui,
    app: &mut ImageViewerApp,
    ctx: &egui::Context,
    source_idx: usize,
    ordered_indices: &[usize],
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
        egui::Sense::click_and_drag(),
    );

    interactions::handle_selection_and_context_menu(
        app,
        ctx,
        &row_resp,
        source_idx,
        ordered_indices,
        row_rect,
        &full_path,
        is_dir,
    );

    let dark_mode = ui.visuals().dark_mode;
    let is_selected = app.global_search.selected_indices.contains(&source_idx);
    let is_renaming = app
        .global_search
        .rename_state
        .as_ref()
        .is_some_and(|rename| rename.source_index == source_idx);
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

    interactions::maybe_start_drag(
        app,
        ctx,
        &row_resp,
        source_idx,
        ordered_indices,
        is_dir,
        is_renaming,
        folder_button_rect,
        open_button_rect,
        icon_tex.clone(),
    );

    if !is_renaming {
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
        crate::domain::file_tag::tag_ids_for_path(app.tag_assignments_normalized.as_ref(), path)
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
    if is_renaming {
        let rename_rect = egui::Rect::from_min_size(
            egui::pos2(text_left, content_rect.top() - 2.0),
            egui::vec2(text_max_w, 22.0),
        );
        if let Some(rename_state) = app.global_search.rename_state.as_mut() {
            match rename::render(ui, rename_state, rename_rect, name_font.clone()) {
                rename::RenameInputAction::None => {}
                rename::RenameInputAction::Cancel => {
                    app.global_search.rename_state = None;
                    app.global_search.interaction_target =
                        crate::app::global_search_state::GlobalSearchInteractionTarget::Results;
                }
                rename::RenameInputAction::Commit(path, new_name) => {
                    app.global_search.rename_state = None;
                    app.global_search.interaction_target =
                        crate::app::global_search_state::GlobalSearchInteractionTarget::Results;
                    *activate_result = Some(ResultAction::CommitRename(path, new_name));
                }
            }
        }
    } else {
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
    }

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

    tooltip::render(
        ui,
        app,
        &row_resp,
        source_idx,
        &full_path,
        &result_name,
        is_dir,
        size,
        &file_type,
        &tag_ids,
        !is_renaming,
    );

    if row_resp.double_clicked() && !is_renaming {
        *activate_result = Some(ResultAction::OpenFolder(full_path, is_dir));
    }
}
