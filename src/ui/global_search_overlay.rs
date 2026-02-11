//! Global search overlay modal (Spotlight-style).
//! Activated via Ctrl+Shift+F.

use crate::app::state::ImageViewerApp;
use eframe::egui;

const MAX_RESULTS: u32 = 200;
const BACKDROP_ALPHA: u8 = 72;
const RESULT_ROW_HEIGHT: f32 = 46.0;
const ICON_SIZE: f32 = 18.0;

/// Render the global search overlay. Returns true if the overlay should remain open.
pub fn render_global_search_overlay(app: &mut ImageViewerApp, ctx: &egui::Context) {
    if !app.global_search_active {
        return;
    }

    let screen_rect = ctx.screen_rect();

    // Modal window dimensions
    let modal_width = (screen_rect.width() * 0.40).clamp(360.0, 640.0);
    let modal_max_height = (screen_rect.height() * 0.72).clamp(400.0, 780.0);
    let modal_x = (screen_rect.width() - modal_width) / 2.0;
    let modal_y = ((screen_rect.height() - modal_max_height) * 0.5).max(8.0);

    let modal_rect = egui::Rect::from_min_size(
        egui::pos2(modal_x, modal_y),
        egui::vec2(modal_width, modal_max_height),
    );

    // Full-screen interaction blocker + lighter backdrop.
    // This prevents click/drag/scroll leakage to the main app while modal is open.
    let mut close_from_backdrop = false;
    egui::Area::new(egui::Id::from("global_search_backdrop_area"))
        .fixed_pos(screen_rect.min)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.set_min_size(screen_rect.size());

            let backdrop_rect = ui.max_rect();
            let backdrop_resp = ui.interact(
                backdrop_rect,
                ui.id().with("global_search_backdrop_interact"),
                egui::Sense::click_and_drag(),
            );

            ui.painter().rect_filled(
                backdrop_rect,
                0.0,
                egui::Color32::from_black_alpha(BACKDROP_ALPHA),
            );

            if backdrop_resp.clicked() {
                if let Some(click_pos) = backdrop_resp.interact_pointer_pos() {
                    if !modal_rect.contains(click_pos) {
                        close_from_backdrop = true;
                    }
                }
            }
        });

    if close_from_backdrop {
        app.global_search_active = false;
        return;
    }

    // ESC closes
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.global_search_active = false;
        return;
    }

    // Render modal
    egui::Area::new(egui::Id::from("global_search_modal"))
        .fixed_pos(egui::pos2(modal_x, modal_y))
        .order(egui::Order::Tooltip)
        .show(ctx, |ui| {
            egui::Frame::window(ui.style())
                .inner_margin(egui::Margin::same(16))
                .corner_radius(8.0)
                .shadow(egui::epaint::Shadow {
                    spread: 8,
                    blur: 16,
                    color: egui::Color32::from_black_alpha(60),
                    offset: [0, 4],
                })
                .show(ui, |ui| {
                    ui.set_width(modal_width - 32.0);
                    ui.set_min_height(modal_max_height - 32.0);

                    // Header
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Busca Global").size(16.0).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if !app.global_search_available {
                                ui.label(
                                    egui::RichText::new("Servico offline")
                                        .size(11.0)
                                        .color(egui::Color32::from_rgb(200, 80, 80)),
                                );
                            } else {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{} arquivos indexados",
                                        format_number(app.global_search_total_indexed)
                                    ))
                                    .size(11.0)
                                    .color(egui::Color32::from_gray(120)),
                                );
                            }
                        });
                    });

                    ui.add_space(8.0);

                    // Search input
                    let search_resp = ui.add_sized(
                        egui::vec2(ui.available_width(), 32.0),
                        egui::TextEdit::singleline(&mut app.global_search_query)
                            .hint_text("Digite para buscar em todo o computador...")
                            .font(egui::TextStyle::Body)
                            .id_source("global_search_input"),
                    );

                    // Auto-focus on open
                    if search_resp.gained_focus() || ctx.memory(|m| !m.has_focus(search_resp.id)) {
                        search_resp.request_focus();
                    }

                    // Trigger search on text change (with debounce)
                    if search_resp.changed() && !app.global_search_query.is_empty() {
                        app.global_search_selected_index = None;
                        app.global_search_loading = true;
                        if let Err(e) = app.global_search_sender.send(
                            crate::workers::global_search_worker::GlobalSearchRequest::Search {
                                query: app.global_search_query.clone(),
                                max_results: MAX_RESULTS,
                            },
                        ) {
                            app.global_search_loading = false;
                            eprintln!("[GLOBAL-SEARCH] Failed to queue search request: {}", e);
                        }
                    } else if app.global_search_query.is_empty() {
                        app.global_search_selected_index = None;
                        app.global_search_results.clear();
                        app.global_search_loading = false;
                    }

                    ui.add_space(8.0);

                    // Results area height is fixed from modal height to avoid dynamic growth.
                    let results_height = (modal_max_height - 172.0).max(220.0);

                    if app.global_search_loading && app.global_search_results.is_empty() {
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), results_height),
                            egui::Layout::top_down(egui::Align::Center),
                            |ui| {
                                ui.add_space(20.0);
                                ui.spinner();
                                ui.label("Buscando...");
                            },
                        );
                    } else if app.global_search_results.is_empty()
                        && !app.global_search_query.is_empty()
                        && !app.global_search_loading
                    {
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), results_height),
                            egui::Layout::top_down(egui::Align::Center),
                            |ui| {
                                ui.add_space(20.0);
                                ui.label(
                                    egui::RichText::new("Nenhum resultado encontrado")
                                        .color(egui::Color32::from_gray(120)),
                                );
                            },
                        );
                    } else if !app.global_search_results.is_empty() {
                        if app
                            .global_search_selected_index
                            .is_some_and(|idx| idx >= app.global_search_results.len())
                        {
                            app.global_search_selected_index = None;
                        }

                        // Header with count
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} resultados",
                                    app.global_search_results.len()
                                ))
                                .size(11.0)
                                .color(egui::Color32::from_gray(120)),
                            );
                            if app.global_search_loading {
                                ui.spinner();
                            }
                        });

                        ui.add_space(4.0);

                        // Scrollable results list (fixed viewport height)
                        let mut activate_result: Option<(String, bool)> = None;
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), results_height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                egui::ScrollArea::vertical()
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        let results = app.global_search_results.clone();
                                        for (row_idx, result) in results.iter().enumerate() {
                                            let path_buf =
                                                std::path::PathBuf::from(&result.full_path);
                                            let is_dir = result.is_dir;
                                            let file_type =
                                                file_type_label(&result.full_path, is_dir);
                                            let size_opt = resolve_result_size(
                                                app,
                                                &result.full_path,
                                                is_dir,
                                                result.size,
                                            );
                                            let size_text = size_opt
                                                .map(crate::infrastructure::windows::format_size)
                                                .unwrap_or_else(|| "—".to_string());
                                            let meta_text =
                                                format!("{} • {}", file_type, size_text);

                                            let icon_tex =
                                                app.item_icon_loader.get_or_load_icon_sized(
                                                    ctx,
                                                    &path_buf,
                                                    crate::domain::file_entry::IconSize::Small,
                                                    is_dir,
                                                    false,
                                                );
                                            // Request async extraction for unique-icon files
                                            // (.exe, .lnk, etc.) that aren't cached yet.
                                            // icon_cache LRU (512) is large enough to hold
                                            // all search results without thrashing.
                                            if icon_tex.is_none()
                                                && !is_dir
                                                && !app.loading_icons.contains(&path_buf)
                                                && app.failed_icons.peek(&path_buf).is_none()
                                            {
                                                app.request_icon_load(path_buf.clone());
                                            }

                                            let (row_rect, _) = ui.allocate_exact_size(
                                                egui::vec2(ui.available_width(), RESULT_ROW_HEIGHT),
                                                egui::Sense::hover(),
                                            );

                                            let row_resp = ui.interact(
                                                row_rect,
                                                ui.id().with(("global_search_row", row_idx)),
                                                egui::Sense::click(),
                                            );

                                            if row_resp.clicked() {
                                                app.global_search_selected_index = Some(row_idx);
                                            }

                                            let is_selected =
                                                app.global_search_selected_index == Some(row_idx);
                                            if is_selected {
                                                ui.painter().rect_filled(
                                                    row_rect,
                                                    4.0,
                                                    ui.style().visuals.selection.bg_fill,
                                                );
                                            } else if row_resp.hovered() {
                                                ui.painter().rect_filled(
                                                    row_rect,
                                                    4.0,
                                                    egui::Color32::from_white_alpha(12),
                                                );
                                            }

                                            let mut row_ui = ui.new_child(
                                                egui::UiBuilder::new()
                                                    .max_rect(
                                                        row_rect.shrink2(egui::vec2(8.0, 4.0)),
                                                    )
                                                    .layout(egui::Layout::left_to_right(
                                                        egui::Align::Center,
                                                    )),
                                            );
                                            row_ui.style_mut().interaction.selectable_labels =
                                                false;

                                            if let Some(icon) = icon_tex {
                                                row_ui
                                                    .add(egui::Image::new(&icon).max_size(
                                                        egui::vec2(ICON_SIZE, ICON_SIZE),
                                                    ));
                                            } else {
                                                let icon_str =
                                                    if is_dir { "\u{1F4C1}" } else { "\u{1F4C4}" };
                                                row_ui.label(
                                                    egui::RichText::new(icon_str).size(14.0),
                                                );
                                            }

                                            row_ui.add_space(8.0);
                                            row_ui.vertical(|ui| {
                                                ui.add(
                                                    egui::Label::new(
                                                        egui::RichText::new(&result.name)
                                                            .strong()
                                                            .size(13.0),
                                                    )
                                                    .truncate(),
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
                                                            egui::RichText::new(&result.full_path)
                                                                .size(10.0)
                                                                .color(egui::Color32::from_gray(
                                                                    120,
                                                                )),
                                                        )
                                                        .truncate(),
                                                    );
                                                });
                                            });

                                            // Double-click navigates to location
                                            if row_resp.double_clicked() {
                                                activate_result =
                                                    Some((result.full_path.clone(), is_dir));
                                            }

                                            ui.separator();
                                        }
                                    });
                            },
                        );

                        // Enter opens selected result (or the first one when none is selected).
                        if activate_result.is_none()
                            && ctx.input(|i| i.key_pressed(egui::Key::Enter))
                            && !app.global_search_results.is_empty()
                        {
                            let idx = app.global_search_selected_index.unwrap_or(0);
                            let idx = idx.min(app.global_search_results.len() - 1);
                            app.global_search_selected_index = Some(idx);

                            if let Some(result) = app.global_search_results.get(idx).cloned() {
                                activate_result = Some((result.full_path, result.is_dir));
                            }
                        }

                        if let Some((full_path, is_dir)) = activate_result {
                            activate_search_result(app, &full_path, is_dir);
                        }
                    } else if app.global_search_query.is_empty() {
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), results_height),
                            egui::Layout::top_down(egui::Align::Center),
                            |ui| {
                                ui.add_space(20.0);
                                ui.label(
                                    egui::RichText::new("Ctrl+Shift+F para abrir/fechar")
                                        .size(11.0)
                                        .color(egui::Color32::from_gray(100)),
                                );
                            },
                        );
                    }
                });
        });
}

fn format_number(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
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
    app.global_search_active = false;

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

    let current_norm = normalize_path_for_compare(&app.current_path);
    let destination_norm = normalize_path_for_compare(&parent_path);

    if current_norm == destination_norm {
        // Already in destination folder: select now.
        // If item list is stale, trigger a reload and pending_select_path
        // will apply after rebuild.
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

fn file_type_label(full_path: &str, is_dir: bool) -> String {
    if is_dir {
        return "Pasta".to_string();
    }

    let path = std::path::Path::new(full_path);
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        if !ext.is_empty() {
            return format!("Arquivo {}", ext.to_uppercase());
        }
    }

    "Arquivo".to_string()
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

    if let Some(cached) = app.global_search_size_cache.get(full_path) {
        return *cached;
    }

    // Avoid I/O for UNC paths; network metadata can block.
    let computed = if full_path.starts_with("\\\\") {
        None
    } else {
        std::fs::metadata(full_path).ok().map(|m| m.len())
    };
    app.global_search_size_cache
        .put(full_path.to_string(), computed);
    computed
}
