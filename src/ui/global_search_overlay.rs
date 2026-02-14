//! Global search overlay modal (Spotlight-style).
//! Activated via Ctrl+Shift+F.

use crate::app::global_search_state::GlobalSearchCategory;
use crate::app::state::ImageViewerApp;
use eframe::egui;

const MAX_RESULTS: u32 = 200;
const BACKDROP_ALPHA: u8 = 72;
const RESULT_ROW_HEIGHT: f32 = 46.0;
const ICON_SIZE: f32 = 18.0;

/// Render the global search overlay. Returns true if the overlay should remain open.
pub fn render_global_search_overlay(app: &mut ImageViewerApp, ctx: &egui::Context) {
    if !app.global_search.active {
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
        .order(egui::Order::Middle)
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
                let popup_open = ctx.memory(|m| m.any_popup_open());
                if !popup_open {
                    if let Some(click_pos) = backdrop_resp.interact_pointer_pos() {
                        if !modal_rect.contains(click_pos) {
                            close_from_backdrop = true;
                        }
                    }
                }
            }
        });

    if close_from_backdrop {
        app.global_search.active = false;
        app.global_search.focus_request = false;
        return;
    }

    // ESC closes
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.global_search.active = false;
        app.global_search.focus_request = false;
        return;
    }

    // Render modal
    egui::Area::new(egui::Id::from("global_search_modal"))
        .fixed_pos(egui::pos2(modal_x, modal_y))
        .order(egui::Order::Foreground)
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
                            if !app.global_search.available {
                                ui.label(
                                    egui::RichText::new("Servico offline")
                                        .size(11.0)
                                        .color(egui::Color32::from_rgb(200, 80, 80)),
                                );
                            } else {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{} arquivos indexados",
                                        format_number(app.global_search.total_indexed)
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
                        egui::TextEdit::singleline(&mut app.global_search.query)
                            .hint_text("Digite para buscar em todo o computador...")
                            .font(egui::TextStyle::Body)
                            .id_source("global_search_input"),
                    );

                    // Auto-focus on open
                    if app.global_search.focus_request {
                        search_resp.request_focus();
                        app.global_search.focus_request = false;
                    }

                    // Trigger search on text change (with debounce)
                    if search_resp.changed() && !app.global_search.query.is_empty() {
                        app.global_search.selected_index = None;
                        app.global_search.loading = true;
                        if let Err(e) = app.global_search.sender.send(
                            crate::workers::global_search_worker::GlobalSearchRequest::Search {
                                query: app.global_search.query.clone(),
                                max_results: MAX_RESULTS,
                            },
                        ) {
                            app.global_search.loading = false;
                            log::error!("[GLOBAL-SEARCH] Failed to queue search request: {}", e);
                        }
                    } else if app.global_search.query.is_empty() {
                        app.global_search.selected_index = None;
                        app.global_search.results.clear();
                        app.global_search.loading = false;
                    }

                    ui.add_space(8.0);
                    render_filter_controls(ui, app);
                    ui.add_space(8.0);

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
                    } else if app.global_search.results.is_empty()
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
                    } else if !app.global_search.results.is_empty() && filtered_indices.is_empty() {
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
                    } else if !filtered_indices.is_empty() {
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

                        // Header with count
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

                        // Scrollable results list (fixed viewport height)
                        let mut activate_result: Option<(String, bool)> = None;
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), results_height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                egui::ScrollArea::vertical()
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        for &source_idx in &filtered_indices {
                                            let Some(result) =
                                                app.global_search.results.get(source_idx).cloned()
                                            else {
                                                continue;
                                            };
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
                                                .unwrap_or_else(|| "-".to_string());
                                            let meta_text =
                                                format!("{} | {}", file_type, size_text);

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
                                                ui.id().with(("global_search_row", source_idx)),
                                                egui::Sense::click(),
                                            );

                                            if row_resp.clicked() {
                                                app.global_search.selected_index = Some(source_idx);
                                            }

                                            let is_selected = app.global_search.selected_index
                                                == Some(source_idx);
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

                            if let Some(result) =
                                app.global_search.results.get(selected_idx).cloned()
                            {
                                activate_result = Some((result.full_path, result.is_dir));
                            }
                        }

                        if let Some((full_path, is_dir)) = activate_result {
                            activate_search_result(app, &full_path, is_dir);
                        }
                    } else if app.global_search.query.is_empty() {
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

fn render_filter_controls(ui: &mut egui::Ui, app: &mut ImageViewerApp) {
    let categories = [
        GlobalSearchCategory::All,
        GlobalSearchCategory::Files,
        GlobalSearchCategory::Folders,
        GlobalSearchCategory::Images,
        GlobalSearchCategory::Videos,
        GlobalSearchCategory::Audio,
        GlobalSearchCategory::Documents,
    ];

    let drives = available_drives(&app.global_search.results);
    if app
        .global_search
        .drive_filter
        .is_some_and(|drive| !drives.contains(&drive))
    {
        app.global_search.drive_filter = None;
        app.global_search.selected_index = None;
    }

    ui.horizontal(|ui| {
        let right_width = 190.0;
        let left_width = (ui.available_width() - right_width).max(120.0);

        ui.allocate_ui_with_layout(
            egui::vec2(left_width, 28.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(
                    egui::RichText::new("Filtros:")
                        .size(10.0)
                        .color(egui::Color32::from_gray(140)),
                );

                for category in categories {
                    let selected = app.global_search.category == category;
                    if ui
                        .selectable_label(selected, category_label(category))
                        .clicked()
                    {
                        app.global_search.category = category;
                        app.global_search.selected_index = None;
                    }
                }
            },
        );

        ui.allocate_ui_with_layout(
            egui::vec2(right_width, 28.0),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                egui::ComboBox::from_id_salt("global_search_drive_filter")
                    .width(120.0)
                    .selected_text(match app.global_search.drive_filter {
                        Some(drive) => format!("{}:\\", drive),
                        None => "Todos".to_string(),
                    })
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(app.global_search.drive_filter.is_none(), "Todos")
                            .clicked()
                        {
                            app.global_search.drive_filter = None;
                            app.global_search.selected_index = None;
                        }

                        for drive in &drives {
                            let selected = app.global_search.drive_filter == Some(*drive);
                            if ui
                                .selectable_label(selected, format!("{}:\\", drive))
                                .clicked()
                            {
                                app.global_search.drive_filter = Some(*drive);
                                app.global_search.selected_index = None;
                            }
                        }
                    });

                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("Drive:")
                        .size(10.0)
                        .color(egui::Color32::from_gray(140)),
                );
            },
        );

        if ui.available_width() > 0.0 {
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), 0.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |_| {},
            );
        }
    });
}

