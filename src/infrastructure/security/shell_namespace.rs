use std::path::Path;

/// Classification for paths that represent explicit Shell Namespace identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellNamespacePathKind {
    ShellUri,
    GuidIdentifier,
}

fn normalize_shell_input<'a>(raw: &'a str) -> &'a str {
    let trimmed = raw.trim();

    trimmed
        .strip_prefix(r"\\?\")
        .or_else(|| trimmed.strip_prefix(r"\\.\"))
        .unwrap_or(trimmed)
}

/// Parses a path string and returns a typed shell-namespace classification.
///
/// This only accepts explicit namespace identifiers and intentionally rejects
/// heuristic/archive-like filesystem paths.
pub fn classify_shell_namespace_str(raw: &str) -> Option<ShellNamespacePathKind> {
    let normalized = normalize_shell_input(raw);
    if normalized.is_empty() {
        return None;
    }

    if normalized
        .get(..6)
        .map(|prefix| prefix.eq_ignore_ascii_case("shell:"))
        .unwrap_or(false)
    {
        return Some(ShellNamespacePathKind::ShellUri);
    }

    if normalized.starts_with("::") {
        return Some(ShellNamespacePathKind::GuidIdentifier);
    }

    None
}

/// Parses a [`Path`](std::path::Path) and returns typed shell namespace classification.
pub fn classify_shell_namespace_path(path: &Path) -> Option<ShellNamespacePathKind> {
    classify_shell_namespace_str(&path.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_shell_uri_case_insensitive() {
        assert_eq!(
            classify_shell_namespace_str("shell:RecycleBinFolder"),
            Some(ShellNamespacePathKind::ShellUri)
        );
        assert_eq!(
            classify_shell_namespace_str("ShElL:AppsFolder"),
            Some(ShellNamespacePathKind::ShellUri)
        );
    }

    #[test]
    fn classifies_guid_identifier_with_or_without_verbatim_prefix() {
        assert_eq!(
            classify_shell_namespace_str("::{645FF040-5081-101B-9F08-00AA002F954E}"),
            Some(ShellNamespacePathKind::GuidIdentifier)
        );
        assert_eq!(
            classify_shell_namespace_str(r"\\?\::{645FF040-5081-101B-9F08-00AA002F954E}"),
            Some(ShellNamespacePathKind::GuidIdentifier)
        );
    }

    #[test]
    fn rejects_regular_filesystem_paths_and_archive_like_paths() {
        assert_eq!(classify_shell_namespace_str(r"C:\temp\file.txt"), None);
        assert_eq!(classify_shell_namespace_str(r"C:\temp\archive.zip\foo"), None);
        assert_eq!(classify_shell_namespace_str(""), None);
    }
}
