//! Per-launch working directory for the default Shell open action.

use std::path::Path;

use crate::infrastructure::security::classify_shell_namespace_path;
use crate::infrastructure::windows::shortcuts::fast_is_directory;

pub(super) fn for_shell_open(path: &Path) -> Option<&Path> {
    // Virtual objects have no filesystem CWD. Relative inputs must not turn
    // lpDirectory into another path relative to the file manager's CWD.
    if !path.is_absolute() || classify_shell_namespace_path(path).is_some() {
        return None;
    }

    // Let the Shell honor a shortcut's own "Start in" setting. Folder opening
    // also keeps its existing Shell behavior rather than using its parent.
    if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"))
        || fast_is_directory(path)
    {
        return None;
    }

    path.parent()
}

#[cfg(test)]
#[path = "working_directory_tests.rs"]
mod tests;
