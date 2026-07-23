//! Layout geometry for the Miller's Columns strip.

use std::path::{Path, PathBuf};

/// Width of an ancestor column (compact icon + name list).
pub const ANCESTOR_COL_WIDTH: f32 = 240.0;
/// Width of the focused (rightmost) column. Slightly wider than ancestor
/// columns to emphasize focus; renders the current directory (name-only).
pub const FOCUSED_COL_WIDTH: f32 = 280.0;
/// Row height for the compact ancestor columns.
pub const COL_ROW_HEIGHT: f32 = 24.0;

/// Compute the ancestor chain from the drive root down to `current_path`
/// (inclusive), root first. Empty components are dropped.
pub fn ancestor_chain(current_path: &str) -> Vec<PathBuf> {
    let mut chain: Vec<PathBuf> = Path::new(current_path)
        .ancestors()
        .map(Path::to_path_buf)
        .filter(|p| !p.as_os_str().is_empty())
        .collect();
    chain.reverse();
    chain
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ancestor_chain_is_root_first() {
        let chain = ancestor_chain(r"C:\A\B\C");
        assert_eq!(
            chain.first().map(|p| p.to_string_lossy().into_owned()),
            Some(r"C:\".to_string())
        );
        assert_eq!(
            chain.last().map(|p| p.to_string_lossy().into_owned()),
            Some(r"C:\A\B\C".to_string())
        );
        assert_eq!(chain.len(), 4);
    }

    #[test]
    fn drive_root_is_single_column() {
        assert_eq!(ancestor_chain(r"C:\").len(), 1);
    }
}
