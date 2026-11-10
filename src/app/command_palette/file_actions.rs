//! Explicit active-pane targets: never use context-menu fallbacks or the current folder as a source.
use super::catalog::Command;
use crate::app::ImageViewerApp;
use std::path::PathBuf;

pub(super) fn selected_paths(app: &ImageViewerApp) -> Vec<PathBuf> {
    selection_paths(
        app.selected_file.as_ref().map(|item| &item.path),
        app.multi_selection.iter(),
    )
}

fn selection_paths<'a>(
    focused: Option<&PathBuf>,
    selected: impl Iterator<Item = &'a PathBuf>,
) -> Vec<PathBuf> {
    let mut paths: Vec<_> = selected.cloned().collect();
    if paths.is_empty() {
        paths.extend(focused.cloned());
    }
    paths
}

pub(super) fn supported_targets(paths: &[PathBuf]) -> bool {
    !paths.is_empty()
        && paths.iter().all(|path| {
            !path.as_os_str().is_empty()
                && !(path.has_root() && path.parent().is_none())
                && !crate::infrastructure::windows::is_drive_root_path(path)
                && !crate::domain::network_path::is_network_location_root(path)
                && !crate::domain::special_paths::is_virtual_path(&path.to_string_lossy())
        })
}

pub(super) fn execute(app: &mut ImageViewerApp, command: Command) {
    match command {
        Command::Copy => app.copy_paths_to_clipboard(&selected_paths(app)),
        Command::Cut => app.cut_paths_to_clipboard(&selected_paths(app)),
        Command::Delete => {
            app.delete_with_shell_for_paths(&selected_paths(app));
        }
        Command::Paste => {
            // command_paste(None) targets the current folder only when no stale
            // right-click targets are present. Do not retarget to a selected subfolder.
            app.context_menu.target_paths.clear();
            app.command_paste(None);
        }
        _ => unreachable!("only clipboard and delete commands are dispatched here"),
    }
}

#[cfg(test)]
mod tests;
