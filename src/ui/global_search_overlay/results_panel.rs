use crate::app::state::ImageViewerApp;
use eframe::egui;
use rust_i18n::t;

use super::actions::{self, ResultAction};
use super::result_row;
use super::scrollbar::{self, SCROLLBAR_GAP, SCROLLBAR_WIDTH};

const LOAD_MORE_STEP: u32 = 500;
const MAX_RESULTS_CAP: u32 = 10_000;
const SCROLL_SENSITIVITY: f32 = 5.0;
const RESULTS_FOOTER_HEIGHT: f32 = 32.0;
const ACTIVE_SCROLL_WINDOW_MS: u64 = 80;
const SCROLL_RENDER_OVERSCAN: usize = 1;

const RESULT_ROW_HEIGHT: f32 = result_row::ROW_HEIGHT;

pub(super) fn render_results_panel(
    ui: &mut egui::Ui,
    app: &mut ImageViewerApp,
    ctx: &egui::Context,
    modal_max_height: f32,
    hover_color: egui::Color32,
) {
    // Use cached filtered indices to avoid O(N) recomputation every frame.
    // Take ownership temporarily to avoid cloning; put back at the end.
    app.global_search.ensure_filter_cache();
    let filtered_indices = std::mem::take(&mut app.global_search.cached_filtered_indices);

    let shows_load_more = !app.global_search.query.is_empty()
        && app.global_search.has_more_results
        && !app.global_search.loading
        && (app.global_search.results.len() as u32) < MAX_RESULTS_CAP;
    let shows_max_reached = app.global_search.has_more_results
        && (app.global_search.results.len() as u32) >= MAX_RESULTS_CAP;
    let footer_height = if shows_load_more || shows_max_reached {
        RESULTS_FOOTER_HEIGHT
    } else {
        0.0
    };
    // Use modal_max_height as hard cap. 212 accounts for header+input+filters+spacing above.
    let panel_height = (modal_max_height - 212.0).max(200.0 + footer_height);
    let results_height = (panel_height - footer_height).max(200.0);

    if app.global_search.loading && app.global_search.results.is_empty() {
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), results_height),
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                ui.add_space(20.0);
                ui.label(t!("search.searching").to_string());
            },
        );
        return;
    }

    if app.global_search.results.is_empty()
        && !app.global_search.query.is_empty()
        && !app.global_search.loading
    {
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), results_height),
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                ui.add_space(20.0);
                ui.label(
                    egui::RichText::new(t!("search.no_results").to_string())
                        .color(egui::Color32::from_gray(120)),
                );
            },
        );
        return;
    }

    if !app.global_search.results.is_empty() && filtered_indices.is_empty() {
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), results_height),
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                ui.add_space(20.0);
                ui.label(
                    egui::RichText::new(t!("search.no_results_filtered").to_string())
                        .color(egui::Color32::from_gray(120)),
                );
            },
        );
        return;
    }

    if app.global_search.query.is_empty() {
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), results_height),
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                ui.add_space(20.0);
                ui.label(
                    egui::RichText::new(t!("search.esc_to_close").to_string())
                        .size(11.0)
                        .color(egui::Color32::from_gray(100)),
                );
            },
        );
        return;
    }

    if app
        .global_search
        .selected_index
        .is_some_and(|idx| idx >= app.global_search.results.len())
    {
        app.global_search.selected_index = None;
    }
    if app.global_search.selected_index.is_some_and(|idx| {
        !actions::filtered_contains(&filtered_indices, idx)
    }) {
        app.global_search.selected_index = None;
    }

    // Header with count.
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(t!("search.results_count", shown = filtered_indices.len(), total = app.global_search.results.len()).to_string())
                .size(11.0)
                .color(egui::Color32::from_gray(120)),
        );
        if app.global_search.loading {
            ui.label(
                egui::RichText::new(t!("search.searching").to_string())
                    .size(11.0)
                    .color(egui::Color32::from_gray(120)),
            );
        }
    });

    ui.add_space(4.0);

    // --- MANUAL VIRTUALIZATION (same approach as list view) ---
    let total_rows = filtered_indices.len();
    let total_content_height = total_rows as f32 * RESULT_ROW_HEIGHT;
    let viewport_h = results_height;

    // Keyboard navigation: Arrow Up/Down move selected_index within filtered results.
    {
        let arrow_down = ctx.input(|i| i.key_pressed(egui::Key::ArrowDown));
        let arrow_up = ctx.input(|i| i.key_pressed(egui::Key::ArrowUp));
        let page_down = ctx.input(|i| i.key_pressed(egui::Key::PageDown));
        let page_up = ctx.input(|i| i.key_pressed(egui::Key::PageUp));

        if (arrow_down || arrow_up || page_down || page_up) && !filtered_indices.is_empty() {
            let current_filtered_pos = app
                .global_search
                .selected_index
                .and_then(|sel| actions::filtered_position(&filtered_indices, sel));

            let page_step = ((viewport_h / RESULT_ROW_HEIGHT).floor() as usize).max(1);
            let new_filtered_pos = if arrow_down {
                match current_filtered_pos {
                    Some(pos) => (pos + 1).min(filtered_indices.len() - 1),
                    None => 0,
                }
            } else if arrow_up {
                match current_filtered_pos {
                    Some(pos) => pos.saturating_sub(1),
                    None => 0,
                }
            } else if page_down {
                match current_filtered_pos {
                    Some(pos) => (pos + page_step).min(filtered_indices.len() - 1),
                    None => 0,
                }
            } else {
                // page_up
                match current_filtered_pos {
                    Some(pos) => pos.saturating_sub(page_step),
                    None => 0,
                }
            };

            app.global_search.selected_index = Some(filtered_indices[new_filtered_pos]);

            // Auto-scroll to keep selected item visible.
            let item_top = new_filtered_pos as f32 * RESULT_ROW_HEIGHT;
            let item_bottom = item_top + RESULT_ROW_HEIGHT;
            let scroll = &mut app.global_search.scroll_offset_y;
            if item_top < *scroll {
                *scroll = item_top.max(0.0);
            } else if item_bottom > *scroll + viewport_h {
                let max_scroll = (total_content_height - viewport_h).max(0.0);
                *scroll = (item_bottom - viewport_h).clamp(0.0, max_scroll);
            }
        }
    }

    let mut activate_result: Option<ResultAction> = None;
    let panel_size = egui::vec2(ui.available_width(), panel_height);
    let (panel_rect, _) = ui.allocate_exact_size(panel_size, egui::Sense::hover());
    let viewport_rect = egui::Rect::from_min_max(
        panel_rect.min,
        egui::pos2(panel_rect.max.x, panel_rect.max.y - footer_height),
    );

    // Reserve space for the scrollbar so rows don't extend underneath it.
    let has_scrollbar = total_content_height > viewport_h;
    let available_w = if has_scrollbar {
        viewport_rect.width() - SCROLLBAR_WIDTH - SCROLLBAR_GAP - 2.0
    } else {
        viewport_rect.width()
    };

    // Mouse wheel scroll (same Ã—5 multiplier as list view).
    let pointer_over = ui
        .ctx()
        .pointer_hover_pos()
        .is_some_and(|pos| viewport_rect.contains(pos));
    if pointer_over {
        let delta = ui.input(|i| i.smooth_scroll_delta.y);
        if delta != 0.0 {
            app.global_search.scroll_offset_y -= delta * SCROLL_SENSITIVITY;
        }
    }

    // Clamp target scroll offset.
    let max_scroll = (total_content_height - viewport_h).max(0.0);
    app.global_search.scroll_offset_y = app.global_search.scroll_offset_y.clamp(0.0, max_scroll);

    if (app.global_search.scroll_offset_y - app.global_search.last_scroll_offset_y).abs() > 0.1 {
        app.global_search.last_scroll_time = std::time::Instant::now();
        app.global_search.last_scroll_offset_y = app.global_search.scroll_offset_y;
    }

    let (current_scroll, scroll_delta) = scrollbar::compute_visual_scroll(
        ui,
        app.global_search.scroll_offset_y,
        viewport_h,
        app.global_search.results_generation,
    );

    let has_recent_scroll_input = app.global_search.last_scroll_time.elapsed()
        < std::time::Duration::from_millis(ACTIVE_SCROLL_WINDOW_MS);
    let is_scrolling = has_recent_scroll_input || scroll_delta > 0.5;

    // Compute visible row range with adaptive overscan.
    let overscan: usize = if is_scrolling { 2 } else { 5 };
    let vis_min_row = ((current_scroll / RESULT_ROW_HEIGHT).floor() as usize).saturating_sub(overscan);
    let vis_max_row = (((current_scroll + viewport_h) / RESULT_ROW_HEIGHT).ceil() as usize) + overscan;
    let vis_max_row = vis_max_row.min(total_rows);
    let trim_rows = overscan.saturating_sub(SCROLL_RENDER_OVERSCAN);
    let tentative_render_min = if is_scrolling {
        vis_min_row.saturating_add(trim_rows)
    } else {
        vis_min_row
    };
    let tentative_render_max = if is_scrolling {
        vis_max_row.saturating_sub(trim_rows).min(total_rows)
    } else {
        vis_max_row
    };
    let (render_min_row, render_max_row) = if tentative_render_min < tentative_render_max {
        (tentative_render_min, tentative_render_max)
    } else {
        (vis_min_row, vis_max_row)
    };
    let mut icon_request_budget = if is_scrolling { 2usize } else { 6usize };
    let open_folder_label = t!("search.open_folder").to_string();
    let open_file_label = t!("search.open_file").to_string();

    // Clip child UI to viewport.
    let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(viewport_rect));
    child_ui.set_clip_rect(viewport_rect);

    let content_min = viewport_rect.min;

    // Full row path for both idle and scrolling states. The HDD stutter was
    // coming from metadata and icon churn, not from drawing the widgets.
    for i in render_min_row..render_max_row {
        let Some(&source_idx) = filtered_indices.get(i) else {
            continue;
        };

        let item_rect = egui::Rect::from_min_size(
            egui::pos2(
                content_min.x,
                content_min.y + (i as f32 * RESULT_ROW_HEIGHT) - current_scroll,
            ),
            egui::vec2(available_w, RESULT_ROW_HEIGHT),
        );

        result_row::render_result_row(
            &mut child_ui,
            app,
            ctx,
            source_idx,
            item_rect,
            hover_color,
            &mut icon_request_budget,
            &open_folder_label,
            &open_file_label,
            &mut activate_result,
        );
    }

    // Custom scrollbar (same as list view).
    if total_content_height > viewport_h && max_scroll > 0.0 {
        scrollbar::render_scrollbar(
            ui,
            viewport_rect,
            viewport_h,
            total_content_height,
            max_scroll,
            current_scroll,
            &mut app.global_search.scroll_offset_y,
        );
    }

    // Real pagination: request next page using offset/limit.
    if !app.global_search.query.is_empty() {
        if shows_load_more {
            let current_loaded = app.global_search.results.len() as u32;
            let next_offset = current_loaded;
            let next_limit = LOAD_MORE_STEP.min(MAX_RESULTS_CAP.saturating_sub(current_loaded));

            let footer_rect = egui::Rect::from_min_max(
                egui::pos2(panel_rect.min.x, panel_rect.max.y - footer_height),
                panel_rect.max,
            );
            let mut footer_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(footer_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            footer_ui.add_space(6.0);
            footer_ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(t!("search.results_loaded", count = current_loaded).to_string())
                        .size(10.0)
                        .color(egui::Color32::from_gray(120)),
                );
                if ui
                    .button(t!("search.load_more", count = next_limit).to_string())
                    .on_hover_text(t!("search.load_more_hint"))
                    .clicked()
                {
                    app.global_search.loading = true;
                    app.global_search.in_flight_query = Some(app.global_search.query.clone());
                    app.global_search.in_flight_started_at = Some(std::time::Instant::now());
                    app.global_search.has_more_results = false;
                    app.global_search.requested_offset = next_offset;
                    app.global_search.requested_limit = next_limit;

                    if let Err(e) = app.global_search.sender.send(
                        crate::workers::global_search_worker::GlobalSearchRequest::Search {
                            query: app.global_search.query.clone(),
                            offset: next_offset,
                            limit: next_limit,
                        },
                    ) {
                        app.global_search.loading = false;
                        app.global_search.in_flight_query = None;
                        app.global_search.in_flight_started_at = None;
                        log::error!("[GLOBAL-SEARCH] Failed to queue load-more request: {}", e);
                    }
                }
            });
        } else if shows_max_reached {
            let footer_rect = egui::Rect::from_min_max(
                egui::pos2(panel_rect.min.x, panel_rect.max.y - footer_height),
                panel_rect.max,
            );
            let mut footer_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(footer_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            footer_ui.add_space(6.0);
            footer_ui.label(
                egui::RichText::new(t!("search.max_reached", count = MAX_RESULTS_CAP).to_string())
                    .size(10.0)
                    .color(egui::Color32::from_gray(120)),
            );
        }
    }

    // Enter opens selected result (or the first visible one when none is selected).
    if activate_result.is_none()
        && ctx.input(|i| i.key_pressed(egui::Key::Enter))
        && !filtered_indices.is_empty()
    {
        let selected_idx = app
            .global_search
            .selected_index
            .filter(|idx| actions::filtered_contains(&filtered_indices, *idx))
            .unwrap_or(filtered_indices[0]);
        app.global_search.selected_index = Some(selected_idx);

        if let Some((full_path, is_dir)) = app
            .global_search
            .results
            .get(selected_idx)
            .map(|r| (r.full_path.clone(), r.is_dir))
        {
            activate_result = Some(ResultAction::OpenFolder(full_path, is_dir));
        }
    }

    // Restore the taken Vec so the cache is valid for the next frame.
    app.global_search.cached_filtered_indices = filtered_indices;

    if let Some(action) = activate_result {
        match action {
            ResultAction::OpenFile(full_path, is_dir) => {
                actions::open_file_with_default(app, &full_path, is_dir);
            }
            ResultAction::OpenFolder(full_path, is_dir) => {
                actions::activate_search_result(app, &full_path, is_dir);
            }
        }
    }
}

