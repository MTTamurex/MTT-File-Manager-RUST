use std::sync::Arc;

use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::FileEntry;
use crate::ui::views::miller_columns_view::{MillerColumnAction, MillerColumnOutput};

pub(super) fn finish_rename_for_click_away(
    app: &mut ImageViewerApp,
    outputs: &mut [(usize, MillerColumnOutput)],
    ancestor_data: &[(Option<Arc<Vec<FileEntry>>>, bool)],
) -> bool {
    let rename_path = app.renaming_state.as_ref().map(|(path, _)| path.clone());
    let clicked_away = rename_path.as_ref().is_some_and(|rename_path| {
        outputs.iter().any(|(column_index, output)| {
            if output.toggled_group.is_some() {
                return true;
            }
            let Some(item_index) = output.action.as_ref().and_then(clicked_item_index) else {
                return false;
            };
            ancestor_data
                .get(*column_index)
                .and_then(|(listing, _)| listing.as_ref())
                .and_then(|listing| listing.get(item_index))
                .is_some_and(|item| item.path != *rename_path)
        })
    });

    let active_update = outputs.iter_mut().find_map(|(_, output)| {
        let update = output.rename_update.take()?;
        app.renaming_state
            .as_ref()
            .is_some_and(|(path, _)| *path == update.path)
            .then_some(update)
    });
    let Some((path, current_text)) = app.renaming_state.take() else {
        return false;
    };
    let update = active_update.filter(|update| update.path == path);
    let should_commit = clicked_away || update.as_ref().is_some_and(|update| update.commit);
    let text = update.map(|update| update.text).unwrap_or(current_text);

    if !should_commit {
        app.renaming_state = Some((path, text));
        return false;
    }

    app.focus_rename = false;
    if app.can_rename_path(&path) {
        app.rename_path_with_shell(path, text);
    }
    clicked_away
}

fn clicked_item_index(action: &MillerColumnAction) -> Option<usize> {
    match action {
        MillerColumnAction::Clicked(index)
        | MillerColumnAction::DoubleClicked(index)
        | MillerColumnAction::SecondaryClicked(index, _) => Some(*index),
        MillerColumnAction::DragStarted(_)
        | MillerColumnAction::EmptyClicked
        | MillerColumnAction::EmptySecondaryClicked(_) => None,
    }
}
