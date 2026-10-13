//! Cleanup modal — review phase: category breakdown, per-item selection,
//! age filter and the confirmation path (including the two-step confirmation
//! required for permanent deletion).

use crate::app::state::ImageViewerApp;
use crate::domain::cleanup::{CleanupCategoryId, CleanupDeleteMode};
use eframe::egui::{self, Color32, Margin, RichText, Vec2};
use rust_i18n::t;
use std::path::PathBuf;

use crate::ui::theme;
use crate::ui::components::cleanup_modal::{modal_frame, secondary_button, title_label};

const REVIEW_MAX_HEIGHT: f32 = 360.0;
/// Items rendered per category in the review list. Everything beyond the cap
/// still participates in selection (via the category checkbox); the cap only
/// keeps the UI responsive on pathological temp folders.
const MAX_SHOWN_ITEMS_PER_CATEGORY: usize = 200;

fn category_label(category: CleanupCategoryId) -> String {
    match category {
        CleanupCategoryId::UserTemp => t!("cleanup.category.user_temp").to_string(),
        CleanupCategoryId::SystemTemp => t!("cleanup.category.system_temp").to_string(),
        CleanupCategoryId::DirectXShaderCache => {
            t!("cleanup.category.directx_shader_cache").to_string()
        }
        CleanupCategoryId::ErrorReports => t!("cleanup.category.error_reports").to_string(),
        CleanupCategoryId::CrashDumps => t!("cleanup.category.crash_dumps").to_string(),
        CleanupCategoryId::ExplorerThumbnailCache => {
            t!("cleanup.category.explorer_thumbnail_cache").to_string()
        }
    }
}

fn age_choice_label(hours: u64) -> String {
    match hours {
        1 => t!("cleanup.age_1h").to_string(),
        24 => t!("cleanup.age_24h").to_string(),
        168 => t!("cleanup.age_7d").to_string(),
        other => format!("{other} h"),
    }
}

/// One rendered row of the review list.
struct ReviewItem {
    path: PathBuf,
    name: String,
    date: String,
    size: u64,
}

#[derive(Default)]
struct ReviewActions {
    confirm: bool,
    close: bool,
    mode_changed: bool,
    rescan_age: Option<u64>,
    toggle_category: Option<(CleanupCategoryId, bool)>,
    toggle_item: Option<(PathBuf, bool)>,
    toggle_expand: Option<CleanupCategoryId>,
}

pub(crate) fn render_review(
    app: &mut ImageViewerApp,
    ctx: &egui::Context,
    screen_rect: egui::Rect,
) {
    // Group candidates by category up front (immutable snapshot).
    let mut grouped: Vec<(CleanupCategoryId, Vec<ReviewItem>)> = Vec::new();
    for category in CleanupCategoryId::ALL {
        let items: Vec<ReviewItem> = app
            .cleanup_state
            .candidates
            .iter()
            .filter(|candidate| candidate.category_id == category)
            .map(|candidate| ReviewItem {
                path: candidate.path.clone(),
                name: candidate
                    .path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| candidate.path.to_string_lossy().into_owned()),
                date: crate::infrastructure::windows::formatting::format_date(
                    candidate.modified_unix,
                ),
                size: candidate.size,
            })
            .collect();
        if !items.is_empty() {
            grouped.push((category, items));
        }
    }
    if grouped.is_empty() {
        render_review_empty(app, ctx, screen_rect);
        return;
    }

    let selected_count = app.cleanup_state.selected_count();
    let selected_bytes = app.cleanup_state.selected_bytes();
    let is_permanent = app.cleanup_state.delete_mode == CleanupDeleteMode::Permanent;
    let confirm_armed = app.cleanup_state.permanent_confirm_armed;

    let mut actions = ReviewActions::default();
    modal_frame(ctx, screen_rect, 460.0, |ui| {
        let dark_mode = ui.visuals().dark_mode;
        title_label(ui, t!("cleanup.review_title"), dark_mode);
        ui.label(
            RichText::new(t!("cleanup.recycle_bin_note").to_string())
                .size(12.0)
                .color(theme::secondary_text_color(dark_mode)),
        );
        ui.add_space(8.0);

        // Age filter row.
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(t!("cleanup.age_filter_label").to_string())
                    .size(12.0)
                    .color(theme::secondary_text_color(dark_mode)),
            );
            for choice in crate::app::cleanup_state::CLEANUP_AGE_CHOICES_HOURS {
                let selected = app.cleanup_state.age_hours == choice;
                if ui
                    .selectable_label(
                        selected,
                        RichText::new(age_choice_label(choice)).size(12.0),
                    )
                    .clicked()
                {
                    actions.rescan_age = Some(choice);
                }
            }
        });
        ui.add_space(6.0);

        // Category sections.
        egui::ScrollArea::vertical()
            .max_height(REVIEW_MAX_HEIGHT)
            .id_salt("cleanup_review_scroll")
            .show(ui, |ui| {
                for (category, items) in &grouped {
                    render_category_section(app, ui, dark_mode, *category, items, &mut actions);
                }
            });

        ui.add_space(10.0);

        // Permanent toggle (advanced).
        let mut permanent_checked = is_permanent;
        if ui
            .checkbox(&mut permanent_checked, t!("cleanup.permanent_toggle"))
            .changed()
        {
            actions.mode_changed = true;
        }
        if is_permanent {
            ui.label(
                RichText::new(t!("cleanup.permanent_warning").to_string())
                    .size(12.0)
                    .color(theme::COLOR_ACCENT),
            );
        }

        ui.add_space(12.0);

        // Footer: selection summary + buttons.
        let summary = t!(
            "cleanup.selected_summary",
            count = selected_count,
            size = crate::infrastructure::windows::formatting::format_size(selected_bytes)
        )
        .to_string();
        ui.label(
            RichText::new(summary)
                .size(13.0)
                .strong()
                .color(theme::text_color(dark_mode)),
        );
        ui.add_space(10.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let confirm_label = if is_permanent && !confirm_armed {
                t!("cleanup.permanent_confirm_button").to_string()
            } else {
                t!("cleanup.clean_button").to_string()
            };
            let confirm_fill = if is_permanent {
                Color32::from_rgb(170, 60, 60)
            } else {
                theme::COLOR_ACCENT
            };
            if ui
                .add_sized(
                    Vec2::new(190.0, 34.0),
                    egui::Button::new(
                        RichText::new(confirm_label)
                            .size(14.0)
                            .strong()
                            .color(Color32::WHITE),
                    )
                    .fill(confirm_fill),
                )
                .clicked()
            {
                actions.confirm = true;
            }
            ui.add_space(12.0);
            if secondary_button(ui, &t!("cleanup.cancel"), dark_mode) {
                actions.close = true;
            }
        });
    });

    apply_review_actions(app, actions, is_permanent, confirm_armed);
}

