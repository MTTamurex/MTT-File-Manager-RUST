use crate::app::state::ImageViewerApp;
use crate::ui::global_search_overlay::filters::build_filtered_indices;
use crate::ui::theme;
use eframe::egui;

const RESULT_ROW_HEIGHT: f32 = 46.0;
const ICON_SIZE: f32 = 18.0;
const LOAD_MORE_STEP: u32 = 500;
const MAX_RESULTS_CAP: u32 = 10_000;

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

    // Results area height is fixed from modal height to avoid dynamic growth.
    let results_height = (modal_max_height - 212.0).max(200.0);

    if app.global_search.loading && app.global_search.results.is_empty() {
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), results_height),
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                ui.add_space(20.0);
                ui.spinner();
                ui.label("Buscando...");
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
                    egui::RichText::new("Nenhum resultado encontrado")
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
                    egui::RichText::new("Nenhum resultado com os filtros atuais")
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
                    egui::RichText::new("ESC para fechar")
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
        !filtered_indices
            .iter()
            .any(|filtered_idx| *filtered_idx == idx)
    }) {
        app.global_search.selected_index = None;
    }

    // Header with count.
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!(
                "{} resultados (de {})",
                filtered_indices.len(),
                app.global_search.results.len()
            ))
            .size(11.0)
            .color(egui::Color32::from_gray(120)),
        );
        if app.global_search.loading {
            ui.spinner();
        }
    });

    ui.add_space(4.0);

    // Scrollable results list (fixed viewport height).
    let mut activate_result: Option<(String, bool)> = None;
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), results_height),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for &source_idx in &filtered_indices {
                        let Some(result) = app.global_search.results.get(source_idx).cloned()
                        else {
                            continue;
                        };
                        let path_buf = std::path::PathBuf::from(&result.full_path);
                        let is_dir = result.is_dir;
                        let file_type = file_type_label(&result.full_path, is_dir);
                        let size_opt =
                            resolve_result_size(app, &result.full_path, is_dir, result.size);
                        let size_text = size_opt
                            .map(crate::infrastructure::windows::format_size)
                            .unwrap_or_else(|| "-".to_string());
                        let meta_text = format!("{} | {}", file_type, size_text);

                        let icon_tex = app.item_icon_loader.get_or_load_icon_sized(
                            ctx,
                            &path_buf,
                            crate::domain::file_entry::IconSize::Small,
                            is_dir,
                            false,
                        );
                        // Request async extraction for unique-icon files (.exe, .lnk, etc.).
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

                        let mut row_ui = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(row_rect.shrink2(egui::vec2(8.0, 4.0)))
                                .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        );
                        row_ui.style_mut().interaction.selectable_labels = false;

                        if let Some(icon) = icon_tex {
                            row_ui.add(
                                egui::Image::new(&icon).max_size(egui::vec2(ICON_SIZE, ICON_SIZE)),
                            );
                        }

                        row_ui.add_space(8.0);
                        row_ui.vertical(|ui| {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(&result.name).strong().size(13.0),
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
                                            .color(egui::Color32::from_gray(120)),
                                    )
                                    .truncate(),
                                );
                            });
                        });

                        // Double-click navigates to location.
                        if row_resp.double_clicked() {
                            activate_result = Some((result.full_path.clone(), is_dir));
                        }

                        ui.separator();
                    }
                });
        },
    );

    // Real pagination: request next page using offset/limit.
    if !app.global_search.query.is_empty() {
        if app.global_search.has_more_results
            && !app.global_search.loading
            && (app.global_search.results.len() as u32) < MAX_RESULTS_CAP
        {
            let current_loaded = app.global_search.results.len() as u32;
            let next_offset = current_loaded;
            let next_limit = LOAD_MORE_STEP.min(MAX_RESULTS_CAP.saturating_sub(current_loaded));

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("{} resultados carregados", current_loaded))
                        .size(10.0)
                        .color(egui::Color32::from_gray(120)),
                );
                if ui
                    .button(format!("Carregar mais (+{})", next_limit))
                    .on_hover_text("Busca a próxima página de resultados")
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
        } else if app.global_search.has_more_results
            && (app.global_search.results.len() as u32) >= MAX_RESULTS_CAP
        {
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(format!(
                    "Limite máximo atingido ({} resultados). Refine a busca para mais precisão.",
                    MAX_RESULTS_CAP
                ))
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

        if let Some(result) = app.global_search.results.get(selected_idx).cloned() {
            activate_result = Some((result.full_path, result.is_dir));
        }
    }

    if let Some((full_path, is_dir)) = activate_result {
        activate_search_result(app, &full_path, is_dir);
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

    if let Some(cached) = app.global_search.size_cache.get(full_path) {
        return *cached;
    }

    // Avoid I/O for UNC paths; network metadata can block.
    let computed = if full_path.starts_with("\\\\") {
        None
    } else {
        std::fs::metadata(full_path).ok().map(|m| m.len())
    };
    app.global_search
        .size_cache
        .put(full_path.to_string(), computed);
    computed
}
