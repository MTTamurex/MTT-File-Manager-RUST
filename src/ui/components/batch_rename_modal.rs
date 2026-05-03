//! Batch Rename Modal — advanced multi-file renaming with live preview,
//! drag-to-reorder, and conflict detection.

use crate::app::batch_rename::{
    BatchRenameState, DragState, NumberPosition, NumberSeparator, PreviewRow,
};
use crate::app::state::ImageViewerApp;
use eframe::egui::{self, Color32, RichText, Sense, Stroke};
use rust_i18n::t;

// ── Constants ─────────────────────────────────────────────────────────────────

const ROW_HEIGHT: f32 = 22.0;
const HANDLE_WIDTH: f32 = 24.0;
const NUM_WIDTH: f32 = 28.0;

// ── Entry point ───────────────────────────────────────────────────────────────

/// Renders the batch rename modal.  Must be called every frame while
/// `app.batch_rename_state` is `Some`.
pub fn render_batch_rename_modal(app: &mut ImageViewerApp, ctx: &egui::Context) {
    let Some(state) = &app.batch_rename_state else {
        return;
    };

    let title = t!("batch_rename.title", count = state.sources.len()).to_string();
    let mut open = true;

    egui::Window::new(title)
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(600.0)
        .default_height(580.0)
        .min_width(520.0)
        .min_height(380.0)
        .max_height(800.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            // We take ownership of the state for the frame, then put it back.
            // This avoids borrow-checker issues when calling mutable methods.
            let mut state = app.batch_rename_state.take().unwrap();

            render_controls(ui, &mut state);

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(4.0);

            // Compute preview once per frame (cheap for typical batch sizes)
            let preview = state.compute_preview();
            let valid_count = preview.iter().filter(|r| !r.conflict).count();

            // Fixed heights for the two scroll areas — avoids the feedback loop
            // where available_height() grows with the window, causing infinite expansion.
            let half = 140.0_f32;

            ui.label(RichText::new(t!("batch_rename.rename_order")).strong());
            ui.add_space(2.0);
            render_reorderable_list(ui, &mut state, &preview, half);

            ui.add_space(6.0);
            ui.label(RichText::new(t!("batch_rename.preview")).strong());
            ui.add_space(2.0);
            render_preview_table(ui, &preview, half);

            // Conflict warning banner
            let conflict_count = preview.iter().filter(|r| r.conflict).count();
            if conflict_count > 0 {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.colored_label(
                        Color32::from_rgb(220, 160, 0),
                        t!("batch_rename.conflict_warning", count = conflict_count).to_string(),
                    );
                });
            }

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(4.0);

            // ── Button row ────────────────────────────────────────────────────
            let mut do_apply = false;
            let mut do_cancel = false;

            ui.horizontal(|ui| {
                let apply_enabled = !state.name_template.is_empty() && valid_count > 0;
                let apply_label = t!("batch_rename.btn_rename", count = valid_count).to_string();

                ui.add_enabled_ui(apply_enabled, |ui| {
                    if ui.button(apply_label).clicked() {
                        do_apply = true;
                    }
                });

                if ui.button(t!("batch_rename.btn_cancel")).clicked() {
                    do_cancel = true;
                }
            });

            // Put state back before deciding apply/cancel
            app.batch_rename_state = Some(state);

            if do_apply {
                app.apply_batch_rename();
            } else if do_cancel {
                app.batch_rename_state = None;
            }
        });

    // If the user closed the window via the X button
    if !open {
        app.batch_rename_state = None;
    }
}

// ── Controls row ─────────────────────────────────────────────────────────────

fn render_controls(ui: &mut egui::Ui, state: &mut BatchRenameState) {
    egui::Grid::new("batch_rename_controls")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            // Row 1 — Name template
            ui.label(t!("batch_rename.base_name"));
            ui.add(
                egui::TextEdit::singleline(&mut state.name_template)
                    .desired_width(f32::INFINITY)
                    .hint_text(t!("batch_rename.base_name_hint")),
            );
            ui.end_row();

            // Row 2 — Position + Separator
            ui.label(t!("batch_rename.position"));
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("batch_pos")
                    .selected_text(state.position.display_name())
                    .width(180.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut state.position,
                            NumberPosition::Suffix,
                            NumberPosition::Suffix.display_name(),
                        );
                        ui.selectable_value(
                            &mut state.position,
                            NumberPosition::Prefix,
                            NumberPosition::Prefix.display_name(),
                        );
                    });

                ui.label(t!("batch_rename.separator"));
                egui::ComboBox::from_id_salt("batch_sep")
                    .selected_text(state.separator.display_name())
                    .width(200.0)
                    .show_ui(ui, |ui| {
                        for sep in [
                            NumberSeparator::Parentheses,
                            NumberSeparator::Underscore,
                            NumberSeparator::Dash,
                            NumberSeparator::Space,
                            NumberSeparator::None,
                        ] {
                            let label = sep.display_name();
                            ui.selectable_value(&mut state.separator, sep, label);
                        }
                    });
            });
            ui.end_row();

            // Row 3 — Start, Step, Padding
            ui.label(t!("batch_rename.numbering"));
            ui.horizontal(|ui| {
                ui.label(t!("batch_rename.start"));
                ui.add(egui::DragValue::new(&mut state.start).range(0..=99999));

                ui.add_space(8.0);
                ui.label(t!("batch_rename.step"));
                ui.add(egui::DragValue::new(&mut state.step).range(1..=9999));

                ui.add_space(8.0);
                ui.label(t!("batch_rename.padding"));
                ui.add(
                    egui::DragValue::new(&mut state.padding)
                        .range(0..=6)
                        .suffix(t!("batch_rename.padding_suffix")),
                );
            });
            ui.end_row();
        });
}

