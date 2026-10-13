//! Built-in cleanup rule catalog
//!
//! The only source of cleanup category roots. Rules are curated constants —
//! the scanner never accepts roots from user input. Environment variables are
//! resolved at call time so the catalog reflects the current session's user
//! profile and Windows directories.

use crate::domain::cleanup::{CleanupCategory, CleanupCategoryId};
use std::path::PathBuf;

/// Resolves `env_var + relative` into an absolute root. Returns `None` when
/// the variable is unset or the join is not absolute (guards against a
/// hostile/odd environment making a "root" relative or traversal-shaped).
fn env_root(env_var: &str, relative: &str) -> Option<PathBuf> {
    let base = std::env::var_os(env_var)?;
    let joined = PathBuf::from(base).join(relative);
    if joined.is_absolute() && !contains_parent_component(&joined) {
        Some(joined)
    } else {
        None
    }
}

fn contains_parent_component(path: &std::path::Path) -> bool {
    path.components()
        .any(|component| component == std::path::Component::ParentDir)
}

/// Windows directory root (`%SystemRoot%`, falling back to the `WINDIR`
/// variable and then `C:\Windows` as a last resort so headless/service-like
/// environments still resolve).
fn system_root() -> Option<PathBuf> {
    env_root("SystemRoot", "")
        .or_else(|| env_root("windir", ""))
        .filter(|root| root.is_absolute())
}

fn user_local_root(relative: &str) -> Option<PathBuf> {
    env_root("LOCALAPPDATA", relative)
}

fn program_data_root(relative: &str) -> Option<PathBuf> {
    env_root("ProgramData", relative)
}

/// Builds the built-in category list. Roots that cannot be resolved (unset
/// environment variables) are silently omitted — a missing root is never an
/// error, it just yields no candidates.
pub fn builtin_categories() -> Vec<CleanupCategory> {
    let mut categories = Vec::with_capacity(6);

    // %TEMP% — resolved by the CRT/env; fall back to %LOCALAPPDATA%\Temp.
    let user_temp = std::env::var_os("TEMP")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && !contains_parent_component(path))
        .or_else(|| user_local_root("Temp"));
    if let Some(root) = user_temp {
        categories.push(CleanupCategory {
            id: CleanupCategoryId::UserTemp,
            roots: vec![root],
            file_patterns: None,
            requires_admin: false,
        });
    }

    if let Some(root) = system_root().map(|base| base.join("Temp")) {
        categories.push(CleanupCategory {
            id: CleanupCategoryId::SystemTemp,
            roots: vec![root],
            file_patterns: None,
            requires_admin: true,
        });
    }

    if let Some(root) = user_local_root("D3DSCache") {
        categories.push(CleanupCategory {
            id: CleanupCategoryId::DirectXShaderCache,
            roots: vec![root],
            file_patterns: None,
            requires_admin: false,
        });
    }

    let wer_roots: Vec<PathBuf> = ["ReportQueue", "ReportArchive"]
        .iter()
        .filter_map(|name| program_data_root(&format!("Microsoft\\Windows\\WER\\{name}")))
        .collect();
    if !wer_roots.is_empty() {
        categories.push(CleanupCategory {
            id: CleanupCategoryId::ErrorReports,
            roots: wer_roots,
            file_patterns: None,
            requires_admin: false,
        });
    }

    if let Some(root) = user_local_root("CrashDumps") {
        categories.push(CleanupCategory {
            id: CleanupCategoryId::CrashDumps,
            roots: vec![root],
            file_patterns: None,
            requires_admin: false,
        });
    }

    if let Some(root) = user_local_root("Microsoft\\Windows\\Explorer") {
        categories.push(CleanupCategory {
            id: CleanupCategoryId::ExplorerThumbnailCache,
            roots: vec![root],
            // Only Explorer's rebuildable database files; the folder also
            // holds other data that must never be touched.
            file_patterns: Some(vec!["thumbcache_", "iconcache_"]),
            requires_admin: false,
        });
    }

    categories
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_roots_are_absolute_without_parent_components() {
        for category in builtin_categories() {
            assert!(!category.roots.is_empty(), "category without roots");
            for root in &category.roots {
                assert!(root.is_absolute(), "root {root:?} is not absolute");
                assert!(
                    !contains_parent_component(root),
                    "root {root:?} contains '..'"
                );
            }
        }
    }

    #[test]
    fn contains_path_enforces_prefix() {
        let category = CleanupCategory {
            id: CleanupCategoryId::UserTemp,
            roots: vec![PathBuf::from("C:\\Users\\user\\AppData\\Local\\Temp")],
            file_patterns: None,
            requires_admin: false,
        };
        assert!(category.contains_path(std::path::Path::new(
            "C:\\Users\\user\\AppData\\Local\\Temp\\junk.txt"
        )));
        assert!(!category.contains_path(std::path::Path::new(
            "C:\\Users\\user\\AppData\\Local\\TempE\\junk.txt"
        )));
        assert!(!category.contains_path(std::path::Path::new(
            "C:\\Windows\\System32\\config"
        )));
    }

    #[test]
    fn thumbnail_cache_category_restricts_to_database_files() {
        let categories = builtin_categories();
        let thumbnails = categories
            .iter()
            .find(|category| category.id == CleanupCategoryId::ExplorerThumbnailCache)
            .expect("thumbnail cache category should exist with LOCALAPPDATA set");
        let patterns = thumbnails.file_patterns.as_deref().expect("patterns required");
        assert!(patterns.iter().any(|pattern| pattern.starts_with("thumb")));
        assert!(patterns.iter().any(|pattern| pattern.starts_with("icon")));
    }

    #[test]
    fn traversal_shaped_environment_is_rejected() {
        // env_root must refuse values that produce parent-directory escapes.
        let malicious = PathBuf::from("C:\\Users\\user\\..\\..\\Windows");
        assert!(contains_parent_component(&malicious));
    }
}
