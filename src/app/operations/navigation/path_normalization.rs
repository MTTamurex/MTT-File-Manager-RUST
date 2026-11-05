use crate::domain::special_paths::is_virtual_path;

/// Normalize before storing navigation state or creating filesystem/cache paths.
/// Directory enumeration accepts `/`, but Shell item parsing may reject it.
pub(super) fn normalize_navigation_path(path: &str) -> String {
    if is_virtual_path(path) {
        return path.to_string();
    }

    let path = path.replace('/', "\\");
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        if bytes.len() == 2 {
            return format!("{path}\\");
        }
        if bytes[2] != b'\\' {
            return format!("{}\\{}", &path[..2], &path[2..]);
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::file_entry::FileEntry;
    use crate::infrastructure::directory_cache::DirectoryCache;
    use std::path::PathBuf;

    #[test]
    fn normalizes_forward_and_mixed_separators() {
        for input in [r"C:/Example/Temp", r"C:\Example/Temp", r"C:\Example\Temp"] {
            assert_eq!(normalize_navigation_path(input), r"C:\Example\Temp");
        }
    }

    #[test]
    fn forward_slash_after_drive_does_not_add_an_extra_separator() {
        assert_eq!(normalize_navigation_path("C:/"), r"C:\");
        assert_eq!(normalize_navigation_path("C:/Example"), r"C:\Example");
    }

    #[test]
    fn retains_drive_root_and_drive_relative_handling() {
        assert_eq!(normalize_navigation_path("C:"), r"C:\");
        assert_eq!(normalize_navigation_path(r"C:Example"), r"C:\Example");
    }

    #[test]
    fn preserves_virtual_views() {
        for input in ["::computer::", "::recycle_bin::", "::tag::9"] {
            assert_eq!(normalize_navigation_path(input), input);
        }
    }

    #[test]
    fn preserves_unc_and_extended_path_prefixes() {
        for (input, expected) in [
            ("//server/share/Temp", r"\\server\share\Temp"),
            (r"\\server\share/Temp", r"\\server\share\Temp"),
            (r"\\?\C:\Example/Temp", r"\\?\C:\Example\Temp"),
            (r"\\?\UNC\server\share/Temp", r"\\?\UNC\server\share\Temp"),
        ] {
            assert_eq!(normalize_navigation_path(input), expected);
            assert_eq!(normalize_navigation_path(expected), expected);
        }
    }

    #[test]
    fn expanded_environment_path_is_normalized_before_child_paths_are_created() {
        let expanded = r"%LOCALAPPDATA%/Temp".replace("%LOCALAPPDATA%", r"C:\Example\Local");
        let folder = PathBuf::from(normalize_navigation_path(&expanded));
        assert_eq!(
            folder.join("file.txt"),
            PathBuf::from(r"C:\Example\Local\Temp\file.txt")
        );
    }

    #[test]
    fn cached_children_use_backslashes_regardless_of_first_input_form() {
        for inputs in [
            [r"C:\Example/Temp", r"C:\Example\Temp"],
            [r"C:\Example\Temp", r"C:\Example/Temp"],
        ] {
            let cache = DirectoryCache::new();
            let first = PathBuf::from(normalize_navigation_path(inputs[0]));
            cache.put(
                first.clone(),
                vec![FileEntry::from_path(first.join("file.txt"), false)],
            );

            let second = PathBuf::from(normalize_navigation_path(inputs[1]));
            let entries = cache.get(&second).expect("same normalized cache key");
            assert_eq!(
                entries[0].path.as_os_str(),
                std::ffi::OsStr::new(r"C:\Example\Temp\file.txt")
            );
        }
    }
}
