//! Global search overlay modal (Spotlight-style).
//! Activated via Ctrl+Shift+F or the secondary toolbar button.

use crate::app::global_search_state::{GlobalSearchCategory, GlobalSearchSortMode, GlobalSearchTagFilter};
use crate::app::state::ImageViewerApp;
use crate::ui::theme;
use date_filter::{date_components_to_unix_ts, date_components_to_unix_ts_end_of_day};
use eframe::egui;
use filters::{category_label, format_exact_number};
use rust_i18n::t;
use std::time::Duration;

mod actions;
mod date_filter;
pub(crate) mod filters;
mod result_row;
mod results_panel;
mod scrollbar;

const INITIAL_PAGE_LIMIT: u32 = 500;
const BACKDROP_ALPHA: u8 = 72;
const SEARCH_INPUT_DEBOUNCE_MS: u64 = 180;
const SEARCH_LOADING_TIMEOUT_MS: u64 = 12_000;
const SERVICE_STARTUP_GRACE_SECS: u64 = 8;
const NO_PROGRESS_WARNING_SECS: u64 = 10;

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
        app.close_global_search();
        return;
    }

    // ESC closes
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.close_global_search();
        return;
    }

    if app.global_search.loading
        && app.global_search.pending_query_dispatch_at.is_none()
        && app
            .global_search
            .in_flight_started_at
            .is_some_and(|started_at| {
                started_at.elapsed() >= std::time::Duration::from_millis(SEARCH_LOADING_TIMEOUT_MS)
            })
    {
        log::warn!(
            "[GLOBAL-SEARCH] Search watchdog released stuck loading state for query='{}'",
            app.global_search.query
        );
        app.global_search.loading = false;
        app.global_search.in_flight_query = None;
        app.global_search.in_flight_started_at = None;
        app.global_search.has_more_results = false;
        app.global_search.total_matches = None;
        let _ = app
            .global_search
            .sender
            .send(crate::workers::global_search_worker::GlobalSearchRequest::CheckStatus);
    }

    if app
        .global_search
        .pending_query_dispatch_at
        .is_some_and(|deadline| {
            std::time::Instant::now() >= deadline && !app.global_search.query.is_empty()
        })
    {
        app.global_search.selected_index = None;
        app.global_search.loading = true;
        app.global_search.results.clear();
        app.global_search.results_generation += 1;
        app.global_search.service_results_loaded = 0;
        app.global_search.tagged_results_cache_key = None;
        app.global_search.has_more_results = false;
        app.global_search.total_matches = None;
        app.global_search.requested_offset = 0;
        app.global_search.requested_limit = INITIAL_PAGE_LIMIT;
        app.global_search.scroll_offset_y = 0.0;
        app.global_search.last_scroll_offset_y = 0.0;
        app.global_search.in_flight_query = Some(app.global_search.query.clone());
        app.global_search.in_flight_started_at = Some(std::time::Instant::now());

        if let Err(e) = app.global_search.sender.send(
            crate::workers::global_search_worker::GlobalSearchRequest::Search {
                query: app.global_search.query.clone(),
                offset: app.global_search.requested_offset,
                limit: app.global_search.requested_limit,
            },
        ) {
            app.global_search.loading = false;
            app.global_search.in_flight_query = None;
            app.global_search.in_flight_started_at = None;
            log::error!("[GLOBAL-SEARCH] Failed to queue search request: {}", e);
        }

        app.global_search.pending_query_dispatch_at = None;
    }

    // Render modal
    egui::Area::new(egui::Id::from("global_search_modal"))
        .fixed_pos(egui::pos2(modal_x, modal_y))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            let hover_color = if ui.visuals().dark_mode {
                theme::color_dark_hover()
            } else {
                theme::color_hover()
            };
            let dark_mode = ui.visuals().dark_mode;
            ui.visuals_mut().selection.bg_fill = theme::selection_color(dark_mode);
            ui.visuals_mut().selection.stroke =
                egui::Stroke::new(0.0, theme::selection_text_color(dark_mode));
            ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
            ui.visuals_mut().widgets.hovered.bg_fill = hover_color;
            ui.visuals_mut().widgets.hovered.weak_bg_fill = hover_color;
            ui.visuals_mut().widgets.hovered.bg_stroke =
                egui::Stroke::new(1.0, theme::COLOR_ACCENT);
            ui.visuals_mut().widgets.active.bg_fill = hover_color;
            ui.visuals_mut().widgets.active.weak_bg_fill = hover_color;
            ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::new(1.0, theme::COLOR_ACCENT);

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
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            egui::RichText::new(&*t!("search.title"))
                                .size(16.0)
                                .strong(),
                        );
                        ui.add_space(10.0);

                        if !app.global_search.available {
                            let service_starting = app.global_search.opened_at.elapsed()
                                < Duration::from_secs(SERVICE_STARTUP_GRACE_SECS);
                            ui.label(
                                egui::RichText::new(if service_starting {
                                    t!("search.service_starting")
                                } else {
                                    t!("search.service_offline")
                                })
                                .size(11.0)
                                .color(if service_starting {
                                    egui::Color32::from_gray(120)
                                } else {
                                    egui::Color32::from_rgb(200, 80, 80)
                                }),
                            );
                        } else if app.global_search.total_indexed == 0 {
                            ui.label(
                                egui::RichText::new(&*t!("search.index_preparing"))
                                    .size(11.0)
                                    .color(egui::Color32::from_gray(120)),
                            );
                        } else {
                            let headline = if app.global_search.indexing_in_progress {
                                t!(
                                    "search.indexing_files",
                                    count = format_exact_number(app.global_search.total_indexed)
                                )
                            } else {
                                t!(
                                    "search.indexed_files",
                                    count = format_exact_number(app.global_search.total_indexed)
                                )
                            };
                            ui.label(
                                egui::RichText::new(headline)
                                    .size(11.0)
                                    .color(egui::Color32::from_gray(120)),
                            );
                        }
                    });

                    ui.add_space(8.0);

                    // Search input (custom container with magnifying glass icon)
                    let input_height = 32.0;
                    let full_width = ui.available_width();
                    let (search_rect, container_resp) = ui.allocate_exact_size(
                        egui::vec2(full_width, input_height),
                        egui::Sense::click_and_drag(),
                    );

                    // Draw background and border
                    let visuals = ui.style().interact(&container_resp);
                    ui.painter().rect_filled(
                        search_rect,
                        visuals.corner_radius,
                        ui.visuals().widgets.inactive.bg_fill,
                    );
                    ui.painter().rect_stroke(
                        search_rect,
                        visuals.corner_radius,
                        ui.visuals().widgets.inactive.bg_stroke,
                        egui::StrokeKind::Inside,
                    );

                    let mut search_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(search_rect)
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    );

                    // Left padding + magnifying glass icon
                    search_ui.add_space(8.0);
                    crate::ui::svg_icons::icon_image(
                        &mut search_ui,
                        &mut app.svg_icon_manager,
                        "search",
                        16.0,
                    );
                    search_ui.add_space(6.0);

                    // Text input
                    let has_text = !app.global_search.query.is_empty();
                    let clear_btn_space = if has_text { 22.0 + 4.0 } else { 0.0 };
                    let text_available_w = search_ui.available_width() - 8.0 - clear_btn_space;
                    let hint = egui::RichText::new(t!("search.placeholder"))
                        .color(egui::Color32::from_gray(120));
                    let search_resp = search_ui.add_sized(
                        egui::vec2(text_available_w, input_height - 2.0),
                        egui::TextEdit::singleline(&mut app.global_search.query)
                            .frame(false)
                            .hint_text(hint)
                            .text_color(ui.visuals().text_color())
                            .vertical_align(egui::Align::Center)
                            .id_source("global_search_input"),
                    );

                    // Clear button (X)
                    if has_text {
                        if search_ui
                            .add(
                                egui::Button::new("✕")
                                    .frame(false)
                                    .min_size(egui::vec2(18.0, 18.0)),
                            )
                            .clicked()
                        {
                            app.global_search.query.clear();
                            app.global_search.release_transient_results();
                            app.global_search.clear_transient_caches();
                            app.global_search.loading = false;
                            app.global_search.scroll_offset_y = 0.0;
                            app.global_search.last_scroll_offset_y = 0.0;
                            app.global_search.in_flight_query = None;
                            app.global_search.in_flight_started_at = None;
                            search_resp.request_focus();
                        }
                    }

                    // Focus input when clicking empty container area
                    if container_resp.clicked() {
                        search_resp.request_focus();
                    }

                    // Auto-focus on open
                    if app.global_search.focus_request {
                        search_resp.request_focus();
                        app.global_search.focus_request = false;
                    }

                    // Trigger search on text change (with debounce)
                    if search_resp.changed() && !app.global_search.query.is_empty() {
                        app.global_search.clear_transient_results();
                        app.global_search.tooltip_texture_cache.clear();
                        app.global_search.metadata_cache.clear();
                        app.global_search.loading = true;
                        app.global_search.requested_offset = 0;
                        app.global_search.requested_limit = INITIAL_PAGE_LIMIT;
                        app.global_search.scroll_offset_y = 0.0;
                        app.global_search.last_scroll_offset_y = 0.0;
                        app.global_search.in_flight_query = None;
                        app.global_search.in_flight_started_at = None;
                        app.global_search.pending_query_dispatch_at = Some(
                            std::time::Instant::now()
                                + std::time::Duration::from_millis(SEARCH_INPUT_DEBOUNCE_MS),
                        );
                        ctx.request_repaint_after(std::time::Duration::from_millis(
                            SEARCH_INPUT_DEBOUNCE_MS,
                        ));
                    } else if app.global_search.query.is_empty() {
                        app.global_search.release_transient_results();
                        app.global_search.clear_transient_caches();
                        app.global_search.loading = false;
                        app.global_search.requested_offset = 0;
                        app.global_search.requested_limit = INITIAL_PAGE_LIMIT;
                        app.global_search.pending_query_dispatch_at = None;
                        app.global_search.scroll_offset_y = 0.0;
                        app.global_search.last_scroll_offset_y = 0.0;
                        app.global_search.in_flight_query = None;
                        app.global_search.in_flight_started_at = None;
                        app.global_search.min_size_mb = None;
                        app.global_search.max_size_mb = None;
                        app.global_search.created_after = None;
                        app.global_search.created_before = None;
                        app.global_search.created_after_month = 0;
                        app.global_search.created_after_day = 0;
                        app.global_search.created_after_year = 0;
                        app.global_search.created_after_month_text.clear();
                        app.global_search.created_after_day_text.clear();
                        app.global_search.created_after_year_text.clear();
                        app.global_search.created_before_month = 0;
                        app.global_search.created_before_day = 0;
                        app.global_search.created_before_year = 0;
                        app.global_search.created_before_month_text.clear();
                        app.global_search.created_before_day_text.clear();
                        app.global_search.created_before_year_text.clear();
                    }

                    if app.global_search.available && app.global_search.indexing_in_progress {
                        ui.add_space(8.0);
                        render_indexing_activity(ui, app, dark_mode);
                    }

                    ui.add_space(8.0);
                    render_filter_controls(ui, app);
                    ui.add_space(8.0);

                    results_panel::render_results_panel(
                        ui,
                        app,
                        ctx,
                        modal_max_height,
                        theme::selection_hover_color(dark_mode),
                    );
                });
        });
}

