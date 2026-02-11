//! Global search overlay modal (Spotlight-style).
//! Activated via Ctrl+Shift+F.

use crate::app::state::ImageViewerApp;
use eframe::egui;

const MAX_RESULTS: u32 = 200;

/// Render the global search overlay. Returns true if the overlay should remain open.
pub fn render_global_search_overlay(app: &mut ImageViewerApp, ctx: &egui::Context) {
    if !app.global_search_active {
        return;
    }

    // Semi-transparent backdrop
    let screen_rect = ctx.screen_rect();
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::from("global_search_backdrop"),
    ));
    painter.rect_filled(
        screen_rect,
        0.0,
        egui::Color32::from_black_alpha(120),
    );

    // Click on backdrop closes the overlay
    let backdrop_resp = ctx.input(|i| {
        i.pointer
            .primary_clicked()
            .then(|| i.pointer.interact_pos())
            .flatten()
    });

    // Modal window dimensions
    let modal_width = (screen_rect.width() * 0.5).clamp(400.0, 800.0);
    let modal_max_height = screen_rect.height() * 0.6;
    let modal_x = (screen_rect.width() - modal_width) / 2.0;
    let modal_y = screen_rect.height() * 0.15;

    let modal_rect = egui::Rect::from_min_size(
        egui::pos2(modal_x, modal_y),
        egui::vec2(modal_width, modal_max_height),
    );

    // Check if click was outside modal
    if let Some(click_pos) = backdrop_resp {
        if !modal_rect.contains(click_pos) {
            app.global_search_active = false;
            return;
        }
    }

    // ESC closes
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.global_search_active = false;
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

                    // Header
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("Busca Global")
                                .size(16.0)
                                .strong(),
                        );
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
                    if search_resp.gained_focus()
                        || ctx.memory(|m| !m.has_focus(search_resp.id))
                    {
                        search_resp.request_focus();
                    }

                    // Trigger search on text change (with debounce)
                    if search_resp.changed() && !app.global_search_query.is_empty() {
                        app.global_search_loading = true;
                        let _ = app.global_search_sender.send(
                            crate::workers::global_search_worker::GlobalSearchRequest::Search {
                                query: app.global_search_query.clone(),
                                max_results: MAX_RESULTS,
                            },
                        );
                    } else if app.global_search_query.is_empty() {
                        app.global_search_results.clear();
                        app.global_search_loading = false;
                    }

                    ui.add_space(8.0);

                    // Results area
                    let results_height = (modal_max_height - 120.0).max(100.0);

                    if app.global_search_loading && app.global_search_results.is_empty() {
                        ui.vertical_centered(|ui| {
                            ui.add_space(20.0);
                            ui.spinner();
                            ui.label("Buscando...");
                        });
                    } else if app.global_search_results.is_empty()
                        && !app.global_search_query.is_empty()
                        && !app.global_search_loading
                    {
                        ui.vertical_centered(|ui| {
                            ui.add_space(20.0);
                            ui.label(
                                egui::RichText::new("Nenhum resultado encontrado")
                                    .color(egui::Color32::from_gray(120)),
                            );
                        });
                    } else if !app.global_search_results.is_empty() {
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

                        // Scrollable results list
                        egui::ScrollArea::vertical()
                            .max_height(results_height)
                            .show(ui, |ui| {
                                let mut navigate_to: Option<String> = None;

                                for result in &app.global_search_results {
                                    let is_dir = result.is_dir;
                                    let icon_str = if is_dir { "\u{1F4C1}" } else { "\u{1F4C4}" };

                                    let resp = ui
                                        .horizontal(|ui| {
                                            ui.set_min_height(28.0);

                                            // Icon
                                            ui.label(
                                                egui::RichText::new(icon_str).size(14.0),
                                            );

                                            ui.vertical(|ui| {
                                                // File name
                                                ui.label(
                                                    egui::RichText::new(&result.name)
                                                        .strong()
                                                        .size(13.0),
                                                );
                                                // Full path (smaller, gray)
                                                ui.label(
                                                    egui::RichText::new(&result.full_path)
                                                        .size(11.0)
                                                        .color(egui::Color32::from_gray(120)),
                                                );
                                            });
                                        })
                                        .response;

                                    // Double-click navigates to location
                                    if resp.double_clicked() {
                                        let path = std::path::Path::new(&result.full_path);
                                        if is_dir {
                                            navigate_to = Some(result.full_path.clone());
                                        } else if let Some(parent) = path.parent() {
                                            navigate_to =
                                                Some(parent.to_string_lossy().to_string());
                                        }
                                    }

                                    // Hover highlight
                                    if resp.hovered() {
                                        ui.painter().rect_filled(
                                            resp.rect,
                                            4.0,
                                            egui::Color32::from_white_alpha(10),
                                        );
                                    }

                                    ui.separator();
                                }

                                // Navigate after iteration (borrow checker)
                                if let Some(path) = navigate_to {
                                    app.global_search_active = false;
                                    app.navigate_to(&path);
                                }
                            });
                    } else if app.global_search_query.is_empty() {
                        ui.vertical_centered(|ui| {
                            ui.add_space(20.0);
                            ui.label(
                                egui::RichText::new("Ctrl+Shift+F para abrir/fechar")
                                    .size(11.0)
                                    .color(egui::Color32::from_gray(100)),
                            );
                        });
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
