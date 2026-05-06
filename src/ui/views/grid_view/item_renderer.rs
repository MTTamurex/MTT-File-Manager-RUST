use super::{GridViewContext, PendingOperations, TOOLTIP_DELAY_SECS};
use crate::domain::file_entry::FileEntry;
use eframe::egui::{self, Color32, Rect, Sense, Ui};
use rust_i18n::t;

#[derive(Clone, Copy)]
struct TooltipLiveFileStat {
    checked_at: f64,
    size: u64,
}

fn resolve_tooltip_live_size(ui: &Ui, item: &FileEntry, ctx: &mut GridViewContext) -> u64 {
    if item.is_dir {
        return item.size;
    }

    let now = ui.input(|i| i.time);
    let cache_id = egui::Id::new("grid_tooltip_live_file_size").with(&item.path);
    let mut resolved = item.size;

    ui.ctx().data_mut(|d| {
        let mut state =
            d.get_temp::<TooltipLiveFileStat>(cache_id)
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
        ui.label(&*t!("file_info.type"));
        ui.label(format!("{:?}", drive.drive_type));
    });
    ui.horizontal(|ui| {
        ui.label(&*t!("file_info.used_space"));
        ui.label(crate::infrastructure::windows::format_size(used_space));
    });
    ui.horizontal(|ui| {
        ui.label(&*t!("file_info.free_space"));
        ui.label(crate::infrastructure::windows::format_size(
            drive.free_space,
        ));
    });
    ui.horizontal(|ui| {
        ui.label(&*t!("file_info.total_space"));
        ui.label(crate::infrastructure::windows::format_size(
            drive.total_space,
        ));
    });
    ui.horizontal(|ui| {
        ui.label(&*t!("file_info.filesystem"));
        ui.label(file_system);
    });
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_grid_item(
    ui: &mut Ui,
    index: usize,
    item: &FileEntry,
    rect: Rect,
    ctx: &mut GridViewContext,
    clicked_item: &mut Option<usize>,
    double_clicked_item: &mut Option<usize>,
    secondary_clicked_item: &mut Option<usize>,
    empty_area_clicked: &mut bool,
    is_scrolling: bool,
) {
    let response = ui.interact(rect, ui.id().with(index), Sense::click_and_drag());
    let click_over_content = response
        .interact_pointer_pos()
        .map(|pos| {
            super::hit_testing::grid_item_content_contains(ui, item, rect.shrink(3.0), ctx, pos)
        })
        .unwrap_or(ctx.is_computer_view);
    if response.clicked() {
        if click_over_content {
            *clicked_item = Some(index);
        } else {
            *empty_area_clicked = true;
        }
    }
    if response.double_clicked() && click_over_content {
        *double_clicked_item = Some(index);
    }
    if response.secondary_clicked() {
        *secondary_clicked_item = Some(index);
    }
    let pointer_moved = ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO);
    let drag_candidate = response.drag_started()
        || response.dragged()
        || (response.is_pointer_button_down_on() && pointer_moved);
    let rectangle_select_active = ctx.rectangle_selection_state.is_some();
    if drag_candidate && !rectangle_select_active {
        if ctx.is_computer_view {
            *ctx.drag_started_item = Some(index);
        } else if let Some(origin) = ui.input(|input| input.pointer.press_origin()) {
            let content_rect = rect.shrink(3.0);
            if super::hit_testing::grid_item_content_contains(ui, item, content_rect, ctx, origin) {
                *ctx.drag_started_item = Some(index);
            } else {
                ctx.rectangle_selection_frame.request_start(origin);
            }
        } else {
            *ctx.drag_started_item = Some(index);
        }
    }
    let is_pointer_over = response.contains_pointer() || response.hovered();
    // For drag-hover detection use ONLY contains_pointer() (geometric check).
    // response.hovered() stays locked to the drag-source widget in egui,
    // so when the source is rendered AFTER the real target (target is to the
    // left / above), it would overwrite drag_hovered_item → wrong target → denied cursor.
    if response.contains_pointer() && item.is_dir {
        *ctx.drag_hovered_item = Some(index);
    }

    let is_selected = ctx
        .rectangle_selection_state
        .map(|state| state.preview_contains(index))
        .unwrap_or_else(|| ctx.multi_selection.contains(&item.path));
    let allow_hover = matches!(ctx.last_input, crate::app::state::LastInput::Mouse);
    let is_hovered_visual = allow_hover && response.hovered() && !is_selected;
    let is_focused = ctx.selected_item == Some(index);

    let rounding = 4.0;
    let accent_color = crate::ui::theme::COLOR_ACCENT;

    if is_selected {
        let stroke_width = if is_hovered_visual { 2.5 } else { 2.0 };
        ui.painter().rect_stroke(
            rect,
            rounding,
            egui::Stroke::new(stroke_width, accent_color),
            egui::StrokeKind::Inside,
        );
    } else if is_hovered_visual || is_focused {
        let hover_color = accent_color.gamma_multiply(0.35);
        ui.painter().rect_stroke(
            rect,
            rounding,
            egui::Stroke::new(1.0, hover_color),
            egui::StrokeKind::Inside,
        );
    }

    let pointer_over_drop_candidate = ctx.is_item_dragging && item.is_dir && is_pointer_over;
    let is_active_drop_target = ctx.is_item_dragging
        && item.is_dir
        && ctx
            .drag_target_folder
            .as_ref()
            .is_some_and(|target| *target == item.path);

    if pointer_over_drop_candidate || is_active_drop_target {
        let stroke_color = if is_active_drop_target {
            Color32::from_rgb(24, 122, 255)
        } else {
            accent_color.gamma_multiply(0.75)
        };
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            rounding,
            egui::Stroke::new(2.0, stroke_color),
            egui::StrokeKind::Inside,
        );
    }

    if response.hovered() && !ctx.is_item_dragging && !rectangle_select_active {
        let current_time = ui.input(|i| i.time);
        // PERF FIX: Use path-based hover ID so the tooltip timer resets when
        // navigating to a different folder (prevents stale timer from the
        // previous folder's item at the same index triggering an immediate
        // tooltip with a blocking std::fs::metadata call on cold cache).
        let hover_id = egui::Id::new("grid_hover_start").with(&item.path);
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
            if let Some(mouse_pos) = ui.input(|i| i.pointer.hover_pos()) {
                let is_recycle = ctx.is_recycle_bin_view;
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
                            ui.label(
                                egui::RichText::new(
                                    crate::ui::components::item_slot::display_name_for_item(item)
                                        .as_ref(),
                                )
                                .strong(),
                            );
                            ui.separator();
                            if item.drive_info.is_some() {
                                render_drive_tooltip(ui, item);
                                return;
                            }
                            ui.horizontal(|ui| {
                                ui.label(rust_i18n::t!("file_info.type").to_string());
                                ui.label(get_file_type_string(item));
                            });
                            if !item.is_dir || item.is_archive() {
                                ui.horizontal(|ui| {
                                    ui.label(rust_i18n::t!("file_info.size").to_string());
                                    ui.label(crate::infrastructure::windows::format_size(
                                        resolve_tooltip_live_size(ui, item, ctx),
                                    ));
                                });
                            }
                            let (date_lbl, date_val) = if is_recycle {
                                (
                                    rust_i18n::t!("list_view.date_deleted").to_string(),
                                    if item.modified > 0 {
                                        crate::infrastructure::windows::format_date(item.modified)
                                    } else {
                                        item.deletion_date()
                                            .map(|s| s.to_string())
                                            .unwrap_or_else(|| "-".to_string())
                                    },
                                )
                            } else {
                                (
                                    rust_i18n::t!("list_view.date_modified").to_string(),
                                    crate::infrastructure::windows::format_date(item.modified),
                                )
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
        let hover_id = egui::Id::new("grid_hover_start").with(&item.path);
        ui.ctx().data_mut(|d| d.remove::<f64>(hover_id));
    }

    let inner_rect = rect.shrink(3.0);
    render_item_slot_for_grid(ui, inner_rect, index, item, ctx, is_scrolling);
}

pub(super) fn render_section_header(ui: &mut Ui, title: &str) {
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new(title)
            .size(13.0)
            .color(Color32::from_gray(120))
            .strong(),
    );
    ui.add_space(4.0);
}

