use super::*;

impl IconLoader {
    /// Poll for completed background icon extractions and upload to GPU.
    /// Call this once per frame (lightweight - just drains the channel).
    pub fn poll_async_icons(&mut self, ctx: &egui::Context) {
        // PERF FIX (A-2): Cap GPU uploads per frame to prevent stutter when
        // many drive/folder icon results arrive simultaneously.
        const MAX_ASYNC_ICON_UPLOADS: usize = 8;
        let mut uploads = 0usize;
        let mut received_any = false;
        while uploads < MAX_ASYNC_ICON_UPLOADS {
            let Ok(result) = self.icon_result_rx.try_recv() else {
                break;
            };
            received_any = true;
            self.loading_drive_icons.remove(&result.key);
            match result.data {
                Some((rgba_data, width, height)) => {
                    let texture = ctx.load_texture(
                        format!("async_icon_{}", &result.key),
                        egui::ColorImage::from_rgba_unmultiplied(
                            [width as usize, height as usize],
                            &rgba_data,
                        ),
                        egui::TextureOptions::LINEAR,
                    );
                    // Jumbo file icon results go into icon_cache (LRU) so
                    // get_or_load_icon_sized finds them on subsequent frames.
                    if result.key.ends_with("_Jumbo") {
                        self.icon_cache.put(result.key, texture);
                    } else {
                        self.drive_icon_cache.put(result.key, texture);
                    }
                    uploads += 1;
                }
                None => {
                    self.failed_drive_icons.put(result.key, ());
                }
            }
        }
        if received_any {
            ctx.request_repaint();
        }
    }

    /// Gets or loads a drive icon (non-blocking).
    ///
    /// Drive icons are intentionally session-only so they follow the current
    /// Windows Shell state on every app launch.
    pub fn get_or_load_drive_icon(
        &mut self,
        _ctx: &egui::Context,
        drive_path: &str,
    ) -> Option<egui::TextureHandle> {
        if self.failed_drive_icons.peek(drive_path).is_some() {
            return None;
        }

        if let Some(icon) = self.drive_icon_cache.get(drive_path) {
            return Some(icon.clone());
        }

        // Already loading in background - wait for result
        if self.loading_drive_icons.contains(drive_path) {
            return None;
        }

        // Spawn background extraction (non-blocking)
        let key = drive_path.to_string();
        self.loading_drive_icons.insert(key.clone());
        let tx = self.icon_result_tx.clone();
        std::thread::spawn(move || {
            let data = windows::extract_drive_icon(&key, IconSize::Jumbo)
                .map_err(|e| log::trace!("[Icon] Drive icon extraction failed for {}: {}", key, e))
                .ok();
            let _ = tx.send(AsyncIconResult { key, data });
        });

        None
    }

    /// Gets or loads a native icon for a specific folder path (like OneDrive).
    ///
    /// Non-blocking and session-only. The fallback emoji/text is shown in the
    /// sidebar until the Windows Shell extraction finishes.
    pub fn get_or_load_folder_path_icon(
        &mut self,
        _ctx: &egui::Context,
        folder_path: &str,
    ) -> Option<egui::TextureHandle> {
        let cache_key = folder_path.to_string();

        if self.failed_drive_icons.peek(&cache_key).is_some() {
            return None;
        }

        if let Some(icon) = self.drive_icon_cache.get(&cache_key) {
            return Some(icon.clone());
        }

        // Already loading in background - wait for result
        if self.loading_drive_icons.contains(&cache_key) {
            return None;
        }

        // Spawn background extraction (non-blocking)
        self.loading_drive_icons.insert(cache_key.clone());
        let tx = self.icon_result_tx.clone();
        let path_owned = folder_path.to_string();
        std::thread::spawn(move || {
            let data = windows::extract_drive_icon(&path_owned, IconSize::Jumbo)
                .map_err(|e| {
                    log::trace!(
                        "[Icon] Folder icon extraction failed for {}: {}",
                        path_owned,
                        e
                    )
                })
                .ok();
            let _ = tx.send(AsyncIconResult {
                key: cache_key,
                data,
            });
        });

        None
    }

    /// Pre-load icons for known special folders.
    ///
    /// Spawns a single background thread to extract icons via COM/Shell API.
    /// Results are cached in RAM only for the current session.
    pub fn preload_special_folder_icons(&mut self) {
        let paths = crate::infrastructure::onedrive::special_folder_paths();
        if paths.is_empty() {
            return;
        }

        // Mark all as loading to prevent duplicate requests from the render loop.
        for p in &paths {
            self.loading_drive_icons.insert(p.clone());
        }

        // Background thread extracts ALL icons via COM.
        let tx = self.icon_result_tx.clone();
        std::thread::spawn(move || {
            #[cfg(target_os = "windows")]
            let _com_guard = super::ComStaGuard::new();

            for path in &paths {
                let fresh = windows::extract_drive_icon(path, IconSize::Jumbo)
                    .map_err(|e| {
                        log::trace!(
                            "[Icon] Special folder icon extraction failed for {}: {}",
                            path,
                            e
                        )
                    })
                    .ok();

                let _ = tx.send(AsyncIconResult {
                    key: path.clone(),
                    data: fresh,
                });
            }
            // ComStaGuard drops here, balancing CoUninitialize automatically.
        });
    }
}