fn render_indexing_activity(ui: &mut egui::Ui, app: &ImageViewerApp, dark_mode: bool) {
    let idle_for = app.global_search.last_progress_advance_at.elapsed();
    let status_age = app.global_search.last_status_received_at.elapsed();
    let freshness_color = if idle_for >= Duration::from_secs(NO_PROGRESS_WARNING_SECS) {
        egui::Color32::from_rgb(196, 141, 56)
    } else {
        egui::Color32::from_gray(135)
    };
    let fill = if dark_mode {
        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 14)
    } else {
        egui::Color32::from_rgba_unmultiplied(0, 0, 0, 10)
    };
    let stroke = if idle_for >= Duration::from_secs(NO_PROGRESS_WARNING_SECS) {
        egui::Stroke::new(
            1.0,
            egui::Color32::from_rgba_unmultiplied(196, 141, 56, 120),
        )
    } else {
        egui::Stroke::new(
            1.0,
            egui::Color32::from_rgba_unmultiplied(120, 120, 120, 60),
        )
    };
    let status_color = if status_age >= Duration::from_secs(2) {
        egui::Color32::from_rgb(196, 141, 56)
    } else {
        egui::Color32::from_gray(135)
    };
    let freshness_text = if app.global_search.total_indexed == 0 {
        t!("search.indexing_waiting_first_batch")
    } else if idle_for < Duration::from_secs(2) {
        t!("search.indexing_recent_update")
    } else if idle_for < Duration::from_secs(NO_PROGRESS_WARNING_SECS) {
        t!(
            "search.indexing_waiting_next_update",
            seconds = idle_for.as_secs().max(1)
        )
    } else {
        t!(
            "search.indexing_waiting_progress",
            seconds = idle_for.as_secs().max(1)
        )
    };
    let status_text = if status_age < Duration::from_secs(2) {
        t!("search.indexing_status_recent")
    } else {
        t!(
            "search.indexing_status_age",
            seconds = status_age.as_secs().max(1)
        )
    };
    let activity_title = if app.global_search.total_indexed == 0 {
        t!("search.index_preparing")
    } else {
        t!(
            "search.indexing_files",
            count = format_exact_number(app.global_search.total_indexed)
        )
    };
    let summary = build_scanning_volume_summary(&app.global_search.status_volumes);

    egui::Frame::new()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(activity_title).size(11.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(freshness_text)
                            .size(10.0)
                            .color(freshness_color),
                    );
                });
            });

            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(status_text)
                    .size(10.0)
                    .color(status_color),
            );

            if !summary.is_empty() {
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(t!("search.indexing_drives", volumes = summary))
                        .size(10.0)
                        .color(egui::Color32::from_gray(145)),
                );
            }
        });
}

