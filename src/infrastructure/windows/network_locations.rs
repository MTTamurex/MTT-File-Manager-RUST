//! Read saved filesystem network locations locally; never probe the remote targets.

use super::shortcuts::resolve_shortcut_target;
use crate::domain::network_location::{
    network_location_path_key, NetworkLocation, NetworkLocationKind,
};
use crate::domain::network_path::unc_server_name;
use std::path::{Component, Path, PathBuf, Prefix};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_NetHood, SHGetKnownFolderPath, KF_FLAG_DONT_VERIFY};

pub fn get_network_locations() -> Vec<NetworkLocation> {
    network_locations_folder()
        .map(|path| read_network_locations(&path, resolve_shortcut_target))
        .unwrap_or_default()
}

fn network_locations_folder() -> Option<PathBuf> {
    let folder = unsafe { SHGetKnownFolderPath(&FOLDERID_NetHood, KF_FLAG_DONT_VERIFY, None) };
    let Ok(folder) = folder else {
        return None;
    };
    let path = unsafe { folder.to_string() }.ok().map(PathBuf::from);
    unsafe { CoTaskMemFree(Some(folder.0.cast())) };
    path
}

/// Resolve only saved entries directly inside the user's Network Locations folder.
/// Read the local shortcut without querying the destination's availability.
pub fn resolve_network_location_target(path: &Path) -> Option<PathBuf> {
    resolve_saved_location(&network_locations_folder()?, path, resolve_shortcut_target)
}

/// Used only for activations already identified by Windows as Folder Shortcuts,
/// including network shortcuts copied outside the Network Locations folder.
pub(crate) fn resolve_folder_shortcut_network_target(path: &Path) -> Option<PathBuf> {
    resolve_saved_location(path.parent()?, path, resolve_shortcut_target)
}

fn resolve_saved_location(
    folder: &Path,
    path: &Path,
    mut resolve: impl FnMut(&Path) -> Option<PathBuf>,
) -> Option<PathBuf> {
    if network_location_path_key(path.parent()?) != network_location_path_key(folder) {
        return None;
    }
    // Folder Shortcut is a directory, even when its display name ends in .lnk.
    resolve(&path.join("target.lnk"))
        .filter(|target| is_network_target(target))
        .or_else(|| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"))
                .then(|| resolve(path).filter(|target| is_network_target(target)))
                .flatten()
        })
}

pub(crate) fn is_network_target(path: &Path) -> bool {
    if unc_server_name(&path.to_string_lossy()).is_some() {
        return true;
    }
    matches!(path.components().next(), Some(Component::Prefix(prefix))
        if matches!(prefix.kind(), Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share)
            if !server.is_empty() && !share.is_empty()))
}

fn read_network_locations(
    folder: &Path,
    mut resolve: impl FnMut(&Path) -> Option<PathBuf>,
) -> Vec<NetworkLocation> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut locations = Vec::new();
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        let (shortcut, label) = if kind.is_dir() {
            (
                path.join("target.lnk"),
                entry.file_name().to_string_lossy().into_owned(),
            )
        } else if kind.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"))
        {
            (
                path.clone(),
                path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            )
        } else {
            continue;
        };
        let Some(target) = resolve(&shortcut).filter(|target| is_network_target(target)) else {
            continue;
        };
        locations.push(NetworkLocation {
            shortcut_path: path.to_string_lossy().into_owned(),
            path: target.to_string_lossy().into_owned(),
            label,
            kind: if kind.is_dir() {
                NetworkLocationKind::FolderShortcut
            } else {
                NetworkLocationKind::LinkFile
            },
        });
    }
    locations.sort_by(|a, b| {
        a.label
            .to_lowercase()
            .cmp(&b.label.to_lowercase())
            .then_with(|| a.path.cmp(&b.path))
    });
    locations
}

#[cfg(test)]
#[path = "network_locations_tests.rs"]
mod tests;
