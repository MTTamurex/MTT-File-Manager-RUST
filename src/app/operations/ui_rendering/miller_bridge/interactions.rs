use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;

use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::FileEntry;
use crate::ui::views::miller_columns_view::{MillerColumnAction, MillerColumnOutput};
use crate::ui::views::rectangle_selection::RectangleSelectionSource;

use super::rename_click;

pub(super) fn apply_ancestor_interactions(
    app: &mut ImageViewerApp,
    ui: &mut egui::Ui,
    outputs: &mut [(usize, MillerColumnOutput)],
    ancestor_data: &[(Option<Arc<Vec<FileEntry>>>, bool)],
    chain: &[PathBuf],
    pending_nav: &mut Option<String>,
) {
    let rename_committed_on_click_away =
        rename_click::finish_rename_for_click_away(app, outputs, ancestor_data);

    for (column_index, output) in outputs.iter_mut() {
        let Some(listing) = ancestor_data[*column_index].0.as_ref() else {
            continue;
        };
        app.handle_miller_rectangle_selection_frame(ui, &output.rectangle_selection_frame, listing);
        if app.renaming_state.is_some() {
            continue;
        }
        let Some(action) = output.action.take() else {
            continue;
        };
        if matches!(
            &action,
            MillerColumnAction::SecondaryClicked(_, _)
                | MillerColumnAction::EmptySecondaryClicked(_)
        ) {
            let selection_belongs_to_column =
                app.rectangle_selection_state.as_ref().is_some_and(|state| {
                    matches!(
                        &state.source,
                        RectangleSelectionSource::MillerAncestor { directory }
                            if directory == &chain[*column_index]
                    )
                });
            if selection_belongs_to_column {
                app.finish_miller_rectangle_selection(&chain[*column_index], listing);
            } else {
                app.cancel_rectangle_selection();
            }
        }

        match action {
            MillerColumnAction::Clicked(index) => {
                if let Some(entry) = listing.get(index).cloned() {
                    let (ctrl, shift) =
                        ui.input(|input| (input.modifiers.ctrl, input.modifiers.shift));
                    if entry.is_dir && !ctrl && !shift {
                        // Anti-collapse: clicking the child already in the
                        // chain keeps the deeper columns intact and selects it.
                        let already = chain
                            .get(*column_index + 1)
                            .map(|child| child.as_path() == entry.path.as_path())
                            .unwrap_or(false);
                        if already {
                            app.select_miller_ancestor_entry(
                                &chain[*column_index],
                                listing,
                                index,
                                false,
                                false,
                            );
                        } else {
                            if rename_committed_on_click_away {
                                app.select_miller_ancestor_entry(
                                    &chain[*column_index],
                                    listing,
                                    index,
                                    false,
                                    false,
                                );
                            }
                            *pending_nav = Some(entry.path.to_string_lossy().to_string());
                        }
                    } else {
                        app.select_miller_ancestor_entry(
                            &chain[*column_index],
                            listing,
                            index,
                            ctrl,
                            shift,
                        );
                    }
                }
            }
            MillerColumnAction::DoubleClicked(index) => {
                if let Some(entry) = listing.get(index).cloned() {
                    if entry.is_dir {
                        if rename_committed_on_click_away {
                            app.select_miller_ancestor_entry(
                                &chain[*column_index],
                                listing,
                                index,
                                false,
                                false,
                            );
                        }
                        *pending_nav = Some(entry.path.to_string_lossy().to_string());
                    } else {
                        if rename_committed_on_click_away {
                            app.select_ancestor_entry_for_preview(entry.clone());
                        }
                        app.open_with_shell_guarded(&entry.path);
                    }
                }
            }
            MillerColumnAction::SecondaryClicked(index, position) => {
                if let Some(entry) = listing.get(index).cloned() {
                    if !app.multi_selection.contains(&entry.path) {
                        app.select_ancestor_entry_for_preview(entry.clone());
                    } else {
                        app.selected_item = None;
                        app.selection_anchor = None;
                        app.update_miller_ancestor_selected_file(entry.clone());
                    }
                    let mut paths = vec![entry.path.clone()];
                    paths.extend(
                        listing
                            .iter()
                            .filter(|item| {
                                item.path != entry.path && app.multi_selection.contains(&item.path)
                            })
                            .map(|item| item.path.clone()),
                    );
                    app.context_menu.open(position, None, paths.clone(), false);
                    app.context_menu.primary_is_directory = Some(entry.is_dir);
                    app.context_menu.operation_directory = Some(chain[*column_index].clone());
                    app.capture_context_menu_panel_origin();
                    app.populate_context_menu(ui.ctx(), &paths, false, None);
                }
            }
            MillerColumnAction::DragStarted(index) => {
                if let Some(entry) = listing.get(index).cloned() {
                    app.begin_miller_ancestor_drag(entry, listing);
                }
            }
            MillerColumnAction::EmptyClicked => app.clear_file_view_selection(),
            MillerColumnAction::EmptySecondaryClicked(position) => {
                let paths = vec![chain[*column_index].clone()];
                app.context_menu.open(position, None, paths.clone(), true);
                app.context_menu.primary_is_directory = Some(true);
                app.capture_context_menu_panel_origin();
                app.populate_context_menu(ui.ctx(), &paths, true, None);
            }
        }
    }
}
