//! Text-only path copying; distinct from file copy/cut clipboard payloads.
use super::{sanitize_operation_path, OpResult};
use clipboard_win::{formats, Clipboard, Setter};
use std::path::{Path, PathBuf};

fn write_text(text: &str) -> OpResult<()> {
    if let Ok(_clipboard) = Clipboard::new_attempts(10) {
        if formats::Unicode.write_clipboard(&text).is_ok() {
            return Ok(());
        }
    }
    Err(rust_i18n::t!("operations.error_clipboard").to_string())
}

/// Retains the unquoted single-path API used by existing callers and file-copy fallbacks.
pub fn copy_path_to_clipboard(path: &Path) -> OpResult<()> {
    let path = sanitize_operation_path(path)?;
    write_text(&path.to_string_lossy())
}

fn quoted_paths_text(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| format!("\"{}\"", path.to_string_lossy()))
        .collect::<Vec<_>>()
        .join("\r\n")
}

fn prepare_paths_text(paths: &[PathBuf]) -> OpResult<String> {
    let validated = paths
        .iter()
        .map(|path| sanitize_operation_path(path))
        .collect::<OpResult<Vec<_>>>()?;
    Ok(quoted_paths_text(&validated))
}

/// Explorer-style Copy path: one quoted path per Windows text line.
pub fn copy_paths_as_text_to_clipboard(paths: &[PathBuf]) -> OpResult<()> {
    if paths.is_empty() {
        return Ok(());
    }
    // Validate the entire selection before opening (and potentially replacing) the clipboard.
    let text = prepare_paths_text(paths)?;
    write_text(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_path_text_includes_all_origins_in_selection_order() {
        let paths = [
            PathBuf::from(r"C:\Folder\file.txt"),
            PathBuf::from(r"D:\Other\file.txt"),
        ];
        assert_eq!(
            quoted_paths_text(&paths),
            "\"C:\\Folder\\file.txt\"\r\n\"D:\\Other\\file.txt\""
        );
    }

    #[test]
    fn copy_path_text_quotes_single_items_and_preserves_spaces_unicode_and_unc() {
        for path in [
            r"C:\Folder Name\arquivo ação.txt",
            r"\\server\share\Folder Name\file.txt",
        ] {
            assert_eq!(
                quoted_paths_text(&[PathBuf::from(path)]),
                format!("\"{path}\"")
            );
        }
    }

    #[test]
    fn copy_path_text_validates_the_whole_selection_before_writing() {
        let folder = tempfile::tempdir().unwrap();
        let paths = [folder.path().to_path_buf(), PathBuf::from("invalid\0.txt")];
        assert!(prepare_paths_text(&paths).is_err());
    }

    #[test]
    fn empty_path_copy_does_not_access_or_replace_the_clipboard() {
        assert_eq!(quoted_paths_text(&[]), "");
        assert!(copy_paths_as_text_to_clipboard(&[]).is_ok());
    }
}
