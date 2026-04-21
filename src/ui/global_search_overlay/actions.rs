use crate::app::state::ImageViewerApp;
use rust_i18n::t;

/// What the user wants to do with a search result.
pub(super) enum ResultAction {
    /// Open the file with its default program (or navigate into if directory).
    OpenFile(String, bool),
    /// Navigate to the parent folder and select the item.
    OpenFolder(String, bool),
    /// Open the file with the internal viewer (text, PDF, image, video/audio).
    PreviewFile(String),
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
    app.close_global_search();

    if is_dir {
        app.navigate_to(full_path);
        return;
    }

    let full_path_buf = std::path::PathBuf::from(full_path);
    let Some(parent) = full_path_buf.parent() else {
        app.navigate_to(full_path);
        return;
    };
    let parent_path = parent.to_string_lossy();

    app.pending_select_path = Some(full_path_buf.clone());

    let current_norm = normalize_path_for_compare(&app.navigation_state.current_path);
    let destination_norm = normalize_path_for_compare(parent_path.as_ref());

    if current_norm == destination_norm {
        if app.select_item_by_path(&full_path_buf) {
            app.pending_select_path = None;
        } else {
            app.loaded_path.clear();
            app.load_folder(false);
        }
    } else {
        app.navigate_to(parent_path.as_ref());
    }
}

pub(super) fn open_file_with_default(app: &mut ImageViewerApp, full_path: &str, is_dir: bool) {
    app.close_global_search();

    if is_dir {
        app.navigate_to(full_path);
    } else {
        let path = std::path::PathBuf::from(full_path);
        app.open_with_shell_guarded(&path);
    }
}

pub(super) fn preview_search_result(app: &mut ImageViewerApp, full_path: &str) {
    use crate::ui::components::media_preview::MediaPreview;

    app.close_global_search();

    let path = std::path::PathBuf::from(full_path);
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_owned();

    let is_video = crate::infrastructure::windows::is_video_extension(&ext);
    let is_audio = crate::infrastructure::windows::is_audio_extension(&ext);
    let is_pdf = ext.eq_ignore_ascii_case("pdf");
    let is_image = crate::infrastructure::windows::is_image_extension(&ext);
    let is_text = crate::text_viewer::is_text_extension(&ext);

    if is_video || is_audio {
        // Open in standalone window (same as secondary toolbar play button)
        app.kill_video_player_process();
        if matches!(app.media_preview.as_ref(), Some(MediaPreview::Video(_))) {
            app.destroy_media_preview();
        }
        if let Some(child) = crate::video_player::open_video_player(path, 0.0, app.session_volume) {
            app.video_player_process = Some(child);
        }
    } else if is_pdf {
        crate::pdf_viewer::open_pdf_viewer(path);
    } else if is_image {
        crate::image_viewer::open_image_viewer(path);
    } else if is_text {
        crate::text_viewer::open_text_viewer(path);
    } else {
        // Fallback to default program if no internal viewer supports this type
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
