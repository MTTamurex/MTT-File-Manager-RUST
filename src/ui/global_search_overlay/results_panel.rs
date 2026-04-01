use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::IconSize;
use crate::ui::global_search_overlay::filters::build_filtered_indices;
use crate::ui::theme;
use eframe::egui;
use rust_i18n::t;

const RESULT_ROW_HEIGHT: f32 = 46.0;
const ICON_SIZE: f32 = 18.0;
const LOAD_MORE_STEP: u32 = 500;
const MAX_RESULTS_CAP: u32 = 10_000;
const SCROLL_SENSITIVITY: f32 = 5.0;
const SCROLLBAR_WIDTH: f32 = 4.0;
const SCROLLBAR_MIN_HANDLE: f32 = 30.0;
const SCROLLBAR_GAP: f32 = 4.0;
const RESULTS_FOOTER_HEIGHT: f32 = 32.0;
const TOOLTIP_DELAY_SECS: f32 = 0.3;
const ACTION_BTN_WIDTH: f32 = 52.0;
const ACTION_BTN_HEIGHT: f32 = 22.0;
const ACTION_BTN_GAP: f32 = 4.0;

/// What the user wants to do with a search result.
enum ResultAction {
    /// Open the file with its default program (or navigate into if directory).
    OpenFile(String, bool),
    /// Navigate to the parent folder and select the item.
    OpenFolder(String, bool),
}

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
    // Primary path: request Large so it matches async worker cache key ("_Large").
    if let Some(icon) = app
        .item_icon_loader
        .get_or_load_icon_sized(ctx, path, IconSize::Large, is_dir, false)
    {
        return Some(icon);
    }

    // Safety guard: if another code path populated Small first, reuse it instead of
    // forcing an unnecessary async request (prevents hit-rate regressions).
    let small_key = cache_key_for_icon(path, IconSize::Small);
    app.item_icon_loader
        .icon_cache
        .get(&small_key)
        .cloned()
}

