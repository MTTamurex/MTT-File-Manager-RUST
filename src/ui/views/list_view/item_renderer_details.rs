//! Tooltip and icon rendering for list view items

use eframe::egui::{self, Color32, Pos2, Rect, RichText, Ui};
use rust_i18n::t;

use super::helpers::get_file_type_string;
use super::ListViewContext;
use super::ListViewOperations;
use crate::domain::file_entry::FileEntry;
use crate::infrastructure::windows::{format_date, format_size};

#[derive(Clone, Copy)]
struct TooltipLiveFileStat {
    checked_at: f64,
    size: u64,
}

fn resolve_tooltip_live_size(ui: &egui::Ui, item: &FileEntry, ctx: &mut ListViewContext) -> u64 {
    if item.is_dir {
        return item.size;
    }

    let now = ui.input(|i| i.time);
    let cache_id = egui::Id::new("tooltip_live_file_size").with(&item.path);
    let mut resolved = item.size;

    ui.ctx().data_mut(|d| {
        let mut state = d
            .get_temp::<TooltipLiveFileStat>(cache_id)
            .unwrap_or(TooltipLiveFileStat {
                checked_at: -10.0,
                size: item.size,
            });

        if (now - state.checked_at) >= 1.0 {
            state.size = crate::app::live_file_size::resolve_cached_or_enqueue_live_file_size(
                &item.path,
                item.modified,
                item.size,
                ctx.live_file_size_cache,
                ctx.live_file_size_loading,
                ctx.live_file_size_req_sender,
            );
            state.checked_at = now;
            d.insert_temp(cache_id, state);
        }

        resolved = state.size;
    });

    resolved
}

fn render_drive_tooltip(ui: &mut Ui, item: &FileEntry) {
    let Some(drive) = &item.drive_info else {
        return;
    };

    let file_system = if drive.file_system.is_empty() {
        "NTFS"
    } else {
        drive.file_system.as_str()
    };
    let used_space = drive.total_space.saturating_sub(drive.free_space);

    ui.horizontal(|ui| {
        ui.label(t!("file_info.type"));
        ui.label(format!("{:?}", drive.drive_type));
    });
    ui.horizontal(|ui| {
        ui.label(t!("file_info.used_space"));
        ui.label(format_size(used_space));
    });
    ui.horizontal(|ui| {
        ui.label(t!("file_info.free_space"));
        ui.label(format_size(drive.free_space));
    });
    ui.horizontal(|ui| {
        ui.label(t!("file_info.total_space"));
        ui.label(format_size(drive.total_space));
    });
    ui.horizontal(|ui| {
        ui.label(t!("file_info.filesystem"));
        ui.label(file_system);
    });
}

// PERFORMANCE: Tooltip debounce to avoid creation/destruction during scroll
use super::super::common::TOOLTIP_DELAY_SECS;

/// Renders tooltip with debounce for a list item
pub(super) fn render_item_tooltip(
    ui: &mut Ui,
    response: &egui::Response,
    item: &FileEntry,
    ctx: &mut ListViewContext,
    is_recycle_bin: bool,
) {
    if response.hovered() {
        let current_time = ui.input(|i| i.time);
        // PERF FIX: Use path-based hover ID so the tooltip timer resets when
        // navigating to a different folder (prevents stale timer triggering
        // immediate tooltip with blocking metadata call on cold cache).
        let hover_id = egui::Id::new("list_hover_start").with(&item.path);

        // Track hover start time using egui's memory
        let hover_start_time = ui
            .ctx()
            .data_mut(|d| *d.get_temp_mut_or_insert_with(hover_id, || current_time));

        let hover_duration = (current_time - hover_start_time) as f32;

        // Request repaint when approaching tooltip delay to ensure it appears
        if hover_duration < TOOLTIP_DELAY_SECS {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f32(
                    TOOLTIP_DELAY_SECS - hover_duration + 0.01,
                ));
        }

        // Only show tooltip if hover duration exceeds threshold
        if hover_duration >= TOOLTIP_DELAY_SECS {
          if let Some(mouse_pos) = ui.input(|i| i.pointer.hover_pos()) {
            // SMART TOOLTIP: Position to avoid video player overlay
            let screen_right = ui.ctx().screen_rect().right();
            let tooltip_width = 320.0;

            let effective_right = if ctx.is_video_docked_visible {
                screen_right * 0.72
            } else {
                screen_right
            };

            let tooltip_x = if mouse_pos.x + tooltip_width > effective_right {
                (effective_right - tooltip_width - 5.0).max(10.0)
            } else {
                mouse_pos.x
            };
            let tooltip_pos = egui::pos2(tooltip_x, mouse_pos.y);

            let tooltip_layer =
                egui::LayerId::new(egui::Order::Tooltip, response.id.with("tooltip"));
            egui::show_tooltip_at(
                ui.ctx(),
                tooltip_layer,
                response.id,
                tooltip_pos,
                |ui: &mut Ui| {
                    ui.set_max_width(300.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(crate::ui::components::item_slot::display_name_for_item(item).as_ref()).strong());
                        ui.separator();
                        if item.drive_info.is_some() {
                            render_drive_tooltip(ui, item);
                            return;
                        }
                        ui.horizontal(|ui| {
                            ui.label(t!("file_info.type"));
                            ui.label(get_file_type_string(item));
                        });
                        if !item.is_dir || item.is_archive() {
                            ui.horizontal(|ui| {
                                ui.label(t!("file_info.size"));
                                ui.label(format_size(resolve_tooltip_live_size(ui, item, ctx)));
                            });
                        }
                        let date_lbl = if is_recycle_bin {
                            t!("list_view.date_deleted")
                        } else {
                            t!("list_view.date_modified")
                        };
                        let date_val = if is_recycle_bin {
                            if item.modified > 0 {
                                format_date(item.modified)
                            } else {
                                item.deletion_date
                                    .clone()
                                    .unwrap_or_else(|| "-".to_string())
                            }
                        } else {
                            format_date(item.modified)
                        };
                        ui.horizontal(|ui| {
                            ui.label(date_lbl);
                            ui.label(":");
                            ui.label(date_val);
                        });
                    });
                },
            );
          } // if let Some(mouse_pos)
        }
    } else {
        // Clear hover time when not hovering
        let hover_id = egui::Id::new("list_hover_start").with(&item.path);
        ui.ctx().data_mut(|d| d.remove::<f64>(hover_id));
    }
}