// ── Reorderable source list ───────────────────────────────────────────────────

fn render_reorderable_list(
    ui: &mut egui::Ui,
    state: &mut BatchRenameState,
    preview: &[PreviewRow],
    height: f32,
) {
    // Collect filenames before borrowing state mutably
    let filenames: Vec<String> = state
        .sources
        .iter()
        .map(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string()
        })
        .collect();

    let n = filenames.len();
    let is_dragging = state.drag_state.is_some();
    let pointer_pos = ui.ctx().pointer_hover_pos();

    let mut drag_started_at: Option<usize> = None;
    let mut drag_released = false;
    let mut hover_target: Option<usize> = None;

    egui::ScrollArea::vertical()
        .id_salt("batch_list_scroll")
        .max_height(height)
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());

            for i in 0..n {
                let is_drag_src = state
                    .drag_state
                    .as_ref()
                    .map_or(false, |ds| ds.dragging_idx == i);
                let is_drag_tgt = state
                    .drag_state
                    .as_ref()
                    .map_or(false, |ds| ds.hover_idx == i && ds.dragging_idx != i);
                let has_conflict = preview.get(i).map_or(false, |r| r.conflict);

                let result = ui.push_id(i, |ui| {
                    let row = ui.horizontal(|ui| {
                        ui.set_min_height(ROW_HEIGHT);

                        // Drag handle — allocate a raw rect with Sense::drag() only so
                        // the cursor stays as a grab/pointer icon and no text is selected.
                        let handle_size = egui::vec2(HANDLE_WIDTH, ROW_HEIGHT);
                        let (handle_rect, handle) =
                            ui.allocate_exact_size(handle_size, Sense::drag());

                        // Show grab cursor on hover/drag
                        if handle.hovered() || handle.dragged() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }

                        // Paint the braille grid glyph centred in the handle rect
                        let visuals = ui.visuals();
                        let glyph_color = if handle.hovered() || handle.dragged() {
                            visuals.widgets.active.fg_stroke.color
                        } else {
                            visuals.weak_text_color()
                        };
                        ui.painter().text(
                            handle_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "⠿",
                            egui::FontId::proportional(14.0),
                            glyph_color,
                        );

                        // Row number
                        ui.add_sized(
                            [NUM_WIDTH, ROW_HEIGHT],
                            egui::Label::new(RichText::new(format!("{}.", i + 1)).weak()),
                        );

                        // Filename (conflict highlighted)
                        let name_text = if has_conflict {
                            RichText::new(&filenames[i]).color(Color32::from_rgb(220, 80, 80))
                        } else {
                            RichText::new(&filenames[i])
                        };
                        ui.label(name_text);

                        handle
                    });

                    let handle_resp = row.inner;
                    let row_rect = row.response.rect;
                    (handle_resp, row_rect)
                });

                let (handle_resp, row_rect) = result.inner;

                // Paint highlights behind the row text via the painter
                if is_drag_src {
                    ui.painter().rect_filled(
                        row_rect,
                        2.0,
                        Color32::from_rgba_unmultiplied(150, 150, 150, 40),
                    );
                }
                if is_drag_tgt {
                    // Draw a bold insertion line at the top of the target row
                    ui.painter().line_segment(
                        [row_rect.left_top(), row_rect.right_top()],
                        Stroke::new(2.0, ui.visuals().selection.bg_fill),
                    );
                }

                if handle_resp.drag_started() {
                    drag_started_at = Some(i);
                }
                if handle_resp.drag_stopped() {
                    drag_released = true;
                }

                // Update hover target while dragging
                if is_dragging {
                    if let Some(ptr) = pointer_pos {
                        if row_rect.contains(ptr) {
                            hover_target = Some(i);
                        }
                    }
                }
            }
        });

    // ── Apply drag-state changes after the loop ───────────────────────────────

    if let Some(idx) = drag_started_at {
        state.drag_state = Some(DragState {
            dragging_idx: idx,
            hover_idx: idx,
        });
    }

    if let Some(ds) = &mut state.drag_state {
        if let Some(t) = hover_target {
            ds.hover_idx = t;
        }
    }

    if drag_released {
        if let Some(ds) = state.drag_state.take() {
            if ds.dragging_idx != ds.hover_idx {
                let insert_idx = if ds.dragging_idx < ds.hover_idx {
                    ds.hover_idx.saturating_sub(1)
                } else {
                    ds.hover_idx
                };
                let item = state.sources.remove(ds.dragging_idx);
                state
                    .sources
                    .insert(insert_idx.min(state.sources.len()), item);
            }
        }
    }
}

// ── Preview table ─────────────────────────────────────────────────────────────

fn render_preview_table(ui: &mut egui::Ui, preview: &[PreviewRow], height: f32) {
    egui::ScrollArea::vertical()
        .id_salt("batch_preview_scroll")
        .max_height(height)
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            egui::Grid::new("batch_preview_grid")
                .num_columns(3)
                .striped(true)
                .spacing([8.0, 2.0])
                .show(ui, |ui| {
                    // Header
                    ui.label(RichText::new(t!("batch_rename.col_num")).strong());
                    ui.label(RichText::new(t!("batch_rename.col_original")).strong());
                    ui.label(RichText::new(t!("batch_rename.col_new")).strong());
                    ui.end_row();

                    for (i, row) in preview.iter().enumerate() {
                        ui.label(RichText::new(format!("{}.", i + 1)).weak());
                        ui.label(&row.old_name);

                        if row.conflict {
                            ui.colored_label(
                                Color32::from_rgb(220, 80, 80),
                                format!("{}{}", row.new_name, t!("batch_rename.conflict_suffix")),
                            );
                        } else {
                            ui.label(&row.new_name);
                        }
                        ui.end_row();
                    }
                });
        });
}