pub(super) fn render_results_panel(
    ui: &mut egui::Ui,
    app: &mut ImageViewerApp,
    ctx: &egui::Context,
    modal_max_height: f32,
    hover_color: egui::Color32,
) {
    let filtered_indices = build_filtered_indices(
        &app.global_search.results,
        app.global_search.category,
        app.global_search.drive_filter,
    );

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
        !filtered_indices.contains(&idx)
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
                .and_then(|sel| filtered_indices.iter().position(|&v| v == sel));

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

    // Mouse wheel scroll (same ×5 multiplier as list view).
    let pointer_over = ui
        .ctx()
        .pointer_hover_pos()
        .is_some_and(|pos| viewport_rect.contains(pos));
    if pointer_over {
        let delta = ui.input(|i| i.smooth_scroll_delta.y);
        if delta != 0.0 {
            app.global_search.scroll_offset_y -= delta * SCROLL_SENSITIVITY;
            app.global_search.last_scroll_time = std::time::Instant::now();
        }
    }

    // Clamp scroll offset.
    let max_scroll = (total_content_height - viewport_h).max(0.0);
    app.global_search.scroll_offset_y = app.global_search.scroll_offset_y.clamp(0.0, max_scroll);
    let current_scroll = app.global_search.scroll_offset_y;

    // Adaptive overscan: fewer rows during active scroll, more when idle.
    let is_scrolling = app.global_search.last_scroll_time.elapsed().as_millis() < 80;

    // Compute visible row range with adaptive overscan.
    let overscan: usize = if is_scrolling { 2 } else { 5 };
    let vis_min_row = ((current_scroll / RESULT_ROW_HEIGHT).floor() as usize).saturating_sub(overscan);
    let vis_max_row = (((current_scroll + viewport_h) / RESULT_ROW_HEIGHT).ceil() as usize) + overscan;
    let vis_max_row = vis_max_row.min(total_rows);

    // Clip child UI to viewport.
    let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(viewport_rect));
    child_ui.set_clip_rect(viewport_rect);

    let content_min = viewport_rect.min;

    for i in vis_min_row..vis_max_row {
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

        render_result_row(
            &mut child_ui,
            app,
            ctx,
            source_idx,
            item_rect,
            hover_color,
            &mut activate_result,
        );
    }

    // Custom scrollbar (same as list view).
    if total_content_height > viewport_h && max_scroll > 0.0 {
        render_scrollbar(
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
            .filter(|idx| filtered_indices.iter().any(|v| v == idx))
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

    if let Some(action) = activate_result {
        match action {
            ResultAction::OpenFile(full_path, is_dir) => {
                open_file_with_default(app, &full_path, is_dir);
            }
            ResultAction::OpenFolder(full_path, is_dir) => {
                activate_search_result(app, &full_path, is_dir);
            }
        }
    }
}

fn normalize_path_for_compare(path: &str) -> String {
    let lower = path.to_lowercase();
    let stripped = lower.strip_prefix(r"\\?\").unwrap_or(&lower);

    if stripped.len() > 3 {
        stripped.trim_end_matches('\\').to_string()
    } else {
        stripped.to_string()
    }
}

fn activate_search_result(app: &mut ImageViewerApp, full_path: &str, is_dir: bool) {
    app.global_search.active = false;
    app.global_search.focus_request = false;
    app.global_search.size_cache.clear();
    app.global_search.tooltip_texture_cache.clear();
    app.global_search.metadata_cache.clear();

    if is_dir {
        app.navigate_to(full_path);
        return;
    }

    let full_path_buf = std::path::PathBuf::from(full_path);
    let Some(parent) = full_path_buf.parent() else {
        app.navigate_to(full_path);
        return;
    };
    let parent_path = parent.to_string_lossy().to_string();

    app.pending_select_path = Some(full_path_buf.clone());

    let current_norm = normalize_path_for_compare(&app.navigation_state.current_path);
    let destination_norm = normalize_path_for_compare(&parent_path);

    if current_norm == destination_norm {
        // Already in destination folder: select now. If list is stale, trigger reload.
        if app.select_item_by_path(&full_path_buf) {
            app.pending_select_path = None;
        } else {
            app.loaded_path.clear();
            app.load_folder(false);
        }
    } else {
        app.navigate_to(&parent_path);
    }
}

/// Open a file with its default Windows program, or navigate into if directory.
fn open_file_with_default(app: &mut ImageViewerApp, full_path: &str, is_dir: bool) {
    app.global_search.active = false;
    app.global_search.focus_request = false;
    app.global_search.size_cache.clear();
    app.global_search.tooltip_texture_cache.clear();
    app.global_search.metadata_cache.clear();

    if is_dir {
        app.navigate_to(full_path);
    } else {
        let path = std::path::PathBuf::from(full_path);
        app.open_with_shell_guarded(&path);
    }
}

fn render_result_row(
    ui: &mut egui::Ui,
    app: &mut ImageViewerApp,
    ctx: &egui::Context,
    source_idx: usize,
    row_rect: egui::Rect,
    hover_color: egui::Color32,
    activate_result: &mut Option<ResultAction>,
) {
    let Some((full_path, result_name, is_dir, size)) = app
        .global_search
        .results
        .get(source_idx)
        .map(|result| {
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

    let path_buf = std::path::PathBuf::from(&full_path);
    let file_type = file_type_label(&full_path, is_dir);
    let size_opt = resolve_result_size(app, &full_path, is_dir, size);
    let size_text = size_opt
        .map(crate::infrastructure::windows::format_size)
        .unwrap_or_else(|| "-".to_string());
    let meta_text = format!("{} | {}", file_type, size_text);

    let icon_tex = lookup_icon_with_size_guard(app, ctx, &path_buf, is_dir);
    if icon_tex.is_none()
        && !is_dir
        && !app.loading_icons.contains(&path_buf)
        && app.failed_icons.peek(&path_buf).is_none()
    {
        app.request_icon_load(path_buf.clone());
    }

    let row_resp = ui.interact(
        row_rect,
        ui.id().with(("global_search_row", source_idx)),
        egui::Sense::click(),
    );

    if row_resp.clicked() {
        app.global_search.selected_index = Some(source_idx);
    }

    let is_selected = app.global_search.selected_index == Some(source_idx);
    if is_selected {
        ui.painter()
            .rect_filled(row_rect, 4.0, theme::COLOR_SELECTION);
    } else if row_resp.hovered() {
        ui.painter().rect_filled(row_rect, 4.0, hover_color);
    }

    // Separator line at bottom of row.
    let separator_color = ui.visuals().widgets.noninteractive.bg_stroke.color;
    ui.painter().hline(
        row_rect.x_range(),
        row_rect.bottom(),
        egui::Stroke::new(1.0, separator_color),
    );

    let mut row_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(row_rect.shrink2(egui::vec2(8.0, 4.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    row_ui.style_mut().interaction.selectable_labels = false;

    if let Some(icon) = icon_tex {
        row_ui.add(egui::Image::new(&icon).max_size(egui::vec2(ICON_SIZE, ICON_SIZE)));
    }

    row_ui.add_space(8.0);

    // Reserve space for action buttons on the right.
    // Include item_spacing.x to account for egui's internal spacing added before
    // the first widget in the right-to-left layout, which would otherwise cause
    // the truncated text (and its ellipsis) to visually overlap the buttons.
    let item_spacing_x = row_ui.spacing().item_spacing.x;
    let buttons_total_w = ACTION_BTN_WIDTH * 2.0 + ACTION_BTN_GAP + 4.0 + item_spacing_x;
    let text_max_w = (row_ui.available_width() - buttons_total_w).max(60.0);

    // File name + metadata (left side, truncated).
    row_ui.allocate_ui_with_layout(
        egui::vec2(text_max_w, row_rect.height() - 8.0),
        egui::Layout::top_down(egui::Align::LEFT),
        |ui| {
            ui.add(
                egui::Label::new(egui::RichText::new(&result_name).strong().size(13.0)).truncate(),
            );
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(&meta_text)
                        .size(10.0)
                        .color(egui::Color32::from_gray(140)),
                );
                ui.add_space(6.0);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&full_path)
                            .size(10.0)
                            .color(egui::Color32::from_gray(120)),
                    )
                    .truncate(),
                );
            });
        },
    );

    // Push buttons to the right.
    row_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let btn_size = egui::vec2(ACTION_BTN_WIDTH, ACTION_BTN_HEIGHT);

        // "Folder" button (rightmost).
        if ui
            .add_sized(
                btn_size,
                egui::Button::new(
                    egui::RichText::new(t!("search.open_folder")).size(11.0),
                ),
            )
            .clicked()
        {
            *activate_result = Some(ResultAction::OpenFolder(full_path.clone(), is_dir));
        }

        ui.add_space(ACTION_BTN_GAP);

        // "Open" button.
        if ui
            .add_sized(
                btn_size,
                egui::Button::new(
                    egui::RichText::new(t!("search.open_file")).size(11.0),
                ),
            )
            .clicked()
        {
            *activate_result = Some(ResultAction::OpenFile(full_path.clone(), is_dir));
        }
    });

    // Tooltip with debounce (same pattern as the main app list/grid views)
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
            // Grab cached thumbnail (if any) before entering the tooltip closure.
            // 1) Check in-memory texture cache first (cheap).
            // 2) Fall back to bounded tooltip texture cache (LRU-evicted).
            // 3) Fall back to SQLite disk cache — decode WebP and upload,
            //    stored in the bounded LRU so GPU textures are evicted properly.
            let thumb_tex: Option<egui::TextureHandle> = if !is_dir {
                let p = std::path::PathBuf::from(&full_path);
                let is_media = p
                    .extension()
                    .map(|ext| crate::infrastructure::windows::is_media_extension(&ext.to_string_lossy()))
                    .unwrap_or(false);
                if is_media {
                    if let Some(tex) = app.cache_manager.get_thumbnail(&p) {
                        Some(tex.clone())
                    } else if let Some(tex) = app.global_search.tooltip_texture_cache.get(&full_path) {
                        Some(tex.clone())
                    } else if let Some(entry) = app.disk_cache.get_latest(&p) {
                        if let Ok(img) = image::load_from_memory_with_format(
                            &entry.data,
                            image::ImageFormat::WebP,
                        ) {
                            let rgba = img.to_rgba8();
                            let size = [rgba.width() as usize, rgba.height() as usize];
                            let tex = ui.ctx().load_texture(
                                format!("gs_thumb_{}", full_path),
                                egui::ColorImage::from_rgba_unmultiplied(size, &rgba),
                                egui::TextureOptions::LINEAR,
                            );
                            app.global_search.tooltip_texture_cache.put(full_path.clone(), tex.clone());
                            Some(tex)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            // Resolve modified timestamp once, then serve from cache.
            // Also populate size_cache from the same metadata call to avoid
            // redundant fs::metadata I/O in resolve_result_size.
            let modified_ts = if let Some(&cached_ts) = app.global_search.metadata_cache.get(&full_path) {
                cached_ts
            } else {
                let meta = std::fs::metadata(&full_path).ok();
                let ts = meta
                    .as_ref()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                if !is_dir {
                    if let Some(len) = meta.as_ref().map(|m| m.len()) {
                        app.global_search.size_cache.put(full_path.clone(), Some(len));
                    }
                }
                app.global_search.metadata_cache.put(full_path.clone(), ts);
                ts
            };

            let tooltip_layer =
                egui::LayerId::new(egui::Order::Tooltip, row_resp.id.with("tooltip"));
            egui::show_tooltip_at(
                ui.ctx(),
                tooltip_layer,
                row_resp.id,
                ui.input(|i| i.pointer.hover_pos()).unwrap_or_default(),
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
                            let display_size = egui::vec2(tex_size.x * scale, tex_size.y * scale);
                            ui.with_layout(
                                egui::Layout::top_down(egui::Align::Center),
                                |ui| {
                                    ui.add(egui::Image::new(tex).fit_to_exact_size(display_size));
                                },
                            );
                            ui.add_space(4.0);
                        }
                        ui.horizontal(|ui| {
                            ui.label(t!("file_info.type"));
                            ui.label(&file_type);
                        });
                        if !is_dir {
                            ui.horizontal(|ui| {
                                ui.label(t!("file_info.size"));
                                ui.label(&size_text);
                            });
                        }
                        ui.horizontal(|ui| {
                            ui.label(t!("file_info.date_modified"));
                            ui.label(crate::infrastructure::windows::format_date(modified_ts));
                        });
                    });
                },
            );
        }
    } else {
        let hover_id = egui::Id::new("global_search_hover_start").with(&full_path);
        ui.ctx().data_mut(|d| d.remove::<f64>(hover_id));
    }

    if row_resp.double_clicked() {
        *activate_result = Some(ResultAction::OpenFolder(full_path, is_dir));
    }
}