/// Renders the item icon (drive, folder, or file)
pub(super) fn render_item_icon(
    ui: &mut Ui,
    item: &FileEntry,
    ctx: &mut ListViewContext,
    ops: &mut dyn ListViewOperations,
    rect: Rect,
    tint: Color32,
) {
    let icon_size_px = 16.0;
    let icon_rect = Rect::from_min_size(
        rect.min + egui::vec2(4.0, 4.0),
        egui::vec2(icon_size_px, icon_size_px),
    );

    if item.drive_info.is_some() {
        if let Some(drive_icon) = ctx
            .item_icon_loader
            .get_or_load_drive_icon(ui.ctx(), &item.path.to_string_lossy())
        {
            ui.painter().image(
                drive_icon.id(),
                icon_rect,
                Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                tint,
            );
        }
        return;
    }

    if item.is_dir && !item.is_archive() {
        let path_lower = item.path.to_string_lossy().to_lowercase();
        let is_virtual_archive =
            crate::domain::file_entry::path_contains_archive_segment(&path_lower);

        if is_virtual_archive {
            if let Some(folder_icon) =
                ctx.item_icon_loader
                    .get_or_load_icon(ui.ctx(), &item.path, true, false)
            {
                ui.painter().image(
                    folder_icon.id(),
                    icon_rect,
                    Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                    tint,
                );
                return;
            }
        }

        // Special folders (Documents, Pictures, Desktop, etc.) get their native
        // Windows icon via async extraction; regular folders get the generic icon.
        if crate::infrastructure::onedrive::is_special_icon_folder(&item.path) {
            if let Some(special_icon) = ctx.item_icon_loader
                .get_or_load_folder_path_icon(ui.ctx(), &item.path.to_string_lossy())
            {
                ui.painter().image(
                    special_icon.id(),
                    icon_rect,
                    Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                    tint,
                );
                return;
            }
        }

        if let Some(folder_icon) = ctx.folder_icon_texture {
            ui.painter().image(
                folder_icon.id(),
                icon_rect,
                Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                tint,
            );
        }
        return;
    }

    if item.is_archive() {
        if let Some(file_icon) =
            ctx.item_icon_loader
                .get_or_load_icon(ui.ctx(), &item.path, false, false)
        {
            ui.painter().image(
                file_icon.id(),
                icon_rect,
                Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                tint,
            );
        } else if !ctx.loading_icons.contains(&item.path)
            && ctx.failed_icons.peek(&item.path).is_none()
        {
            ops.request_icon_load(item.path.clone());
        }
        return;
    }

    if let Some(file_icon) = ctx
        .item_icon_loader
        .get_or_load_icon(ui.ctx(), &item.path, item.is_dir, false)
    {
        ui.painter().image(
            file_icon.id(),
            icon_rect,
            Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
            tint,
        );
    } else if !ctx.loading_icons.contains(&item.path) && ctx.failed_icons.peek(&item.path).is_none()
    {
        ops.request_icon_load(item.path.clone());
    }
}
