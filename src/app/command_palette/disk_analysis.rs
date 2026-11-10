//! Resolve only the current pane's location, never a selected drive or an old menu target.
use crate::app::ImageViewerApp;
use crate::application::disk_analysis_policy::supports_disk_analysis;
use crate::domain::file_entry::DriveInfo;
use std::path::{Component, Path, Prefix};

pub(super) fn current_drive(app: &ImageViewerApp) -> Option<char> {
    drive_for_path(Path::new(&app.navigation_state.current_path), |root| {
        app.drive_state.cached_drive_info(root)
    })
}

fn drive_for_path(
    path: &Path,
    cached_info: impl FnOnce(&str) -> Option<DriveInfo>,
) -> Option<char> {
    let mut components = path.components();
    let Component::Prefix(prefix) = components.next()? else {
        return None;
    };
    let letter = match prefix.kind() {
        Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) if letter.is_ascii_alphabetic() => {
            (letter as char).to_ascii_uppercase()
        }
        _ => return None,
    };
    if components.next() != Some(Component::RootDir) {
        return None;
    }
    let root = format!("{letter}:\\");
    // UI eligibility uses the existing asynchronous cache, not synchronous
    // volume queries on every frame. Missing metadata fails closed until refreshed.
    let info = cached_info(&root)?;
    supports_disk_analysis(Path::new(&root), info.drive_type, &info.file_system).then_some(letter)
}

#[cfg(test)]
mod tests;
