//! Miller's Columns bridge - orchestrates the multi-column view.
//!
//! The rightmost (focused) column is `current_path` and reuses the full
//! details list view (`render_list_view`) for complete interaction parity
//! (rename, multi-select, drag, rectangle selection, keyboard, context menu).
//! Ancestor columns to its left are rendered from a lightweight background
//! listing cache (`MillerColumnsState`) and support select / open / rename /
//! context menu via path-based actions. Clicking an ancestor folder navigates into it
//! (promoting it to the focused column) — matching a Finder-style column view.

use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::FileEntry;
use crate::domain::special_paths::is_virtual_path;
use crate::ui::views::miller_columns_view::{
    ancestor_chain, render_miller_column, MillerColumnContext, ANCESTOR_COL_WIDTH,
    FOCUSED_COL_WIDTH,
};
use crate::ui::views::rectangle_selection::RectangleSelectionSource;

mod interactions;
mod rename_click;

impl ImageViewerApp {
    pub fn render_miller_columns_view(&mut self, ui: &mut egui::Ui) {
        self.miller_columns.poll();

        // Non-hierarchical virtual views use the compact list. Filesystem and
        // archive namespace paths retain the full Miller ancestor chain.
        if self.navigation_state.is_computer_view
            || self.navigation_state.is_recycle_bin_view
            || self.navigation_state.current_path.is_empty()
            || is_virtual_path(&self.navigation_state.current_path)
        {
            if self
                .rectangle_selection_state
                .as_ref()
                .is_some_and(|state| {
                    matches!(
                        state.source,
                        RectangleSelectionSource::MillerAncestor { .. }
                    )
                })
            {
                self.cancel_rectangle_selection();
            }
            self.render_list_view_compact(ui);
            return;
        }

        let current_path = self.navigation_state.current_path.clone();

        // Left/Right arrow: move between columns (Up/Down/Enter belong to the
        // focused list view). Deferred so current_path is stable this frame.
        // This must run before the drive-root shortcut below: the root still
        // represents the focused Miller column even though it has no ancestors.
        let mut pending_nav: Option<String> = None;
        let allow_kb = !self.suppress_file_panel_keyboard
            && !self.global_search.active
            && !self.navigation_state.show_settings_window
            && self.renaming_state.is_none()
            && self.rectangle_selection_state.is_none()
            && !ui.ctx().egui_wants_keyboard_input();
        if allow_kb {
            let (left, right) = ui.input(|i| {
                (
                    i.key_pressed(egui::Key::ArrowLeft),
                    i.key_pressed(egui::Key::ArrowRight),
                )
            });
            if left {
                if let Some(parent) = Path::new(&current_path).parent() {
                    let parent_s = parent.to_string_lossy().to_string();
                    if !parent_s.is_empty() && parent_s != current_path {
                        pending_nav = Some(parent_s);
                    }
                }
            } else if right {
                if let Some(sel) = self.selected_file.as_ref() {
                    if sel.is_dir {
                        pending_nav = Some(sel.path.to_string_lossy().to_string());
                    }
                }
            }
        }

        let chain = ancestor_chain(&current_path);
        if chain.len() <= 1 {
            // Drive root: nothing to the left, but Miller remains name-only.
            self.render_list_view_compact(ui);
            if let Some(target) = pending_nav {
                self.activate_location(&target);
                ui.ctx().request_repaint();
            }
            return;
        }
        let focused_index = chain.len() - 1;

        // Keep ancestor listings warm with the active sort/filter signature.
        let signature = (
            self.sort_mode,
            self.sort_descending,
            self.folders_position,
            self.show_hidden_files,
        );
        self.miller_columns.set_signature(signature);
        let keep: std::collections::HashSet<PathBuf> = chain.iter().cloned().collect();
        self.miller_columns.retain(&keep);
        for dir in chain.iter().take(focused_index) {
            self.miller_columns.ensure(dir);
        }

        // Track focus changes so the strip scrolls to reveal the focused
        // column after navigation.
        self.miller_columns.note_focused_dir(&current_path);

        let viewport = ui.available_rect_before_wrap();

        // Owned data for ancestor columns (Arc clones — no borrow on self).
        let folder_icon = self.cache_manager.folder_icon_texture.clone();
        let selected_file_path = self.selected_file.as_ref().map(|f| f.path.clone());
        let selected_paths = self.multi_selection.clone();
        let rectangle_selection_state = self.rectangle_selection_state.clone();
        let scroll_interpolation_enabled = self.uses_scroll_interpolation();
        let ancestor_data: Vec<(Option<Arc<Vec<FileEntry>>>, bool)> = chain
            .iter()
            .take(focused_index)
            .map(|dir| {
                (
                    self.miller_columns.get_arc(dir),
                    self.miller_columns.is_loading(dir),
                )
            })
            .collect();

        let ancestor_groups: Vec<_> = chain
            .iter()
            .zip(&ancestor_data)
            .take(focused_index)
            .map(|(dir, (listing, _))| {
                let projection = listing
                    .as_ref()
                    .map(|items| {
                        self.miller_columns.grouping.projection(
                            dir,
                            items.clone(),
                            self.group_mode,
                            self.group_descending,
                        )
                    })
                    .unwrap_or_default();
                let key = self.grouping_context_key_for_path(&dir.to_string_lossy());
                let collapsed = self
                    .collapsed_groups_by_context
                    .get(&key)
                    .cloned()
                    .unwrap_or_default();
                (projection, collapsed)
            })
            .collect();

        let snap_to_focused = self.miller_columns.take_scroll_to_focused_pending();
        let strip_width = focused_index as f32 * ANCESTOR_COL_WIDTH + FOCUSED_COL_WIDTH;
        let max_horizontal_offset = (strip_width - viewport.width()).max(0.0);
        let horizontal_offset = if snap_to_focused {
            max_horizontal_offset
        } else {
            self.miller_columns
                .horizontal_scroll_offset()
                .clamp(0.0, max_horizontal_offset)
        };

        // Render the strip inside a horizontal scroll area: egui provides the
        // horizontal scrollbar and correct clipping (no left-bleed / overlap).
        // Each column is a fixed-width region; the focused column reuses the
        // full details list view for complete interaction parity.
        let mut ancestor_outputs = Vec::new();
        let mut icon_requests = Vec::new();
        let mut column_drop_target = None;
        let external_drop_pointer_pos = if self.external_drop_active {
            self.external_drop_pointer_pos
                .or_else(|| ui.input(|input| input.pointer.hover_pos()))
        } else {
            None
        };
        let scroll_output = ui.scope(|ui| {
            // The global scrollbar style is fully transparent while dormant.
            // Keep this navigation scrollbar visible whenever it is needed.
            ui.style_mut().spacing.scroll.dormant_handle_opacity = 0.4;

            egui::ScrollArea::horizontal()
                .id_salt("miller_strip")
                .auto_shrink([false, false])
                .scroll_source(egui::scroll_area::ScrollSource {
                    drag: egui::scroll_area::DragScroll::Never,
                    ..Default::default()
                })
                .horizontal_scroll_offset(horizontal_offset)
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                .show(ui, |ui| {
                    // Fixed child allocations may report only their used size.
                    // Pin the strip width so overflow and the scrollbar are detected.
                    ui.set_min_width(strip_width);
                    let col_height = ui.available_height();
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;

                        for (col_idx, (listing, loading)) in ancestor_data.iter().enumerate() {
                            let items: &[FileEntry] =
                                listing.as_deref().map(|v| v.as_slice()).unwrap_or(&[]);
                            let selected_child = chain.get(col_idx + 1).map(|p| p.as_path());
                            let selection_source = RectangleSelectionSource::MillerAncestor {
                                directory: chain[col_idx].clone(),
                            };
                            let column_rectangle_selection = rectangle_selection_state
                                .as_ref()
                                .filter(|state| state.source == selection_source);
                            let inner = ui.allocate_ui_with_layout(
                                egui::vec2(ANCESTOR_COL_WIDTH, col_height),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    let mut cctx = MillerColumnContext {
                                        items,
                                        group_projection: &ancestor_groups[col_idx].0,
                                        collapsed_groups: &ancestor_groups[col_idx].1,
                                        directory: &chain[col_idx],
                                        selected_child,
                                        selected_file: selected_file_path.as_deref(),
                                        multi_selection: &selected_paths,
                                        renaming_state: self.renaming_state.as_ref(),
                                        focus_rename: self.focus_rename,
                                        rectangle_selection_state: column_rectangle_selection,
                                        icon_loader: &mut self.item_icon_loader,
                                        folder_icon: folder_icon.as_ref(),
                                        loading_icons: &self.loading_icons,
                                        failed_icons: &self.failed_icons,
                                        icon_requests: &mut icon_requests,
                                        is_item_dragging: self.is_item_dragging
                                            || self.external_drop_active,
                                        drop_target: &mut column_drop_target,
                                        external_drop_pointer_pos,
                                        is_loading: *loading,
                                        scroll_interpolation_enabled,
                                    };
                                    render_miller_column(ui, ("miller_col", col_idx), &mut cctx)
                                },
                            );
                            ancestor_outputs.push((col_idx, inner.inner));
                        }

                        if self.is_item_dragging {
                            self.drag_target_folder = column_drop_target
                                .as_ref()
                                .filter(|target| self.is_valid_drop_target(target))
                                .cloned();
                            let (ctrl, shift, primary_released) = ui.input(|input| {
                                (
                                    input.modifiers.ctrl,
                                    input.modifiers.shift,
                                    input.pointer.primary_released(),
                                )
                            });
                            if self.drag_target_folder.is_some() && primary_released {
                                self.complete_item_drag(ctrl, shift);
                            }
                        } else if self.external_drop_active {
                            self.drag_target_folder = column_drop_target
                                .as_ref()
                                .filter(|target| !Self::path_is_archive_namespace(target))
                                .cloned();
                        }

                        // Focused (rightmost) column: the current directory, using
                        // the details list view in compact (name-only) mode so it
                        // keeps full interactions but matches the column look.
                        ui.allocate_ui_with_layout(
                            egui::vec2(FOCUSED_COL_WIDTH, col_height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                self.render_list_view_compact(ui);
                            },
                        );
                        if self.is_item_dragging {
                            self.drag_target_folder = column_drop_target
                                .as_ref()
                                .filter(|target| self.is_valid_drop_target(target))
                                .cloned();
                        }
                    });
                })
        });
        self.miller_columns
            .set_horizontal_scroll_offset(scroll_output.inner.state.offset.x);

        if let Some(active_source) = self
            .rectangle_selection_state
            .as_ref()
            .map(|state| &state.source)
            .filter(|source| matches!(source, RectangleSelectionSource::MillerAncestor { .. }))
        {
            let source_is_visible = ancestor_outputs
                .iter()
                .any(|(_, output)| &output.rectangle_selection_frame.source == active_source);
            if !source_is_visible {
                self.cancel_rectangle_selection();
            }
        }

        for path in icon_requests {
            let _ = self.request_icon_load(path, crate::domain::file_entry::IconSize::Large);
        }

        if self.context_menu.is_open
            && self
                .rectangle_selection_state
                .as_ref()
                .is_some_and(|state| {
                    matches!(
                        state.source,
                        RectangleSelectionSource::MillerAncestor { .. }
                    )
                })
        {
            self.cancel_rectangle_selection();
        }

        interactions::apply_ancestor_interactions(
            self,
            ui,
            &mut ancestor_outputs,
            &ancestor_data,
            &chain,
            &mut pending_nav,
        );

        if let Some(target) = pending_nav {
            self.activate_location(&target);
            ui.ctx().request_repaint();
        }
    }
}
