use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};

/// Caches root availability so a tag view touches each drive or UNC share at
/// most once while loading potentially thousands of paths.
#[derive(Default)]
pub struct RootAvailabilityCache {
    roots: FxHashMap<PathBuf, bool>,
}

impl RootAvailabilityCache {
    pub fn is_root_accessible(&mut self, path: &Path) -> bool {
        let Some(root) = filesystem_root(path) else {
            return false;
        };

        *self
            .roots
            .entry(root.clone())
            .or_insert_with(|| crate::infrastructure::onedrive::fast_path_exists(&root))
    }
}

fn filesystem_root(path: &Path) -> Option<PathBuf> {
    let normalized = crate::domain::file_tag::normalize_tag_path_text(&path.to_string_lossy());
    let bytes = normalized.as_bytes();
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\' {
        return Some(PathBuf::from(format!(
            "{}:\\",
            (bytes[0] as char).to_ascii_uppercase()
        )));
    }

    let rest = normalized.strip_prefix("\\\\")?;
    let mut parts = rest.split('\\');
    let server = parts.next().filter(|part| !part.is_empty())?;
    let share = parts.next().filter(|part| !part.is_empty())?;
    Some(PathBuf::from(format!("\\\\{}\\{}", server, share)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_drive_root_from_normal_and_extended_paths() {
        assert_eq!(
            filesystem_root(Path::new("e:\\folder\\file.txt")),
            Some(PathBuf::from("E:\\"))
        );
        assert_eq!(
            filesystem_root(Path::new("\\\\?\\E:\\folder\\file.txt")),
            Some(PathBuf::from("E:\\"))
        );
    }

    #[test]
    fn extracts_unc_share_root() {
        assert_eq!(
            filesystem_root(Path::new("\\\\server\\share\\folder\\file.txt")),
            Some(PathBuf::from("\\\\server\\share"))
        );
        assert_eq!(
            filesystem_root(Path::new("\\\\?\\UNC\\server\\share\\file.txt")),
            Some(PathBuf::from("\\\\server\\share"))
        );
    }

    #[test]
    fn rejects_paths_without_a_filesystem_root() {
        assert_eq!(filesystem_root(Path::new("relative\\file.txt")), None);
        assert_eq!(filesystem_root(Path::new("::tag::1")), None);
    }

    #[test]
    fn reports_unmounted_drive_root_as_inaccessible() {
        let bitmask = super::super::get_logical_drives_bitmask();
        let Some(letter_index) = (0..26).rev().find(|index| bitmask & (1 << index) == 0) else {
            return;
        };
        let drive_letter = (b'A' + letter_index as u8) as char;
        let path = PathBuf::from(format!("{}:\\tag-test\\file.txt", drive_letter));

        assert!(!RootAvailabilityCache::default().is_root_accessible(&path));
    }
}
