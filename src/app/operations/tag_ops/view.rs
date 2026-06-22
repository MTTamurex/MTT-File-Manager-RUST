use super::*;
use crate::app::state::{FolderLoadError, ImageViewerApp};
use crate::domain::file_entry::FileEntry;
use crate::domain::file_tag::FileTag;
use crate::domain::special_paths::{tag_id_from_view_path, tag_view_path};
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Instant;
use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{
    GetFileAttributesExW, GetFileExInfoStandard, INVALID_FILE_ATTRIBUTES,
    WIN32_FILE_ATTRIBUTE_DATA,
};

/// Convert a Windows FILETIME (100-nanosecond intervals since 1601-01-01) to
/// Unix seconds. Returns 0 for invalid/zero timestamps.
fn filetime_to_unix_secs(ft: &windows::Win32::Foundation::FILETIME) -> u64 {
    let ticks = ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64);
    if ticks > 116444736000000000 {
        (ticks - 116444736000000000) / 10_000_000
    } else {
        0
    }
}

fn tag_view_file_entry(path: PathBuf, show_hidden: bool) -> Option<FileEntry> {
    // Use GetFileAttributesExW instead of std::fs::metadata().
    // std::fs::metadata() calls CreateFileW + GetFileInformationByHandle + CloseHandle,
    // which opens a kernel handle per file (security check, share mode check, object
    // allocation/deallocation). On a cold NTFS cache this is 5-15ms per file on HDD.
    //
    // GetFileAttributesExW reads from the directory entry cache in a single kernel
    // call — no handle, no OneDrive file recall, returns in microseconds.
    // It returns WIN32_FILE_ATTRIBUTE_DATA with all fields we need: attributes,
    // file size, and creation/last-write timestamps.
    let path_wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut data = WIN32_FILE_ATTRIBUTE_DATA::default();
    let result = unsafe {
        GetFileAttributesExW(
            PCWSTR(path_wide.as_ptr()),
            GetFileExInfoStandard,
            &mut data as *mut _ as *mut std::ffi::c_void,
        )
    };
    if result.is_err() || data.dwFileAttributes == INVALID_FILE_ATTRIBUTES {
        return None;
    }

    let name = path.file_name()?.to_string_lossy().to_string();
    let is_archive = crate::domain::file_entry::is_archive_extension(&name);
    let attrs = data.dwFileAttributes;
    let is_real_dir = (attrs & 0x10) != 0; // FILE_ATTRIBUTE_DIRECTORY
    let is_dir = is_real_dir || is_archive;
    let is_hidden = (attrs & 0x2) != 0;
    if is_hidden && !show_hidden {
        return None;
    }

    let size = if is_real_dir && !is_archive {
        0
    } else {
        ((data.nFileSizeHigh as u64) << 32) | (data.nFileSizeLow as u64)
    };
    let modified = filetime_to_unix_secs(&data.ftLastWriteTime);
    let created = {
        let c = filetime_to_unix_secs(&data.ftCreationTime);
        if c > 0 { Some(c) } else { None }
    };
    let is_cloud = crate::infrastructure::onedrive::is_cloud_sync_path(&path);
    let sync_status = if is_cloud {
        crate::infrastructure::onedrive::get_sync_status(attrs, true)
    } else {
        crate::domain::file_entry::SyncStatus::None
    };

    Some(FileEntry {
        path,
        name,
        is_dir,
        size,
        modified,
        created,
        folder_cover: None,
        drive_info: None,
        sync_status,
        is_hidden,
        recycle_bin: None,
    })
}

impl ImageViewerApp {
    pub fn sorted_tag_definitions(&self) -> Vec<FileTag> {
        let mut tags: Vec<FileTag> = self.tag_definitions.values().cloned().collect();
        tags.sort_by_key(tag_sort_key);
        tags
    }

    pub fn tag_view_display_name(&self, tag_id: i64) -> String {
        let name = self
            .tag_definitions
            .get(&tag_id)
            .map(|tag| tag.name.clone())
            .unwrap_or_else(|| tag_id.to_string());
        rust_i18n::t!("tags.filter_active", name = name).to_string()
    }

    pub fn tag_view_display_name_for_path(&self, path: &str) -> Option<String> {
        tag_id_from_view_path(path).map(|tag_id| self.tag_view_display_name(tag_id))
    }

