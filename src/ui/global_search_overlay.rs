//! Global search overlay modal (Spotlight-style).
//! Activated via Ctrl+Shift+F or the secondary toolbar button.

use crate::app::global_search_state::GlobalSearchCategory;
use crate::app::state::ImageViewerApp;
use crate::ui::theme;
use eframe::egui;
use filters::{category_label, format_number};
use rust_i18n::t;

mod actions;
pub(crate) mod filters;
mod result_row;
mod results_panel;
mod scrollbar;

const INITIAL_PAGE_LIMIT: u32 = 200;
const BACKDROP_ALPHA: u8 = 72;
const SEARCH_INPUT_DEBOUNCE_MS: u64 = 180;
const SEARCH_LOADING_TIMEOUT_MS: u64 = 12_000;

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
        && app.global_search.in_flight_started_at.is_some_and(|started_at| {
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

    if app.global_search.pending_query_dispatch_at.is_some_and(|deadline| {
        std::time::Instant::now() >= deadline && !app.global_search.query.is_empty()
    }) {
        app.global_search.selected_index = None;
        app.global_search.loading = true;
        app.global_search.results.clear();
        app.global_search.results_generation += 1;
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
            ui.visuals_mut().selection.stroke = egui::Stroke::new(0.0, theme::selection_text_color(dark_mode));
            ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
            ui.visuals_mut().widgets.hovered.bg_fill = hover_color;
            ui.visuals_mut().widgets.hovered.weak_bg_fill = hover_color;
            ui.visuals_mut().widgets.hovered.bg_stroke =
                egui::Stroke::new(1.0, theme::COLOR_ACCENT);
            ui.visuals_mut().widgets.active.bg_fill = hover_color;
            ui.visuals_mut().widgets.active.weak_bg_fill = hover_color;
            ui.visuals_mut().widgets.active.bg_stroke =
                egui::Stroke::new(1.0, theme::COLOR_ACCENT);

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
                        ui.label(egui::RichText::new(&*t!("search.title")).size(16.0).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if !app.global_search.available {
                                ui.label(
                                    egui::RichText::new(&*t!("search.service_offline"))
                                        .size(11.0)
                                        .color(egui::Color32::from_rgb(200, 80, 80)),
                                );
                            } else {
                                ui.label(
                                    egui::RichText::new(t!("search.indexed_files", count = format_number(app.global_search.total_indexed)))
                                    .size(11.0)
                                    .color(egui::Color32::from_gray(120)),
                                );
                            }
                        });
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
                            app.global_search.results.clear();
                            app.global_search.results_generation += 1;
                            app.global_search.selected_index = None;
                            app.global_search.has_more_results = false;
                            app.global_search.total_matches = None;
                            app.global_search.loading = false;
                            app.global_search.scroll_offset_y = 0.0;
                            app.global_search.last_scroll_offset_y = 0.0;
                            app.global_search.in_flight_query = None;
                            app.global_search.in_flight_started_at = None;
                            app.global_search.tooltip_texture_cache.clear();
                            app.global_search.metadata_cache.clear();
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
                        app.global_search.selected_index = None;
                        app.global_search.results.clear();
                        app.global_search.results_generation += 1;
                        app.global_search.loading = true;
                        app.global_search.has_more_results = false;
                        app.global_search.total_matches = None;
                        app.global_search.requested_offset = 0;
                        app.global_search.requested_limit = INITIAL_PAGE_LIMIT;
                        app.global_search.scroll_offset_y = 0.0;
                        app.global_search.last_scroll_offset_y = 0.0;
                        app.global_search.in_flight_query = None;
                        app.global_search.in_flight_started_at = None;
                        app.global_search.tooltip_texture_cache.clear();
                        app.global_search.metadata_cache.clear();
                        app.global_search.pending_query_dispatch_at = Some(
                            std::time::Instant::now()
                                + std::time::Duration::from_millis(SEARCH_INPUT_DEBOUNCE_MS),
                        );
                        ctx.request_repaint_after(std::time::Duration::from_millis(
                            SEARCH_INPUT_DEBOUNCE_MS,
                        ));
                    } else if app.global_search.query.is_empty() {
                        app.global_search.selected_index = None;
                        app.global_search.results.clear();
                        app.global_search.results_generation += 1;
                        app.global_search.loading = false;
                        app.global_search.has_more_results = false;
                        app.global_search.total_matches = None;
                        app.global_search.requested_offset = 0;
                        app.global_search.requested_limit = INITIAL_PAGE_LIMIT;
                        app.global_search.pending_query_dispatch_at = None;
                        app.global_search.scroll_offset_y = 0.0;
                        app.global_search.last_scroll_offset_y = 0.0;
                        app.global_search.in_flight_query = None;
                        app.global_search.in_flight_started_at = None;
                        app.global_search.tooltip_texture_cache.clear();
                        app.global_search.metadata_cache.clear();
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
    app.global_search.ensure_filter_cache();
    let drives = app.global_search.cached_available_drives.clone();
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
}