fn build_scanning_volume_summary(volumes: &[mtt_search_protocol::VolumeStatus]) -> String {
    volumes
        .iter()
        .filter(|volume| volume.state == "scanning")
        .map(format_scanning_volume)
        .collect::<Vec<_>>()
        .join(" • ")
}

fn format_scanning_volume(volume: &mtt_search_protocol::VolumeStatus) -> String {
    let phase = phase_label(&volume.phase);
    match volume.phase_total {
        Some(total) => format!(
            "{}: {} {}/{}",
            volume.drive_letter,
            phase,
            format_exact_number(volume.phase_progress.unwrap_or(0)),
            format_exact_number(total),
        ),
        None => format!(
            "{}: {} {}",
            volume.drive_letter,
            phase,
            format_exact_number(volume.phase_progress.unwrap_or(volume.files_indexed)),
        ),
    }
}

fn phase_label(phase: &str) -> String {
    match phase {
        "loading_cache" => t!("search.phase_loading_cache").to_string(),
        "catching_up" => t!("search.phase_catching_up").to_string(),
        "scanning_mft" => t!("search.phase_scanning_mft").to_string(),
        "loading_sizes" => t!("search.phase_loading_sizes").to_string(),
        "persisting" => t!("search.phase_persisting").to_string(),
        "filesystem_scan" => t!("search.phase_filesystem_scan").to_string(),
        "open_volume" | "query_journal" => t!("search.phase_starting").to_string(),
        _ => t!("search.phase_indexing").to_string(),
    }
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

    // Use cached drives to avoid O(N) recomputation every frame.
    app.global_search.ensure_filter_cache(&app.tag_assignments);
    let drives = app.global_search.cached_available_drives.clone();
    if app
        .global_search
        .drive_filter
        .is_some_and(|drive| !drives.contains(&drive))
    {
        app.global_search.drive_filter = None;
        app.global_search.selected_index = None;
    }

    // Row 1: category filters (left) + drive filter (right)
    ui.horizontal(|ui| {
        let right_width = 190.0;
        let left_width = (ui.available_width() - right_width).max(120.0);

        ui.allocate_ui_with_layout(
            egui::vec2(left_width, 28.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(
                    egui::RichText::new(t!("search.filters"))
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
                // ID includes drives so egui creates a fresh popup Area
                // (no cached size) whenever the drive list changes.
                egui::ComboBox::from_id_salt(
                    egui::Id::new("global_search_drive_filter").with(&drives),
                )
                .width(120.0)
                .height(800.0)
                .selected_text(match app.global_search.drive_filter {
                    Some(drive) => format!("{}:\\", drive),
                    None => t!("search.all_drives").to_string(),
                })
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(
                            app.global_search.drive_filter.is_none(),
                            t!("search.all_drives"),
                        )
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
                    egui::RichText::new(t!("search.drive_filter"))
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

    ui.add_space(4.0);

    // Row 2: sort + size + tag filter
    ui.horizontal(|ui| {
        let label_color = egui::Color32::from_gray(140);

        ui.label(
            egui::RichText::new(t!("search.sort_by"))
                .size(10.0)
                .color(label_color),
        );

        egui::ComboBox::from_id_salt("global_search_sort_mode")
            .width(110.0)
            .selected_text(match app.global_search.sort_mode {
                GlobalSearchSortMode::Relevance => t!("search.sort_relevance").to_string(),
                GlobalSearchSortMode::ModifiedDate => t!("search.sort_modified_date").to_string(),
                GlobalSearchSortMode::Name => t!("search.sort_name").to_string(),
            })
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(
                        app.global_search.sort_mode == GlobalSearchSortMode::Relevance,
                        t!("search.sort_relevance"),
                    )
                    .clicked()
                {
                    app.global_search.sort_mode = GlobalSearchSortMode::Relevance;
                    app.global_search.selected_index = None;
                }
                if ui
                    .selectable_label(
                        app.global_search.sort_mode == GlobalSearchSortMode::ModifiedDate,
                        t!("search.sort_modified_date"),
                    )
                    .clicked()
                {
                    app.global_search.sort_mode = GlobalSearchSortMode::ModifiedDate;
                    app.global_search.selected_index = None;
                }
                if ui
                    .selectable_label(
                        app.global_search.sort_mode == GlobalSearchSortMode::Name,
                        t!("search.sort_name"),
                    )
                    .clicked()
                {
                    app.global_search.sort_mode = GlobalSearchSortMode::Name;
                    app.global_search.selected_index = None;
                }
            });

        ui.add_space(4.0);

        // Sort direction toggle
        let dir_label = if app.global_search.sort_descending {
            "↓"
        } else {
            "↑"
        };
        if ui.button(dir_label).clicked() {
            app.global_search.sort_descending = !app.global_search.sort_descending;
            app.global_search.selected_index = None;
        }

        ui.add_space(8.0);

        // Min size (MB)
        ui.label(
            egui::RichText::new(t!("search.filter_min_size"))
                .size(10.0)
                .color(label_color),
        );
        let mut min_val: u64 = app.global_search.min_size_mb.unwrap_or(0);
        let min_resp = ui.add(
            egui::DragValue::new(&mut min_val)
                .range(0..=10_000_000u64)
                .speed(1)
                .suffix(" MB"),
        );
        if min_resp.changed() {
            app.global_search.min_size_mb = if min_val > 0 { Some(min_val) } else { None };
            app.global_search.selected_index = None;
        }

        ui.add_space(4.0);

        // Max size (MB)
        ui.label(
            egui::RichText::new(t!("search.filter_max_size"))
                .size(10.0)
                .color(label_color),
        );
        let mut max_val: u64 = app.global_search.max_size_mb.unwrap_or(0);
        let max_resp = ui.add(
            egui::DragValue::new(&mut max_val)
                .range(0..=10_000_000u64)
                .speed(1)
                .suffix(" MB"),
        );
        if max_resp.changed() {
            app.global_search.max_size_mb = if max_val > 0 { Some(max_val) } else { None };
            app.global_search.selected_index = None;
        }

        ui.add_space(8.0);

        // Tag filter: label + button, inline after the size controls.
        ui.label(
            egui::RichText::new(t!("search.filter_tag"))
                .size(10.0)
                .color(label_color),
        );
        ui.add_space(4.0);
        render_tag_filter_button(ui, app);
    });

    ui.add_space(4.0);

    // Row 3: created date filters
    ui.horizontal(|ui| {
        let label_color = egui::Color32::from_gray(140);

        // Helper to render a compact numeric text input with placeholder,
        // styled to match DragValue inputs (bordered frame, consistent padding).
        fn date_component_edit(
            ui: &mut egui::Ui,
            text: &mut String,
            value: &mut u32,
            hint: &str,
            width: f32,
        ) -> bool {
            let mut changed = false;
            let widget_visuals = ui.visuals().widgets.inactive;
            let text_color = ui.visuals().text_color();
            egui::Frame::NONE
                .fill(widget_visuals.bg_fill)
                .inner_margin(egui::Margin::symmetric(4, 2))
                .stroke(widget_visuals.bg_stroke)
                .corner_radius(widget_visuals.corner_radius)
                .show(ui, |ui| {
                    let resp = ui.add(
                        egui::TextEdit::singleline(text)
                            .hint_text(egui::RichText::new(hint).color(text_color))
                            .desired_width(width)
                            .margin(egui::Vec2::ZERO)
                            .frame(false),
                    );
                    if resp.changed() {
                        *text = text
                            .chars()
                            .filter(|c| c.is_ascii_digit())
                            .take(4)
                            .collect();
                        *value = text.parse().unwrap_or(0);
                        changed = true;
                    }
                });
            changed
        }

        // Created after
        ui.label(
            egui::RichText::new(t!("search.filter_created_after"))
                .size(10.0)
                .color(label_color),
        );
        let mut after_changed = false;
        after_changed |= date_component_edit(
            ui,
            &mut app.global_search.created_after_month_text,
            &mut app.global_search.created_after_month,
            "MM",
            28.0,
        );
        after_changed |= date_component_edit(
            ui,
            &mut app.global_search.created_after_day_text,
            &mut app.global_search.created_after_day,
            "DD",
            28.0,
        );
        after_changed |= date_component_edit(
            ui,
            &mut app.global_search.created_after_year_text,
            &mut app.global_search.created_after_year,
            "YYYY",
            40.0,
        );
        if after_changed {
            app.global_search.created_after = date_components_to_unix_ts(
                app.global_search.created_after_month,
                app.global_search.created_after_day,
                app.global_search.created_after_year,
            );
            app.global_search.selected_index = None;
        }

        ui.add_space(8.0);

        // Created before
        ui.label(
            egui::RichText::new(t!("search.filter_created_before"))
                .size(10.0)
                .color(label_color),
        );
        let mut before_changed = false;
        before_changed |= date_component_edit(
            ui,
            &mut app.global_search.created_before_month_text,
            &mut app.global_search.created_before_month,
            "MM",
            28.0,
        );
        before_changed |= date_component_edit(
            ui,
            &mut app.global_search.created_before_day_text,
            &mut app.global_search.created_before_day,
            "DD",
            28.0,
        );
        before_changed |= date_component_edit(
            ui,
            &mut app.global_search.created_before_year_text,
            &mut app.global_search.created_before_year,
            "YYYY",
            40.0,
        );
        if before_changed {
            app.global_search.created_before = date_components_to_unix_ts_end_of_day(
                app.global_search.created_before_month,
                app.global_search.created_before_day,
                app.global_search.created_before_year,
            );
            app.global_search.selected_index = None;
        }
    });
}

/// Fixed width of the tag filter popup (pixels).
const TAG_FILTER_POPUP_WIDTH: f32 = 220.0;

/// Renders the tag filter button (used inline within Row 2) and its popup.
///
/// The popup is a three-zone container:
/// 1. global modes (`All` / `Any tag`) — radio-style selection
/// 2. per-tag checkboxes for specific selections
/// 3. "Clear all" button when the filter is active
///
/// Visual feedback (the checkmark next to each selected option) is drawn
/// manually with `painter.text` because the global `visuals_mut()`
/// overrides applied to the modal suppress `selectable_label`'s built-in
/// checkmark, leaving selected options indistinguishable from the rest.
fn render_tag_filter_button(ui: &mut egui::Ui, app: &mut ImageViewerApp) {
    let button_label: String = match &app.global_search.tag_filter {
        GlobalSearchTagFilter::All => t!("search.filter_tag_all").to_string(),
        GlobalSearchTagFilter::Any => t!("search.filter_tag_any").to_string(),
        GlobalSearchTagFilter::Selected(ids) if ids.len() == 1 => app
            .tag_definitions
            .get(&ids[0])
            .map(|tag| tag.name.clone())
            .unwrap_or_else(|| t!("search.filter_tag_any").to_string()),
        GlobalSearchTagFilter::Selected(ids) => format!(
            "{} {}",
            ids.len(),
            t!("search.filter_tag_selected_suffix"),
        ),
    };

    let popup_id = egui::Id::new("global_search_tag_filter_popup");
    let mut show_popup = ui
        .ctx()
        .memory(|m| m.data.get_temp::<bool>(popup_id).unwrap_or(false));

    let button = egui::Button::new(button_label);
    let button_response = ui.add(button);
    let button_rect = button_response.rect;

    if button_response.clicked() {
        show_popup = !show_popup;
        ui.ctx()
            .memory_mut(|m| m.data.insert_temp::<bool>(popup_id, show_popup));
    }

    if !show_popup {
        return;
    }

    let mut close_popup = false;
    let mut item_clicked = false;
    let popup_pos = egui::pos2(button_rect.left(), button_rect.bottom() + 2.0);

    // Pre-compute the optimal popup width by measuring the widest label.
    // Each row has: 6px left pad + 14px leading column + 6px gap + text + 8px right pad.
    let font_id = egui::FontId::proportional(12.0);
    let leading_col = 6.0 + 14.0 + 6.0 + 8.0; // 34px fixed overhead per row
    let opt_popup_width = ui.fonts(|fonts| {
        let mut max_w = TAG_FILTER_POPUP_WIDTH;
        let all_w = fonts
            .layout_no_wrap(
                t!("search.filter_tag_all").to_string(),
                font_id.clone(),
                egui::Color32::WHITE,
            )
            .rect
            .width();
        max_w = max_w.max(all_w + leading_col);
        let any_w = fonts
            .layout_no_wrap(
                t!("search.filter_tag_any").to_string(),
                font_id.clone(),
                egui::Color32::WHITE,
            )
            .rect
            .width();
        max_w = max_w.max(any_w + leading_col);
        for tag in app.sorted_tag_definitions() {
            let tw = fonts
                .layout_no_wrap(tag.name.clone(), font_id.clone(), egui::Color32::WHITE)
                .rect
                .width();
            max_w = max_w.max(tw + leading_col);
        }
        if !matches!(app.global_search.tag_filter, GlobalSearchTagFilter::All) {
            let cw = fonts
                .layout_no_wrap(
                    t!("search.filter_tag_clear").to_string(),
                    font_id.clone(),
                    egui::Color32::WHITE,
                )
                .rect
                .width();
            max_w = max_w.max(cw + 16.0); // button padding
        }
        max_w
    });

    let popup_response = egui::Area::new(popup_id)
        .order(egui::Order::Foreground)
        .fixed_pos(popup_pos)
        .show(ui.ctx(), |ui| {
            // Override visuals for the popup body so selectable_label's
            // background matches the popup frame.
            let dark_mode = ui.visuals().dark_mode;
            let popup_fill = if dark_mode {
                egui::Color32::from_rgb(40, 40, 44)
            } else {
                egui::Color32::from_rgb(245, 245, 247)
            };
            ui.visuals_mut().widgets.inactive.bg_fill = popup_fill;
            ui.visuals_mut().widgets.hovered.bg_fill = if dark_mode {
                egui::Color32::from_rgb(58, 58, 62)
            } else {
                egui::Color32::from_rgb(225, 225, 230)
            };
            ui.visuals_mut().widgets.hovered.weak_bg_fill = ui.visuals().widgets.hovered.bg_fill;
            ui.visuals_mut().widgets.active.bg_fill = ui.visuals().widgets.hovered.bg_fill;
            ui.visuals_mut().widgets.active.weak_bg_fill = ui.visuals().widgets.hovered.bg_fill;
            ui.visuals_mut().selection.bg_fill = theme::selection_color(dark_mode);
            ui.visuals_mut().selection.stroke =
                egui::Stroke::new(1.0, theme::selection_text_color(dark_mode));

            egui::Frame::popup(ui.style())
                .inner_margin(egui::Margin::same(4))
                .show(ui, |ui| {
                    ui.set_min_width(opt_popup_width);
                    ui.set_max_width(opt_popup_width);
                    ui.spacing_mut().item_spacing.y = 2.0;

                    // --- Zone 1: global modes (radio) ---
                    let is_all = matches!(app.global_search.tag_filter, GlobalSearchTagFilter::All);
                    if popup_menu_item(ui, is_all, t!("search.filter_tag_all").as_ref(), None).clicked() {
                        app.global_search.tag_filter = GlobalSearchTagFilter::All;
                        app.global_search.selected_index = None;
                        close_popup = true;
                        item_clicked = true;
                    }
                    let is_any = matches!(app.global_search.tag_filter, GlobalSearchTagFilter::Any);
                    if popup_menu_item(ui, is_any, t!("search.filter_tag_any").as_ref(), None).clicked() {
                        app.global_search.tag_filter = GlobalSearchTagFilter::Any;
                        app.global_search.selected_index = None;
                        close_popup = true;
                        item_clicked = true;
                    }

                    ui.separator();

                    // --- Zone 2: per-tag checkboxes ---
                    if app.tag_definitions.is_empty() {
                        ui.add_space(2.0);
                        ui.weak(t!("search.filter_tag_no_tags"));
                    } else {
                        // Snapshot selected IDs to avoid borrow issues
                        // while iterating tag definitions.
                        let selected_ids: Vec<i64> = match &app.global_search.tag_filter {
                            GlobalSearchTagFilter::Selected(ids) => ids.clone(),
                            _ => Vec::new(),
                        };

                        for tag in app.sorted_tag_definitions() {
                            let is_selected = selected_ids.contains(&tag.id);
                            let dot_color = Some(tag.color.to_color32());
                            if popup_menu_item(ui, is_selected, &tag.name, dot_color).clicked() {
                                toggle_tag_in_filter(&mut app.global_search.tag_filter, tag.id);
                                app.global_search.selected_index = None;
                                item_clicked = true;
                            }
                        }
                    }

                    // --- Zone 3: explicit clear button ---
                    let has_active_filter = !matches!(
                        app.global_search.tag_filter,
                        GlobalSearchTagFilter::All
                    );
                    if has_active_filter {
                        ui.separator();
                        if ui
                            .button(t!("search.filter_tag_clear"))
                            .clicked()
                        {
                            app.global_search.tag_filter = GlobalSearchTagFilter::All;
                            app.global_search.selected_index = None;
                            close_popup = true;
                            item_clicked = true;
                        }
                    }
                });
        });

    if close_popup {
        ui.ctx()
            .memory_mut(|m| m.data.insert_temp::<bool>(popup_id, false));
    } else if !item_clicked && ui.ctx().input(|i| i.pointer.any_pressed()) {
        // Only run the outside-click check when no popup item consumed the
        // click. This prevents the fragile rect-based check from closing
        // the popup on the same frame a tag is clicked.
        if let Some(pointer_pos) = ui.ctx().input(|i| i.pointer.press_origin()) {
            let clicked_button = button_rect.contains(pointer_pos);
            let clicked_popup = popup_response.response.rect.contains(pointer_pos);
            if !clicked_button && !clicked_popup {
                ui.ctx()
                    .memory_mut(|m| m.data.insert_temp::<bool>(popup_id, false));
            }
        }
    }
}

/// Renders a single popup menu row with a manual checkmark (✓) prefix when
/// `selected` is `true`, plus an optional small colored dot (used for
/// per-tag rows). The whole row is clickable; hover and selection states
/// are painted manually for full control.
fn popup_menu_item(
    ui: &mut egui::Ui,
    selected: bool,
    label: &str,
    leading_color: Option<egui::Color32>,
) -> egui::Response {
    let row_height = 22.0;
    let desired_width = ui.available_width();
    let (row_rect, response) = ui.allocate_exact_size(
        egui::vec2(desired_width, row_height),
        egui::Sense::click(),
    );

    // Background: highlight on hover or when selected.
    if selected {
        let fill = ui.visuals().selection.bg_fill;
        ui.painter().rect_filled(row_rect, 4.0, fill);
    } else if response.hovered() {
        let fill = ui.visuals().widgets.hovered.bg_fill;
        ui.painter().rect_filled(row_rect, 4.0, fill);
    }

    // Leading column: optional colored dot OR checkmark for selected rows.
    let leading_size = 14.0_f32;
    let leading_rect = egui::Rect::from_min_size(
        egui::pos2(row_rect.left() + 6.0, row_rect.center().y - leading_size * 0.5),
        egui::vec2(leading_size, leading_size),
    );
    if let Some(color) = leading_color {
        ui.painter()
            .circle_filled(leading_rect.center(), 4.0, color);
    } else if selected {
        // Draw a checkmark using the selection's text color for contrast.
        let text_color = if ui.visuals().dark_mode {
            egui::Color32::WHITE
        } else {
            egui::Color32::BLACK
        };
        ui.painter().text(
            leading_rect.center(),
            egui::Align2::CENTER_CENTER,
            "✓",
            egui::FontId::proportional(12.0),
            text_color,
        );
    }

    // Label.
    let text_x = row_rect.left() + 6.0 + leading_size + 6.0;
    let text_color = if selected {
        theme::selection_text_color(ui.visuals().dark_mode)
    } else {
        ui.visuals().text_color()
    };
    ui.painter().text(
        egui::pos2(text_x, row_rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(12.0),
        text_color,
    );

    response
}

/// Toggles `tag_id` membership in the given filter, transitioning between
/// the three global states (`All` / `Any` / `Selected`) according to the
/// state-machine rules:
/// - From `All` or `Any` + click any tag  → `Selected([tag_id])`
/// - From `Selected(ids)` + click new tag  → `Selected(ids + [tag_id])`
/// - From `Selected(ids)` + click existing  → removes the tag; if `ids`
///   becomes empty, transitions back to `All`.
fn toggle_tag_in_filter(filter: &mut GlobalSearchTagFilter, tag_id: i64) {
    match filter {
        GlobalSearchTagFilter::All | GlobalSearchTagFilter::Any => {
            *filter = GlobalSearchTagFilter::Selected(vec![tag_id]);
        }
        GlobalSearchTagFilter::Selected(ids) => {
            if let Some(pos) = ids.iter().position(|id| *id == tag_id) {
                ids.remove(pos);
                if ids.is_empty() {
                    *filter = GlobalSearchTagFilter::All;
                }
            } else {
                ids.push(tag_id);
            }
        }
    }
}
