//! Global search overlay modal (Spotlight-style).
//! Activated via Ctrl+Shift+F.

use crate::app::global_search_state::GlobalSearchCategory;
use crate::app::state::ImageViewerApp;
use crate::ui::theme;
use eframe::egui;
use filters::{available_drives, category_label, format_number};

mod filters;
mod results_panel;

const INITIAL_PAGE_LIMIT: u32 = 200;
const BACKDROP_ALPHA: u8 = 72;
const SEARCH_INPUT_DEBOUNCE_MS: u64 = 180;

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
        app.global_search.pending_query_dispatch_at = None;
        return;
    }

    if app.global_search.pending_query_dispatch_at.is_some_and(|deadline| {
        std::time::Instant::now() >= deadline && !app.global_search.query.is_empty()
    }) {
        app.global_search.selected_index = None;
        app.global_search.loading = true;
        app.global_search.has_more_results = false;
        app.global_search.requested_offset = 0;
        app.global_search.requested_limit = INITIAL_PAGE_LIMIT;

        if let Err(e) = app.global_search.sender.send(
            crate::workers::global_search_worker::GlobalSearchRequest::Search {
                query: app.global_search.query.clone(),
                offset: app.global_search.requested_offset,
                limit: app.global_search.requested_limit,
            },
        ) {
            app.global_search.loading = false;
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
            ui.visuals_mut().selection.bg_fill = theme::COLOR_SELECTION;
            ui.visuals_mut().selection.stroke = egui::Stroke::new(0.0, theme::COLOR_SELECTION_TEXT);
            ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
            ui.visuals_mut().widgets.hovered.bg_fill = hover_color;
            ui.visuals_mut().widgets.hovered.weak_bg_fill = hover_color;
            ui.visuals_mut().widgets.hovered.bg_stroke = egui::Stroke::NONE;
            ui.visuals_mut().widgets.active.bg_fill = hover_color;
            ui.visuals_mut().widgets.active.weak_bg_fill = hover_color;
            ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::NONE;

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
                        app.global_search.has_more_results = false;
                        app.global_search.requested_offset = 0;
                        app.global_search.requested_limit = INITIAL_PAGE_LIMIT;
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
                        app.global_search.loading = false;
                        app.global_search.has_more_results = false;
                        app.global_search.requested_offset = 0;
                        app.global_search.requested_limit = INITIAL_PAGE_LIMIT;
                        app.global_search.pending_query_dispatch_at = None;
                    }

                    ui.add_space(8.0);
                    render_filter_controls(ui, app);
                    ui.add_space(8.0);

                    results_panel::render_results_panel(
                        ui,
                        app,
                        ctx,
                        modal_max_height,
                        hover_color,
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

    let drives = available_drives(&app.global_search.results);
    let drive_filter_row_count = (drives.len() + 1) as f32; // +1 for "Todos"
    let drive_filter_popup_height = drive_filter_row_count * 24.0;
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
                    .height(drive_filter_popup_height)
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