fn category_label(category: GlobalSearchCategory) -> &'static str {
    match category {
        GlobalSearchCategory::All => "Tudo",
        GlobalSearchCategory::Files => "Arquivos",
        GlobalSearchCategory::Folders => "Pastas",
        GlobalSearchCategory::Images => "Imagens",
        GlobalSearchCategory::Videos => "Videos",
        GlobalSearchCategory::Audio => "Audio",
        GlobalSearchCategory::Documents => "Documentos",
    }
}

fn build_filtered_indices(
    results: &[mtt_search_protocol::SearchResultItem],
    category: GlobalSearchCategory,
    drive_filter: Option<char>,
) -> Vec<usize> {
    let mut filtered = Vec::with_capacity(results.len());

    for (idx, result) in results.iter().enumerate() {
        if let Some(drive) = drive_filter {
            if extract_drive_letter(&result.full_path) != Some(drive) {
                continue;
            }
        }

        if matches_category(result, category) {
            filtered.push(idx);
        }
    }

    filtered
}

fn available_drives(results: &[mtt_search_protocol::SearchResultItem]) -> Vec<char> {
    let mut drives: Vec<char> = results
        .iter()
        .filter_map(|r| extract_drive_letter(&r.full_path))
        .collect();
    drives.sort_unstable();
    drives.dedup();
    drives
}

