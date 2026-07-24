use eframe::egui;
use std::path::{Path, PathBuf};

use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::FileEntry;
use crate::ui::cache::FxHashSet;

impl ImageViewerApp {
    pub(super) fn update_miller_ancestor_selected_file(&mut self, entry: FileEntry) {
        self.selected_file = Some(entry.clone());
        self.update_selected_thumbnail();
        self.ensure_detail_panel_thumbnail_for_file(&entry);
    }

    /// Select an entry shown outside the focused column for preview and actions.
    pub(super) fn select_ancestor_entry_for_preview(&mut self, entry: FileEntry) {
        self.selected_item = None;
        self.selection_anchor = None;
        self.multi_selection.clear();
        self.multi_selection.insert(entry.path.clone());
        self.update_miller_ancestor_selected_file(entry);
    }

    pub(super) fn select_miller_ancestor_entry(
        &mut self,
        directory: &Path,
        listing: &[FileEntry],
        index: usize,
        ctrl: bool,
        shift: bool,
    ) {
        let Some(entry) = listing.get(index).cloned() else {
            return;
        };
        let listing_paths: FxHashSet<_> = listing.iter().map(|item| item.path.clone()).collect();
        let anchor = self
            .miller_columns
            .selection_anchor_index(directory, listing);

        self.multi_selection
            .retain(|path| listing_paths.contains(path));
        if ctrl {
            if !self.multi_selection.remove(&entry.path) {
                self.multi_selection.insert(entry.path.clone());
            }
            self.miller_columns
                .set_selection_anchor(directory, &entry.path);
        } else if shift {
            if let Some(anchor) = anchor {
                let (start, end) = if anchor < index {
                    (anchor, index)
                } else {
                    (index, anchor)
                };
                for item in &listing[start..=end] {
                    self.multi_selection.insert(item.path.clone());
                }
            } else {
                self.multi_selection.clear();
                self.multi_selection.insert(entry.path.clone());
                self.miller_columns
                    .set_selection_anchor(directory, &entry.path);
            }
        } else {
            self.multi_selection.clear();
            self.multi_selection.insert(entry.path.clone());
            self.miller_columns
                .set_selection_anchor(directory, &entry.path);
        }

        self.selected_item = None;
        self.selection_anchor = None;
        self.update_miller_ancestor_selected_file(entry);
        self.ui_ctx.request_repaint();
    }

    pub(super) fn begin_miller_ancestor_drag(&mut self, entry: FileEntry, listing: &[FileEntry]) {
        let preserve_selection = self.multi_selection.contains(&entry.path);
        let payload_entries: Vec<&FileEntry> = if preserve_selection {
            listing
                .iter()
                .filter(|item| self.multi_selection.contains(&item.path))
                .collect()
        } else {
            vec![&entry]
        };
        let moving_open_ancestor = payload_entries.iter().any(|item| {
            !crate::domain::file_entry::is_path_inside_existing_archive_file(&item.path)
                && self.path_is_same_or_ancestor_of_open_panel(&item.path)
        });
        if moving_open_ancestor
            || self.is_item_dragging
            || self.file_panel_input_blocked_by_drag_move_confirmation()
            || self.outbound_drag_input_guard
                != crate::app::drag_drop_state::OutboundDragInputGuard::Inactive
            || !self
                .ui_ctx
                .input(|input| input.pointer.button_down(egui::PointerButton::Primary))
        {
            return;
        }

        let source_folder = entry.path.parent().map(Path::to_path_buf);
        let paths: Vec<PathBuf> = payload_entries
            .iter()
            .map(|item| item.path.clone())
            .collect();
        if !preserve_selection {
            self.multi_selection.clear();
            self.multi_selection.insert(entry.path.clone());
        }
        self.selected_item = None;
        self.selection_anchor = None;
        self.update_miller_ancestor_selected_file(entry.clone());

        self.is_item_dragging = true;
        self.item_drag_origin = crate::app::drag_drop_state::ItemDragOrigin::FileView;
        self.drag_payload_paths = paths;
        self.drag_payload_is_single_directory = self.drag_payload_paths.len() == 1 && entry.is_dir;
        self.drag_source_folder = source_folder;
        self.drag_source_cross_panel_context = self.drag_drop_cross_panel_context;
        self.drag_target_folder = None;
        self.drag_hovered_folder = None;
        self.drag_cross_panel_target = None;

        let ui_ctx = self.ui_ctx.clone();
        self.drag_icon_cache = if entry.is_dir && !entry.is_archive() {
            self.item_icon_loader
                .get_or_load_icon(&ui_ctx, &entry.path, true, true)
                .or_else(|| self.cache_manager.folder_icon_texture.clone())
        } else if entry.is_media() {
            self.cache_manager
                .texture_cache
                .get(&entry.path)
                .cloned()
                .or_else(|| {
                    self.item_icon_loader
                        .get_or_load_icon(&ui_ctx, &entry.path, false, true)
                })
        } else {
            self.item_icon_loader
                .get_or_load_icon(&ui_ctx, &entry.path, false, true)
        };
        self.ui_ctx.request_repaint();
    }
}
