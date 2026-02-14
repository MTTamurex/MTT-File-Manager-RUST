use crate::app::global_search_state::GlobalSearchCategory;

pub(super) fn category_label(category: GlobalSearchCategory) -> &'static str {
    match category {
        GlobalSearchCategory::All => "Tudo",
        GlobalSearchCategory::Files => "Arquivos",
        GlobalSearchCategory::Folders => "Pastas",
        GlobalSearchCategory::Images => "Imagens",
        GlobalSearchCategory::Videos => "Videos",
        GlobalSearchCategory::Audio => "Audio",
        GlobalSearchCategory::Documents => "Documentos",
    }
}

pub(super) fn build_filtered_indices(
    results: &[mtt_search_protocol::SearchResultItem],
    category: GlobalSearchCategory,
    drive_filter: Option<char>,
) -> Vec<usize> {
    let mut filtered = Vec::with_capacity(results.len());

    for (idx, result) in results.iter().enumerate() {
        if let Some(drive) = drive_filter {
            if extract_drive_letter(&result.full_path) != Some(drive) {
                continue;
            }
        }

        if matches_category(result, category) {
            filtered.push(idx);
        }
    }

    filtered
}

pub(super) fn available_drives(results: &[mtt_search_protocol::SearchResultItem]) -> Vec<char> {
    let mut drives: Vec<char> = results
        .iter()
        .filter_map(|r| extract_drive_letter(&r.full_path))
        .collect();
    drives.sort_unstable();
    drives.dedup();
    drives
}

pub(super) fn extract_drive_letter(path: &str) -> Option<char> {
    use std::path::{Component, Path, Prefix};

    // Accept regular and verbatim Windows paths:
    // - C:\foo
    // - \\?\C:\foo
    // - \\.\C:\foo
    if let Some(Component::Prefix(prefix_component)) = Path::new(path).components().next() {
        match prefix_component.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                return Some((letter as char).to_ascii_uppercase());
            }
            _ => {}
        }
    }

    // Fallback for uncommon string forms (e.g., slashes normalized by other layers).
    let normalized = path
        .strip_prefix(r"\\?\")
        .or_else(|| path.strip_prefix(r"\\.\"))
        .or_else(|| path.strip_prefix("//?/"))
        .or_else(|| path.strip_prefix("//./"))
        .or_else(|| path.strip_prefix(r"\??\"))
        .unwrap_or(path);

    let mut chars = normalized.chars();
    let drive = chars.next()?.to_ascii_uppercase();
    if drive.is_ascii_alphabetic() && chars.next() == Some(':') {
        return Some(drive);
    }

    None
}

fn matches_category(
    result: &mtt_search_protocol::SearchResultItem,
    category: GlobalSearchCategory,
) -> bool {
    match category {
        GlobalSearchCategory::All => true,
        GlobalSearchCategory::Files => !result.is_dir,
        GlobalSearchCategory::Folders => result.is_dir,
        GlobalSearchCategory::Images => extension_in(
            &result.full_path,
            &[
                "jpg", "jpeg", "png", "gif", "bmp", "webp", "tiff", "tif", "svg", "heic", "avif",
                "ico",
            ],
        ),
        GlobalSearchCategory::Videos => extension_in(
            &result.full_path,
            &[
                "mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v", "mpeg", "mpg", "ts",
            ],
        ),
        GlobalSearchCategory::Audio => extension_in(
            &result.full_path,
            &["mp3", "wav", "flac", "aac", "ogg", "wma", "m4a", "opus"],
        ),
        GlobalSearchCategory::Documents => extension_in(
            &result.full_path,
            &[
                "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "md", "rtf", "odt",
                "csv",
            ],
        ),
    }
}

fn extension_in(path: &str, allowed: &[&str]) -> bool {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());

    let Some(ext) = ext else {
        return false;
    };

    allowed.iter().any(|candidate| *candidate == ext)
}

pub(super) fn format_number(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::extract_drive_letter;

    #[test]
    fn extract_drive_letter_accepts_regular_windows_path() {
        assert_eq!(extract_drive_letter(r"C:\Users\foo.txt"), Some('C'));
        assert_eq!(extract_drive_letter(r"z:\vault\file.docx"), Some('Z'));
    }

    #[test]
    fn extract_drive_letter_accepts_verbatim_windows_path() {
        assert_eq!(extract_drive_letter(r"\\?\D:\data\file.bin"), Some('D'));
        assert_eq!(extract_drive_letter(r"\\.\E:\media\movie.mkv"), Some('E'));
    }

    #[test]
    fn extract_drive_letter_rejects_non_drive_paths() {
        assert_eq!(extract_drive_letter(r"\\server\share\file.txt"), None);
        assert_eq!(extract_drive_letter(r"/home/user/file.txt"), None);
    }
}
