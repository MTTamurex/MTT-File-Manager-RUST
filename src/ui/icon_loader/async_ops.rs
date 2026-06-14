use super::*;

fn icon_resource_path(resource: &str) -> Option<std::path::PathBuf> {
    let trimmed = resource.trim().trim_matches('"');
    let path_part = trimmed
        .rfind(',')
        .map(|idx| &trimmed[..idx])
        .unwrap_or(trimmed)
        .trim()
        .trim_matches('"');
    (!path_part.is_empty()).then(|| std::path::PathBuf::from(path_part))
}

fn folder_path_icon_cache_key(folder_path: &str) -> String {
    folder_path
        .to_lowercase()
        .trim_end_matches(['\\', '/'])
        .to_string()
}

fn cloud_root_icon_cache_key(root_path: &str) -> String {
    format!("cloud:{}", folder_path_icon_cache_key(root_path))
}

impl IconLoader {
    pub fn set_cloud_root_icon_resources(
        &mut self,
        roots: &[crate::domain::cloud_root::CloudRoot],
    ) {
        self.registered_folder_icon_resources.clear();

        for root in roots {
            let Some(resource) = root.icon_resource.as_deref() else {
                continue;
            };
            if resource.trim().is_empty() {
                continue;
            }

            self.registered_folder_icon_resources
                .insert(folder_path_icon_cache_key(&root.path), resource.to_string());
        }
    }

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

        // Spawn background extraction (non-blocking, bounded)
        let key = drive_path.to_string();
        let tx = self.icon_result_tx.clone();
        let active = self.auxiliary_icon_threads.clone();

        if !super::try_reserve_auxiliary_icon_thread(&active) {
            return None;
        }

        self.loading_drive_icons.insert(key.clone());
        let thread_key = key.clone();
        let extraction_key = key.clone();
        let thread_active = active.clone();
        let spawn_result = std::thread::Builder::new()
            .name("drive-icon-worker".to_string())
            .spawn(move || {
                let _guard = super::ThreadCountGuard(thread_active);
                let data = windows::extract_drive_icon(&extraction_key, IconSize::Jumbo)
                    .map_err(|e| {
                        log::trace!(
                            "[Icon] Drive icon extraction failed for {}: {}",
                            extraction_key,
                            e
                        )
                    })
                    .ok();
                let _ = tx.send(AsyncIconResult {
                    key: thread_key,
                    data,
                });
            });

        if let Err(error) = spawn_result {
            active.fetch_sub(1, Ordering::Relaxed);
            self.loading_drive_icons.remove(&key);
            log::error!("[Icon] Failed to spawn drive-icon-worker: {}", error);
        }

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
        let cache_key = folder_path_icon_cache_key(folder_path);

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

        // Spawn background extraction (non-blocking, bounded)
        let tx = self.icon_result_tx.clone();
        let path_owned = folder_path.to_string();
        let active = self.auxiliary_icon_threads.clone();

        if !super::try_reserve_auxiliary_icon_thread(&active) {
            return None;
        }

        self.loading_drive_icons.insert(cache_key.clone());
        let thread_cache_key = cache_key.clone();
        let thread_active = active.clone();
        let spawn_result = std::thread::Builder::new()
            .name("folder-path-icon-worker".to_string())
            .spawn(move || {
                let _guard = super::ThreadCountGuard(thread_active);
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
                    key: thread_cache_key,
                    data,
                });
            });

        if let Err(error) = spawn_result {
            active.fetch_sub(1, Ordering::Relaxed);
            self.loading_drive_icons.remove(&cache_key);
            log::error!("[Icon] Failed to spawn folder-path-icon-worker: {}", error);
        }

        None
    }

    /// Gets a provider-registered folder icon for an operational folder path.
    ///
    /// Used for providers such as Google Drive where the usable folder is the
    /// shortcut target, but the correct icon lives on the `.lnk` in the virtual
    /// drive root.
    pub fn get_or_load_registered_folder_icon(
        &mut self,
        ctx: &egui::Context,
        folder_path: &str,
    ) -> Option<egui::TextureHandle> {
        let cache_key = folder_path_icon_cache_key(folder_path);
        let resource = self
            .registered_folder_icon_resources
            .get(&cache_key)?
            .clone();

        self.get_or_load_cloud_root_icon(ctx, folder_path, Some(&resource))
    }

    pub fn has_registered_folder_icon(&self, folder_path: &str) -> bool {
        let cache_key = folder_path_icon_cache_key(folder_path);
        self.registered_folder_icon_resources
            .contains_key(&cache_key)
    }

    /// Gets or loads a Cloud Files sync-root icon (non-blocking).
    ///
    /// Sync roots such as Proton Drive can expose a normal filesystem path plus
    /// a provider icon resource registered in Explorer. Prefer the registered
    /// provider icon so this matches Explorer's sidebar instead of the generic
    /// folder icon for the backing filesystem path.
    pub fn get_or_load_cloud_root_icon(
        &mut self,
        _ctx: &egui::Context,
        root_path: &str,
        icon_resource: Option<&str>,
    ) -> Option<egui::TextureHandle> {
        let cache_key = cloud_root_icon_cache_key(root_path);

        if self.failed_drive_icons.peek(&cache_key).is_some() {
            return None;
        }

        if let Some(icon) = self.drive_icon_cache.get(&cache_key) {
            return Some(icon.clone());
        }

        if self.loading_drive_icons.contains(&cache_key) {
            return None;
        }

        let tx = self.icon_result_tx.clone();
        let path_owned = root_path.to_string();
        let resource_path = icon_resource.and_then(icon_resource_path);
        let active = self.auxiliary_icon_threads.clone();

        if !super::try_reserve_auxiliary_icon_thread(&active) {
            return None;
        }

        self.loading_drive_icons.insert(cache_key.clone());
        let thread_cache_key = cache_key.clone();
        let thread_active = active.clone();
        let spawn_result = std::thread::Builder::new()
            .name("cloud-root-icon-worker".to_string())
            .spawn(move || {
                let _guard = super::ThreadCountGuard(thread_active);
                let _com = super::ComStaGuard::new();
                let data = (|| {
                    if let Some(path) = resource_path.as_ref() {
                        if let Ok(icon) = windows::extract_file_icon_by_path(path, IconSize::Jumbo)
                        {
                            return Ok(icon);
                        }
                    }

                    windows::extract_drive_icon(&path_owned, IconSize::Jumbo).or_else(|_| {
                        resource_path
                            .as_ref()
                            .ok_or_else(|| "missing icon resource".into())
                            .and_then(|path| {
                                windows::extract_file_icon_by_path(path, IconSize::Jumbo)
                            })
                    })
                })()
                .map_err(|e| {
                    log::trace!(
                        "[Icon] Cloud root icon extraction failed for {}: {}",
                        path_owned,
                        e
                    )
                })
                .ok();
                let _ = tx.send(AsyncIconResult {
                    key: thread_cache_key,
                    data,
                });
            });

        if let Err(error) = spawn_result {
            active.fetch_sub(1, Ordering::Relaxed);
            self.loading_drive_icons.remove(&cache_key);
            log::error!("[Icon] Failed to spawn cloud-root-icon-worker: {}", error);
        }

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
        let failed_paths = paths.clone();
        let spawn_result = std::thread::Builder::new()
            .name("special-folder-icon-worker".to_string())
            .spawn(move || {
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

        if let Err(error) = spawn_result {
            for path in failed_paths {
                self.loading_drive_icons.remove(&path);
            }
            log::error!(
                "[Icon] Failed to spawn special-folder-icon-worker: {}",
                error
            );
        }
    }
}