fn extract_drive_letter(path: &str) -> Option<char> {
    use std::path::{Component, Path, Prefix};

    // Accept regular and verbatim Windows paths:
    // - C:\foo
    // - \\?\C:\foo
    // - \\.\C:\foo
    if let Some(Component::Prefix(prefix_component)) = Path::new(path).components().next() {
        match prefix_component.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                return Some((letter as char).to_ascii_uppercase());
            }
            _ => {}
        }
    }

    // Fallback for uncommon string forms (e.g., slashes normalized by other layers).
    let normalized = path
        .strip_prefix(r"\\?\")
        .or_else(|| path.strip_prefix(r"\\.\"))
        .or_else(|| path.strip_prefix("//?/"))
        .or_else(|| path.strip_prefix("//./"))
        .or_else(|| path.strip_prefix(r"\??\"))
        .unwrap_or(path);

    let mut chars = normalized.chars();
    let drive = chars.next()?.to_ascii_uppercase();
    if drive.is_ascii_alphabetic() && chars.next() == Some(':') {
        return Some(drive);
    }

    None
}

fn matches_category(
    result: &mtt_search_protocol::SearchResultItem,
    category: GlobalSearchCategory,
) -> bool {
    match category {
        GlobalSearchCategory::All => true,
        GlobalSearchCategory::Files => !result.is_dir,
        GlobalSearchCategory::Folders => result.is_dir,
        GlobalSearchCategory::Images => extension_in(
            &result.full_path,
            &[
                "jpg", "jpeg", "png", "gif", "bmp", "webp", "tiff", "tif", "svg", "heic", "avif",
                "ico",
            ],
        ),
        GlobalSearchCategory::Videos => extension_in(
            &result.full_path,
            &[
                "mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v", "mpeg", "mpg", "ts",
            ],
        ),
        GlobalSearchCategory::Audio => extension_in(
            &result.full_path,
            &["mp3", "wav", "flac", "aac", "ogg", "wma", "m4a", "opus"],
        ),
        GlobalSearchCategory::Documents => extension_in(
            &result.full_path,
            &[
                "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "md", "rtf", "odt",
                "csv",
            ],
        ),
    }
}

fn extension_in(path: &str, allowed: &[&str]) -> bool {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());

    let Some(ext) = ext else {
        return false;
    };

    allowed.iter().any(|candidate| *candidate == ext)
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

    if let Some(cached) = app.global_search.size_cache.get(full_path) {
        return *cached;
    }

    // Avoid I/O for UNC paths; network metadata can block.
    let computed = if full_path.starts_with("\\\\") {
        None
    } else {
        std::fs::metadata(full_path).ok().map(|m| m.len())
    };
    app.global_search.size_cache
        .put(full_path.to_string(), computed);
    computed
}

#[cfg(test)]
mod tests {
    use super::extract_drive_letter;

    #[test]
    fn extract_drive_letter_accepts_regular_windows_path() {
        assert_eq!(extract_drive_letter(r"C:\Users\foo.txt"), Some('C'));
        assert_eq!(extract_drive_letter(r"z:\vault\file.docx"), Some('Z'));
    }

    #[test]
    fn extract_drive_letter_accepts_verbatim_windows_path() {
        assert_eq!(extract_drive_letter(r"\\?\D:\data\file.bin"), Some('D'));
        assert_eq!(extract_drive_letter(r"\\.\E:\media\movie.mkv"), Some('E'));
    }

    #[test]
    fn extract_drive_letter_rejects_non_drive_paths() {
        assert_eq!(extract_drive_letter(r"\\server\share\file.txt"), None);
        assert_eq!(extract_drive_letter(r"/home/user/file.txt"), None);
    }
}