/// Custom scrollbar with track-click and drag (matches list view behavior).
fn render_scrollbar(
    ui: &mut egui::Ui,
    viewport_rect: egui::Rect,
    viewport_h: f32,
    total_content_height: f32,
    max_scroll: f32,
    current_scroll: f32,
    scroll_offset: &mut f32,
) {
    let bar_rect = egui::Rect::from_min_max(
        egui::pos2(
            viewport_rect.right() - SCROLLBAR_WIDTH - 2.0,
            viewport_rect.top(),
        ),
        egui::pos2(viewport_rect.right() - 2.0, viewport_rect.bottom()),
    );

    let handle_h = (viewport_h / total_content_height * viewport_h)
        .max(SCROLLBAR_MIN_HANDLE)
        .min(viewport_h.max(SCROLLBAR_MIN_HANDLE));
    let travel = (viewport_h - handle_h).max(1.0);
    let handle_top = (current_scroll / max_scroll) * travel;
    let handle_rect = egui::Rect::from_min_size(
        egui::pos2(bar_rect.left(), viewport_rect.top() + handle_top),
        egui::vec2(SCROLLBAR_WIDTH, handle_h),
    );

    let scroll_id = ui.id().with("global_search_scrollbar");
    let response = ui.interact(bar_rect, scroll_id, egui::Sense::click_and_drag());

    if response.clicked() {
        if let Some(click_pos) = ui.input(|i| i.pointer.interact_pos()) {
            let relative_y = click_pos.y - bar_rect.top();
            let target_top = relative_y - (handle_h / 2.0);
            let ratio = target_top / travel;
            *scroll_offset = (ratio * max_scroll).clamp(0.0, max_scroll);
        }
    } else if response.dragged() {
        let delta = response.drag_delta().y;
        let scroll_per_pixel = max_scroll / travel;
        *scroll_offset += delta * scroll_per_pixel;
        *scroll_offset = scroll_offset.clamp(0.0, max_scroll);
    }

    // Draw track.
    ui.painter()
        .rect_filled(bar_rect, 0.0, egui::Color32::from_black_alpha(10));

    // Draw handle.
    let handle_color = if response.dragged() {
        egui::Color32::from_gray(100)
    } else if response.hovered() {
        egui::Color32::from_gray(150)
    } else {
        egui::Color32::from_gray(200)
    };
    ui.painter()
        .rect_filled(handle_rect, 2.0, handle_color);
}

fn file_type_label(full_path: &str, is_dir: bool) -> String {
    if is_dir {
        return rust_i18n::t!("search_results.folder").to_string();
    }

    let path = std::path::Path::new(full_path);
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        if !ext.is_empty() {
            return rust_i18n::t!("search_results.file_ext", ext = ext.to_uppercase()).to_string();
        }
    }

    rust_i18n::t!("search_results.file_generic").to_string()
}

fn resolve_result_size(
    app: &mut ImageViewerApp,
    full_path: &str,
    is_dir: bool,
    size: u64,
) -> Option<u64> {
    if is_dir {
        return None;
    }

    if size > 0 {
        return Some(size);
    }

    if let Some(cached) = app.global_search.size_cache.get(full_path) {
        return *cached;
    }

    // Fetch actual file size from disk when the search index doesn't have it.
    let resolved = std::fs::metadata(full_path).ok().map(|m| m.len());
    app.global_search
        .size_cache
        .put(full_path.to_string(), resolved);
    resolved
}
