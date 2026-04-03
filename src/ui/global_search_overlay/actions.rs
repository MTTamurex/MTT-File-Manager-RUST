use crate::app::state::ImageViewerApp;
use rust_i18n::t;

/// What the user wants to do with a search result.
pub(super) enum ResultAction {
    /// Open the file with its default program (or navigate into if directory).
    OpenFile(String, bool),
    /// Navigate to the parent folder and select the item.
    OpenFolder(String, bool),
}

#[inline]
pub(super) fn format_result_meta(file_type: &str) -> String {
    file_type.to_string()
}

#[inline]
pub(super) fn filtered_contains(filtered_indices: &[usize], source_idx: usize) -> bool {
    filtered_indices.binary_search(&source_idx).is_ok()
}

#[inline]
pub(super) fn filtered_position(filtered_indices: &[usize], source_idx: usize) -> Option<usize> {
    filtered_indices.binary_search(&source_idx).ok()
}

pub(super) fn normalize_path_for_compare(path: &str) -> String {
    let lower = path.to_lowercase();
    let stripped = lower.strip_prefix(r"\\?\").unwrap_or(&lower);

    if stripped.len() > 3 {
        stripped.trim_end_matches('\\').to_string()
    } else {
        stripped.to_string()
    }
}

pub(super) fn activate_search_result(app: &mut ImageViewerApp, full_path: &str, is_dir: bool) {
    app.global_search.active = false;
    app.global_search.focus_request = false;
    app.global_search.size_cache.clear();
    app.global_search.tooltip_texture_cache.clear();
    app.global_search.metadata_cache.clear();

    if is_dir {
        app.navigate_to(full_path);
        return;
    }

    let full_path_buf = std::path::PathBuf::from(full_path);
    let Some(parent) = full_path_buf.parent() else {
        app.navigate_to(full_path);
        return;
    };
    let parent_path = parent.to_string_lossy().to_string();

    app.pending_select_path = Some(full_path_buf.clone());

    let current_norm = normalize_path_for_compare(&app.navigation_state.current_path);
    let destination_norm = normalize_path_for_compare(&parent_path);

    if current_norm == destination_norm {
        if app.select_item_by_path(&full_path_buf) {
            app.pending_select_path = None;
        } else {
            app.loaded_path.clear();
            app.load_folder(false);
        }
    } else {
        app.navigate_to(&parent_path);
    }
}

pub(super) fn open_file_with_default(app: &mut ImageViewerApp, full_path: &str, is_dir: bool) {
    app.global_search.active = false;
    app.global_search.focus_request = false;
    app.global_search.size_cache.clear();
    app.global_search.tooltip_texture_cache.clear();
    app.global_search.metadata_cache.clear();

    if is_dir {
        app.navigate_to(full_path);
    } else {
        let path = std::path::PathBuf::from(full_path);
        app.open_with_shell_guarded(&path);
    }
}

pub(super) fn file_type_label(full_path: &str, is_dir: bool) -> String {
    if is_dir {
        return t!("search_results.folder").to_string();
    }

    let path = std::path::Path::new(full_path);
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        if !ext.is_empty() {
            return t!("search_results.file_ext", ext = ext.to_uppercase()).to_string();
        }
    }

    t!("search_results.file_generic").to_string()
}

pub(super) fn resolve_result_size(
    app: &mut ImageViewerApp,
    full_path: &str,
    is_dir: bool,
    size: u64,
) -> Option<u64> {
    if is_dir {
        return None;
    }

    if size > 0 {
        return Some(size);
    }

    app.global_search
        .size_cache
        .get(full_path)
        .copied()
        .flatten()
}