fn render_category_section(
    app: &mut ImageViewerApp,
    ui: &mut egui::Ui,
    dark_mode: bool,
    category: CleanupCategoryId,
    items: &[ReviewItem],
    actions: &mut ReviewActions,
) {
    let fully_selected = app.cleanup_state.category_is_fully_selected(category);
    let expanded = app.cleanup_state.is_category_expanded(category);
    let category_size: u64 = items.iter().map(|item| item.size).sum();

    ui.horizontal(|ui| {
        let mut checkbox = fully_selected;
        if ui.checkbox(&mut checkbox, "").changed() {
            actions.toggle_category = Some((category, checkbox));
        }
        let header = format!(
            "{}   {} · {} {}",
            category_label(category),
            items.len(),
            crate::infrastructure::windows::formatting::format_size(category_size),
            if expanded { "▾" } else { "▸" }
        );
        if ui
            .selectable_label(false, RichText::new(header).size(14.0).strong())
            .clicked()
        {
            actions.toggle_expand = Some(category);
        }
    });

    if !expanded {
        return;
    }

    egui::Frame::new()
        .inner_margin(Margin {
            left: 20,
            right: 0,
            top: 2,
            bottom: 2,
        })
        .show(ui, |ui| {
            for (index, item) in items.iter().enumerate() {
                if index >= MAX_SHOWN_ITEMS_PER_CATEGORY {
                    ui.label(
                        RichText::new(
                            t!(
                                "cleanup.more_items",
                                count = items.len() - MAX_SHOWN_ITEMS_PER_CATEGORY
                            )
                            .to_string(),
                        )
                        .size(12.0)
                        .color(theme::secondary_text_color(dark_mode))
                        .italics(),
                    );
                    break;
                }
                ui.horizontal(|ui| {
                    let mut checked = app.cleanup_state.selected.contains(&item.path);
                    if ui.checkbox(&mut checked, "").changed() {
                        actions.toggle_item = Some((item.path.clone(), checked));
                    }
                    ui.label(
                        RichText::new(item.name.clone())
                            .size(12.5)
                            .color(theme::text_color(dark_mode)),
                    )
                    .on_hover_text(item.path.to_string_lossy().to_string());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!(
                                "{} · {}",
                                crate::infrastructure::windows::formatting::format_size(item.size),
                                item.date
                            ))
                            .size(11.5)
                            .color(theme::secondary_text_color(dark_mode)),
                        );
                    });
                });
            }
        });
    ui.add_space(4.0);
}

fn render_review_empty(app: &mut ImageViewerApp, ctx: &egui::Context, screen_rect: egui::Rect) {
    let mut close = false;
    modal_frame(ctx, screen_rect, 150.0, |ui| {
        let dark_mode = ui.visuals().dark_mode;
        title_label(ui, t!("cleanup.title"), dark_mode);
        ui.label(
            RichText::new(t!("cleanup.no_candidates").to_string())
                .size(14.0)
                .color(theme::text_color(dark_mode)),
        );
        ui.add_space(14.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if secondary_button(ui, &t!("cleanup.close"), dark_mode) {
                close = true;
            }
        });
    });
    if close {
        app.close_cleanup_modal();
    }
}

fn apply_review_actions(
    app: &mut ImageViewerApp,
    actions: ReviewActions,
    was_permanent: bool,
    was_armed: bool,
) {
    if actions.close {
        app.close_cleanup_modal();
        return;
    }
    if let Some(hours) = actions.rescan_age {
        app.set_cleanup_age_hours(hours);
        return;
    }
    if actions.mode_changed {
        let new_mode = if was_permanent {
            CleanupDeleteMode::RecycleBin
        } else {
            CleanupDeleteMode::Permanent
        };
        app.set_cleanup_delete_mode(new_mode);
        app.cleanup_state.permanent_confirm_armed = false;
        return;
    }
    if let Some(category) = actions.toggle_expand {
        app.cleanup_state.toggle_category_expanded(category);
    }
    if let Some((category, selected)) = actions.toggle_category {
        app.cleanup_state.set_category_selected(category, selected);
    }
    if let Some((path, checked)) = actions.toggle_item {
        if checked {
            app.cleanup_state.selected.insert(path);
        } else {
            app.cleanup_state.selected.remove(&path);
        }
    }
    if actions.confirm {
        if was_permanent && !was_armed {
            app.cleanup_state.permanent_confirm_armed = true;
            return;
        }
        app.confirm_cleanup();
    }
}
