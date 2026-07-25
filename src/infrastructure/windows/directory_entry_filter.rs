use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM};

pub(crate) fn should_include_directory_entry(
    name: &str,
    attributes: u32,
    show_hidden: bool,
) -> bool {
    let is_hidden = (attributes & FILE_ATTRIBUTE_HIDDEN.0) != 0;
    let is_system = (attributes & FILE_ATTRIBUTE_SYSTEM.0) != 0;
    let is_special = matches!(
        name.to_lowercase().as_str(),
        "desktop.ini" | "thumbs.db" | "$recycle.bin" | "system volume information"
    );

    (show_hidden || !is_hidden) && !is_system && !is_special
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL};

    #[test]
    fn dot_prefixed_names_are_not_hidden_on_windows() {
        assert!(should_include_directory_entry(
            ".gitconfig",
            FILE_ATTRIBUTE_NORMAL.0,
            false
        ));
        assert!(should_include_directory_entry(
            ".claude.json",
            FILE_ATTRIBUTE_NORMAL.0,
            true
        ));
        assert!(should_include_directory_entry(
            ".local",
            FILE_ATTRIBUTE_DIRECTORY.0,
            false
        ));
    }

    #[test]
    fn hidden_attribute_respects_visibility_setting() {
        assert!(!should_include_directory_entry(
            "normal.txt",
            FILE_ATTRIBUTE_HIDDEN.0,
            false
        ));
        assert!(should_include_directory_entry(
            "normal.txt",
            FILE_ATTRIBUTE_HIDDEN.0,
            true
        ));
        assert!(!should_include_directory_entry(
            ".secret",
            FILE_ATTRIBUTE_HIDDEN.0,
            false
        ));
        assert!(should_include_directory_entry(
            ".secret",
            FILE_ATTRIBUTE_HIDDEN.0,
            true
        ));
    }

    #[test]
    fn system_and_special_entries_remain_excluded() {
        assert!(!should_include_directory_entry(
            "system-file",
            FILE_ATTRIBUTE_SYSTEM.0,
            true
        ));
        assert!(!should_include_directory_entry(
            "desktop.ini",
            FILE_ATTRIBUTE_NORMAL.0,
            true
        ));
        assert!(!should_include_directory_entry(
            "Thumbs.db",
            FILE_ATTRIBUTE_NORMAL.0,
            true
        ));
    }
}
