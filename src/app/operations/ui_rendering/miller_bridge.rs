//! Miller's Columns bridge - orchestrates the multi-column view.
//!
//! The rightmost (focused) column is `current_path` and reuses the full
//! details list view (`render_list_view`) for complete interaction parity
//! (rename, multi-select, drag, rectangle selection, keyboard, context menu).
//! Ancestor columns to its left are rendered from a lightweight background
//! listing cache (`MillerColumnsState`) and support select / open / context
//! menu via path-based actions. Clicking an ancestor folder navigates into it
//! (promoting it to the focused column) — matching a Finder-style column view.

use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::FileEntry;
use crate::domain::special_paths::is_virtual_path;
use crate::ui::views::miller_columns_view::{
    ancestor_chain, render_miller_column, MillerColumnAction, MillerColumnContext,
    ANCESTOR_COL_WIDTH, FOCUSED_COL_WIDTH,
};
use crate::ui::views::rectangle_selection::RectangleSelectionSource;

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
            && self.renaming_state.is_none()
            && self.rectangle_selection_state.is_none()
            && !ui.ctx().wants_keyboard_input();
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
                self.navigate_to(&target);
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
        let scroll_output = ui.scope(|ui| {
            // The global scrollbar style is fully transparent while dormant.
            // Keep this navigation scrollbar visible whenever it is needed.
            ui.style_mut().spacing.scroll.dormant_handle_opacity = 0.4;

            egui::ScrollArea::horizontal()
                .id_salt("miller_strip")
                .auto_shrink([false, false])
                .drag_to_scroll(false)
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
                            let listing_id = listing
                                .as_ref()
                                .map(|listing| Arc::as_ptr(listing) as usize)
                                .unwrap_or(0);
                            let selection_source = RectangleSelectionSource::MillerAncestor {
                                directory: chain[col_idx].clone(),
                                listing_id,
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
                                        directory: &chain[col_idx],
                                        selected_child,
                                        selected_file: selected_file_path.as_deref(),
                                        multi_selection: &selected_paths,
                                        rectangle_selection_state: column_rectangle_selection,
                                        listing_id,
                                        icon_loader: &mut self.item_icon_loader,
                                        folder_icon: folder_icon.as_ref(),
                                        loading_icons: &self.loading_icons,
                                        failed_icons: &self.failed_icons,
                                        icon_requests: &mut icon_requests,
                                        is_item_dragging: self.is_item_dragging,
                                        drop_target: &mut column_drop_target,
                                        is_loading: *loading,
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
            self.request_icon_load(path);
        }

        // ── Apply deferred ancestor interactions. ──
        let right_bound = viewport.right();
        for (col_idx, output) in ancestor_outputs {
            let Some(listing) = ancestor_data[col_idx].0.as_ref() else {
                continue;
            };
            self.handle_miller_rectangle_selection_frame(
                ui,
                &output.rectangle_selection_frame,
                listing,
            );
            let Some(action) = output.action else {
                continue;
            };
            match action {
                MillerColumnAction::Clicked(i) => {
                    if let Some(entry) = listing.get(i).cloned() {
                        if entry.is_dir {
                            // Anti-collapse: clicking the child already in the
                            // chain keeps the deeper columns intact.
                            let already = chain
                                .get(col_idx + 1)
                                .map(|c| c.as_path() == entry.path.as_path())
                                .unwrap_or(false);
                            if !already {
                                pending_nav = Some(entry.path.to_string_lossy().to_string());
                            }
                        } else {
                            self.select_ancestor_entry_for_preview(entry);
                        }
                    }
                }
                MillerColumnAction::DoubleClicked(i) => {
                    if let Some(entry) = listing.get(i).cloned() {
                        if entry.is_dir {
                            pending_nav = Some(entry.path.to_string_lossy().to_string());
                        } else {
                            self.open_with_shell_guarded(&entry.path);
                        }
                    }
                }
                MillerColumnAction::SecondaryClicked(i, pos) => {
                    if let Some(entry) = listing.get(i).cloned() {
                        if !self.multi_selection.contains(&entry.path) {
                            self.select_ancestor_entry_for_preview(entry.clone());
                        } else {
                            self.selected_item = None;
                            self.selection_anchor = None;
                            self.selected_file = Some(entry.clone());
                            self.update_selected_thumbnail();
                        }
                        let mut paths = vec![entry.path.clone()];
                        paths.extend(
                            listing
                                .iter()
                                .filter(|item| {
                                    item.path != entry.path
                                        && self.multi_selection.contains(&item.path)
                                })
                                .map(|item| item.path.clone()),
                        );
                        self.context_menu
                            .open(pos, right_bound, None, paths.clone(), false);
                        self.context_menu.primary_is_directory = Some(entry.is_dir);
                        self.populate_context_menu(ui.ctx(), &paths, false, None);
                    }
                }
                MillerColumnAction::DragStarted(i) => {
                    if let Some(entry) = listing.get(i).cloned() {
                        self.begin_miller_ancestor_drag(entry, listing);
                    }
                }
                MillerColumnAction::EmptySecondaryClicked(pos) => {
                    let paths = vec![chain[col_idx].clone()];
                    self.context_menu
                        .open(pos, right_bound, None, paths.clone(), true);
                    self.context_menu.primary_is_directory = Some(true);
                    self.populate_context_menu(ui.ctx(), &paths, true, None);
                }
            }
        }

        if let Some(target) = pending_nav {
            self.navigate_to(&target);
            ui.ctx().request_repaint();
        }
    }
}
