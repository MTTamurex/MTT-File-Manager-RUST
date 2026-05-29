//! Batch Rename Modal — advanced multi-file renaming with live preview,
//! drag-to-reorder, and conflict detection.

use crate::app::batch_rename::{
    BatchRenameState, DragState, NumberPosition, NumberSeparator, PreviewRow,
};
use crate::app::state::ImageViewerApp;
use crate::ui::theme;
use eframe::egui::{self, Color32, Margin, RichText, Sense, Stroke, Vec2};
use rust_i18n::t;

// ── Constants ─────────────────────────────────────────────────────────────────

const ROW_HEIGHT: f32 = 22.0;
const HANDLE_WIDTH: f32 = 24.0;
const NUM_WIDTH: f32 = 28.0;
const BACKDROP_ALPHA: u8 = 72;

// ── Entry point ───────────────────────────────────────────────────────────────

/// Renders the batch rename modal.  Must be called every frame while
/// `app.batch_rename_state` is `Some`.
pub fn render_batch_rename_modal(app: &mut ImageViewerApp, ctx: &egui::Context) {
    let Some(state) = &app.batch_rename_state else {
        return;
    };

    let title = t!("batch_rename.title", count = state.sources.len()).to_string();
    let mut open = true;

    let screen_rect = ctx.screen_rect();

    // ── Backdrop (blocks interaction outside the modal) ──────────────────────
    let mut close_from_backdrop = false;
    egui::Area::new(egui::Id::from("batch_rename_backdrop"))
        .fixed_pos(screen_rect.min)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            ui.set_min_size(screen_rect.size());
            let backdrop_rect = ui.max_rect();
            let backdrop_resp = ui.interact(
                backdrop_rect,
                ui.id().with("batch_rename_backdrop_interact"),
                egui::Sense::click(),
            );
            ui.painter().rect_filled(
                backdrop_rect,
                0.0,
                Color32::from_black_alpha(BACKDROP_ALPHA),
            );
            if backdrop_resp.clicked() {
                close_from_backdrop = true;
            }
        });

    if close_from_backdrop {
        app.batch_rename_state = None;
        return;
    }

    // ESC cancels
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.batch_rename_state = None;
        return;
    }

    let dark_mode = ctx.style().visuals.dark_mode;
    let bg_color = if dark_mode {
        Color32::from_rgb(50, 50, 50)
    } else {
        Color32::from_rgb(250, 250, 250)
    };

    let frame = egui::Frame::new()
        .inner_margin(Margin {
            left: 24,
            right: 24,
            top: 20,
            bottom: 16,
        })
        .corner_radius(10.0)
        .fill(bg_color)
        .stroke(Stroke::new(
            1.0,
            if dark_mode {
                Color32::from_gray(70)
            } else {
                Color32::from_gray(220)
            },
        ))
        .shadow(egui::epaint::Shadow {
            spread: 4,
            blur: 12,
            color: Color32::from_black_alpha(25),
            offset: [0, 3],
        });

    let mut do_apply = false;
    let mut do_cancel = false;

    egui::Window::new("")
        .open(&mut open)
        .title_bar(false)
        .collapsible(false)
        .resizable(true)
        .default_width(600.0)
        .default_height(580.0)
        .min_width(520.0)
        .min_height(380.0)
        .max_height(800.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .order(egui::Order::Foreground)
        .frame(frame)
        .show(ctx, |ui| {
            // We take ownership of the state for the frame, then put it back.
            let mut state = app.batch_rename_state.take().unwrap();

            // Header
            ui.label(
                RichText::new(&title)
                    .size(18.0)
                    .strong()
                    .color(theme::text_color(dark_mode)),
            );

            ui.add_space(14.0);

            render_controls(ui, &mut state, dark_mode);

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(4.0);

            // Compute preview once per frame (cheap for typical batch sizes)
            let preview = state.compute_preview();
            let valid_count = preview.iter().filter(|r| !r.conflict).count();

            // Fixed heights for the two scroll areas — avoids the feedback loop
            // where available_height() grows with the window, causing infinite expansion.
            let half = 140.0_f32;

            ui.label(
                RichText::new(t!("batch_rename.rename_order"))
                    .strong()
                    .color(theme::text_color(dark_mode)),
            );
            ui.add_space(2.0);
            render_reorderable_list(ui, &mut state, &preview, half, dark_mode);

            ui.add_space(6.0);
            ui.label(
                RichText::new(t!("batch_rename.preview"))
                    .strong()
                    .color(theme::text_color(dark_mode)),
            );
            ui.add_space(2.0);
            render_preview_table(ui, &preview, half, dark_mode);

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

            // ── Button row (right-aligned) ───────────────────────────────────
            let apply_enabled = !state.name_template.is_empty() && valid_count > 0;
            let apply_label = t!("batch_rename.btn_rename", count = valid_count).to_string();

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(apply_enabled, |ui| {
                    if ui
                        .add_sized(
                            Vec2::new(140.0, 34.0),
                            egui::Button::new(
                                RichText::new(apply_label)
                                    .size(14.0)
                                    .strong()
                                    .color(Color32::WHITE),
                            )
                            .fill(theme::COLOR_ACCENT),
                        )
                        .clicked()
                    {
                        do_apply = true;
                    }
                });

                ui.add_space(12.0);

                if ui
                    .add_sized(
                        Vec2::new(90.0, 34.0),
                        egui::Button::new(
                            RichText::new(t!("batch_rename.btn_cancel"))
                                .size(14.0)
                                .color(theme::secondary_text_color(dark_mode)),
                        )
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::new(
                            1.0,
                            theme::secondary_text_color(dark_mode),
                        )),
                    )
                    .clicked()
                {
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

    // If the user closed the window via the X button (title_bar is false, so this
    // only fires if something else sets open = false).
    if !open {
        app.batch_rename_state = None;
    }
}

// ── Controls row ─────────────────────────────────────────────────────────────

fn render_controls(ui: &mut egui::Ui, state: &mut BatchRenameState, dark_mode: bool) {
    egui::Grid::new("batch_rename_controls")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            // Row 1 — Name template
            ui.label(
                RichText::new(t!("batch_rename.base_name")).color(theme::text_color(dark_mode)),
            );
            ui.add(
                egui::TextEdit::singleline(&mut state.name_template)
                    .desired_width(f32::INFINITY)
                    .hint_text(t!("batch_rename.base_name_hint")),
            );
            ui.end_row();

            // Row 2 — Position + Separator
            ui.label(
                RichText::new(t!("batch_rename.position")).color(theme::text_color(dark_mode)),
            );
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

                ui.label(
                    RichText::new(t!("batch_rename.separator"))
                        .color(theme::text_color(dark_mode)),
                );
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
            ui.label(
                RichText::new(t!("batch_rename.numbering")).color(theme::text_color(dark_mode)),
            );
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(t!("batch_rename.start"))
                        .color(theme::text_color(dark_mode)),
                );
                ui.add(egui::DragValue::new(&mut state.start).range(0..=99999));

                ui.add_space(8.0);
                ui.label(
                    RichText::new(t!("batch_rename.step")).color(theme::text_color(dark_mode)),
                );
                ui.add(egui::DragValue::new(&mut state.step).range(1..=9999));

                ui.add_space(8.0);
                ui.label(
                    RichText::new(t!("batch_rename.padding")).color(theme::text_color(dark_mode)),
                );
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
    dark_mode: bool,
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
                        let glyph_color = if handle.hovered() || handle.dragged() {
                            ui.visuals().widgets.active.fg_stroke.color
                        } else {
                            ui.visuals().weak_text_color()
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
                            egui::Label::new(
                                RichText::new(format!("{}.", i + 1))
                                    .weak()
                                    .color(theme::secondary_text_color(dark_mode)),
                            ),
                        );

                        // Filename (conflict highlighted)
                        let name_text = if has_conflict {
                            RichText::new(&filenames[i]).color(Color32::from_rgb(220, 80, 80))
                        } else {
                            RichText::new(&filenames[i]).color(theme::text_color(dark_mode))
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

fn render_preview_table(
    ui: &mut egui::Ui,
    preview: &[PreviewRow],
    height: f32,
    dark_mode: bool,
) {
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
                    ui.label(
                        RichText::new(t!("batch_rename.col_num"))
                            .strong()
                            .color(theme::text_color(dark_mode)),
                    );
                    ui.label(
                        RichText::new(t!("batch_rename.col_original"))
                            .strong()
                            .color(theme::text_color(dark_mode)),
                    );
                    ui.label(
                        RichText::new(t!("batch_rename.col_new"))
                            .strong()
                            .color(theme::text_color(dark_mode)),
                    );
                    ui.end_row();

                    for (i, row) in preview.iter().enumerate() {
                        ui.label(
                            RichText::new(format!("{}.", i + 1))
                                .weak()
                                .color(theme::secondary_text_color(dark_mode)),
                        );
                        ui.label(
                            RichText::new(&row.old_name).color(theme::text_color(dark_mode)),
                        );

                        if row.conflict {
                            ui.colored_label(
                                Color32::from_rgb(220, 80, 80),
                                format!("{}{}", row.new_name, t!("batch_rename.conflict_suffix")),
                            );
                        } else {
                            ui.label(
                                RichText::new(&row.new_name).color(theme::text_color(dark_mode)),
                            );
                        }
                        ui.end_row();
                    }
                });
        });
}