fn render_item_slot_for_grid(
    ui: &mut Ui,
    rect: Rect,
    idx: usize,
    item: &FileEntry,
    ctx: &mut GridViewContext,
    is_scrolling: bool,
) {
    use crate::ui::components::item_slot::{render_item_slot, ItemSlotContext};

    let is_renaming = ctx.renaming_state.as_ref().is_some_and(|(i, _)| *i == idx);

    let mut renaming_text_clone = if is_renaming {
        ctx.renaming_state.as_ref().map(|(_, s)| s.clone())
    } else {
        None
    };

    {
        let renaming_text = renaming_text_clone.as_mut();

        let mut item_slot_ctx = ItemSlotContext {
            item,
            idx,
            thumbnail_size: ctx.thumbnail_size,
            is_renaming,
            renaming_text,
            focus_rename: ctx.focus_rename,
            is_recycle_bin_view: ctx.is_recycle_bin_view,
            texture_cache: ctx.texture_cache,
            attempted_thumbnail_bucket: ctx.attempted_thumbnail_bucket,
            icon_loader: ctx.item_icon_loader,
            scanned_folders: ctx.scanned_folders,
            loading_set: ctx.loading_set,
            loading_icons: ctx.loading_icons,
            failed_icons: ctx.failed_icons,
            folder_preview_cache: ctx.folder_preview_cache,
            folder_preview_loading: ctx.folder_preview_loading,
            skip_folder_media_reads: ctx.skip_folder_media_reads,
            failed_thumbnails: ctx.failed_thumbnails,
            pending_upload_set: ctx.pending_upload_set,
            is_dense_mode: false,
            is_scrolling,
            thumbnail_requests_this_frame: &mut ctx.thumbnail_requests_this_frame,
        };

        struct SimpleOps<'a> {
            pending_ops: &'a mut PendingOperations,
        }

        impl<'a> crate::ui::components::item_slot::ItemSlotOperations for SimpleOps<'a> {
            fn request_thumbnail_load(
                &mut self,
                path: std::path::PathBuf,
                size: u32,
                directory_index: Option<usize>,
                modified: u64,
            ) {
                self.pending_ops
                    .thumbnail_loads
                    .push((path, size, directory_index, modified));
            }

            fn request_folder_scan(&mut self, path: std::path::PathBuf) {
                self.pending_ops.folder_scans.push(path);
            }
            fn request_folder_preview_load(&mut self, path: std::path::PathBuf) {
                self.pending_ops.folder_preview_loads.push(path);
            }
            fn request_icon_load(&mut self, path: std::path::PathBuf) {
                self.pending_ops.icon_loads.push(path);
            }

            fn rename_item(&mut self, idx: usize) {
                self.pending_ops.renames.push(idx);
            }
        }

        let mut simple_ops = SimpleOps {
            pending_ops: ctx.pending_ops,
        };

        if item.is_hidden {
            ui.scope(|ui| {
                ui.multiply_opacity(0.5);
                render_item_slot(ui, rect, &mut item_slot_ctx, &mut simple_ops);
            });
        } else {
            render_item_slot(ui, rect, &mut item_slot_ctx, &mut simple_ops);
        }
    }

    if let Some(new_text) = renaming_text_clone {
        if is_renaming {
            if let Some((_, ref mut text)) = ctx.renaming_state {
                *text = new_text;
            }
        }
    }
}

fn get_file_type_string(item: &FileEntry) -> String {
    if let Some(label) = crate::domain::file_entry::archive_type_label(&item.name) {
        return label;
    }
    if item.is_dir {
        return rust_i18n::t!("file_types.folder").to_string();
    }

    if let Some(ext) = item.path.extension() {
        return rust_i18n::t!(
            "file_info.file_generic",
            ext = ext.to_string_lossy().to_uppercase()
        )
        .to_string();
    }

    rust_i18n::t!("file_info.file_unknown").to_string()
}