    pub fn setup_tag_view(&mut self, tag_id: i64) {
        if !self.tag_definitions.contains_key(&tag_id) {
            self.active_tag_filter = None;
            self.navigate_to_computer();
            return;
        }

        let view_path = tag_view_path(tag_id);
        self.bump_folder_load_generation();
        self.invalidate_active_items_rebuild();

        self.navigation_state.current_path = view_path.clone();
        self.navigation_state.path_input = self.tag_view_display_name(tag_id);
        self.navigation_state.is_computer_view = false;
        self.navigation_state.is_recycle_bin_view = false;
        self.active_tag_filter = Some(tag_id);

        self.sort_mode = self.sort_mode_normal;
        self.sort_descending = self.sort_descending_normal;
        self.folders_position = self.folders_position_normal;
        self.current_folder_locked = false;

        self.items = Arc::new(Vec::new());
        self.all_items_mut().clear();
        self.total_items = 0;
        self.is_loading_folder = true;
        self.folder_load_error = None;
        self.pending_all_items_clear = false;
        self.hold_visible_items_until_load_complete = false;
        self.pending_items_rebuild = false;
        self.pending_items_count = 0;
        self.loading_started_at = Instant::now();
        self.loaded_path = view_path;
        self.reset_selection_and_search();

        // The in-memory tag_assignments map is loaded at startup (init.rs)
        // and maintained by all tag mutation paths via sync_tag_assignments_normalized().
        // No need to reload from DB here — the filtered path list is queried
        // directly from SQLite in the background thread (see below).

        let my_gen = self.generation;
        let sender = self.file_entry_sender.clone();
        let ui_ctx = self.ui_ctx.clone();
        let show_hidden = self.show_hidden_files;
        let failure_path = PathBuf::from(&self.navigation_state.current_path);
        let app_state_db = self.app_state_db.clone();

        // Generation-aware cancellation: for the active panel, use the shared
        // current_generation atomic so the thread can detect navigation away.
        // For the inactive panel, use a local atomic that never changes
        // (bump_folder_load_generation skips current_generation for inactive
        // panel context, so using the shared atomic would cause immediate break).
        let gen_tracker: Arc<AtomicUsize> = if self.in_inactive_panel_context {
            Arc::new(AtomicUsize::new(self.generation))
        } else {
            self.current_generation.clone()
        };

        let spawn_result = std::thread::Builder::new()
            .name("tag-view-load".into())
            .spawn(move || {
                const BATCH_SIZE: usize = 100;

                // Query only paths for this specific tag using the
                // idx_file_tag_assignments_tag index — avoids full table scan.
                let mut paths = app_state_db.get_tag_assignment_paths(tag_id);
                paths.sort_by_key(|path| path.to_string_lossy().to_lowercase());

                let mut batch = Vec::with_capacity(BATCH_SIZE);

                for path in paths {
                    if gen_tracker.load(std::sync::atomic::Ordering::Relaxed) != my_gen {
                        break; // User navigated away — abort early
                    }
                    if let Some(entry) = tag_view_file_entry(path, show_hidden) {
                        batch.push(entry);
                        if batch.len() >= BATCH_SIZE {
                            let _ = sender.send((my_gen, std::mem::take(&mut batch)));
                            ui_ctx.request_repaint();
                            batch = Vec::with_capacity(BATCH_SIZE);
                        }
                    }
                }

                // Always send end-of-load sentinel, even after generation-break.
                // The consumer discards batches from old generations, so this is safe.
                if !batch.is_empty() {
                    let _ = sender.send((my_gen, batch));
                }
                let _ = sender.send((my_gen, Vec::new()));
                ui_ctx.request_repaint();
            });

        if let Err(error) = spawn_result {
            let message = format!("Failed to spawn tag view loader: {}", error);
            log::error!("[TAGS] {}", message);
            self.is_loading_folder = false;
            self.folder_load_error = Some(FolderLoadError::other(
                failure_path.clone(),
                message.clone(),
            ));
            let _ = self
                .folder_load_failure_sender
                .send((my_gen, FolderLoadError::other(failure_path, message)));
            self.ui_ctx.request_repaint();
        }
    }

    pub fn reload_visible_tag_views(&mut self) -> bool {
        let active_tag_id = tag_id_from_view_path(&self.navigation_state.current_path);
        let inactive_tag_id = self
            .dual_panel_inactive_state
            .as_ref()
            .and_then(|snapshot| tag_id_from_view_path(&snapshot.path));

        if active_tag_id.is_none() && inactive_tag_id.is_none() {
            return false;
        }

        if let Some(tag_id) = active_tag_id {
            self.setup_tag_view(tag_id);
        }

        if let Some(tag_id) = inactive_tag_id {
            self.with_inactive_panel(|app| {
                app.setup_tag_view(tag_id);
            });
        }

        true
    }
}
