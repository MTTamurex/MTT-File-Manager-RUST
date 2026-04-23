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
                    // Sentinel keys from background revalidation of special icons.
                    if result.key == "__computer__" {
                        let texture = ctx.load_texture(
                            "computer_icon",
                            egui::ColorImage::from_rgba_unmultiplied(
                                [width as usize, height as usize],
                                &rgba_data,
                            ),
                            egui::TextureOptions::LINEAR,
                        );
                        self.computer_icon_texture = Some(texture);
                        uploads += 1;
                        continue;
                    }
                    if result.key == "__recyclebin__" {
                        let texture = ctx.load_texture(
                            "recycle_bin_icon",
                            egui::ColorImage::from_rgba_unmultiplied(
                                [width as usize, height as usize],
                                &rgba_data,
                            ),
                            egui::TextureOptions::LINEAR,
                        );
                        self.recycle_bin_icon_texture = Some(texture);
                        uploads += 1;
                        continue;
                    }

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
                        self.drive_icon_cache.insert(result.key, texture);
                    }
                    uploads += 1;
                }
                None => {
                    self.failed_drive_icons.insert(result.key);
                }
            }
        }
        if received_any {
            ctx.request_repaint();
        }
    }

    /// Gets or loads a drive icon (non-blocking).
    ///
    /// On first call, checks SQLite cache; on miss spawns a background thread
    /// for extraction and returns None.  The emoji fallback is shown until the
    /// icon is ready.  On subsequent frames, `poll_async_icons()` picks up the
    /// result and caches it.
    pub fn get_or_load_drive_icon(
        &mut self,
        _ctx: &egui::Context,
        drive_path: &str,
    ) -> Option<egui::TextureHandle> {
        if self.failed_drive_icons.contains(drive_path) {
            return None;
        }

        if let Some(icon) = self.drive_icon_cache.get(drive_path) {
            return Some(icon.clone());
        }

        // Already loading in background - wait for result
        if self.loading_drive_icons.contains(drive_path) {
            return None;
        }

        // Try SQLite cache first → instant icon without COM call.
        let cache_key_sql = format!("drive:{}", drive_path);
        if let Some(dc) = &self.disk_cache {
            if let Some((pixels, w, h)) = dc.get_shell_icon(&cache_key_sql) {
                let tx = self.icon_result_tx.clone();
                let key = drive_path.to_string();
                self.loading_drive_icons.insert(key.clone());
                let _ = tx.send(AsyncIconResult {
                    key,
                    data: Some((pixels, w, h)),
                });
                return None; // will be picked up by poll_async_icons next frame
            }
        }

        // Spawn background extraction (non-blocking)
        let key = drive_path.to_string();
        self.loading_drive_icons.insert(key.clone());
        let tx = self.icon_result_tx.clone();
        let dc = self.disk_cache.clone();
        std::thread::spawn(move || {
            let data = windows::extract_drive_icon(&key, IconSize::Jumbo)
                .map_err(|e| log::trace!("[Icon] Drive icon extraction failed for {}: {}", key, e))
                .ok();
            // Persist to SQLite for next launch
            if let (Some(dc), Some((ref pixels, w, h))) = (&dc, &data) {
                dc.put_shell_icon(&format!("drive:{}", key), pixels, *w, *h);
            }
            let _ = tx.send(AsyncIconResult { key, data });
        });

        None
    }

    /// Gets or loads a native icon for a specific folder path (like OneDrive).
    ///
    /// Non-blocking: checks SQLite cache first, then spawns background extraction
    /// on miss.  Returns None until ready.  The fallback emoji/text is shown in
    /// the sidebar meanwhile.
    pub fn get_or_load_folder_path_icon(
        &mut self,
        _ctx: &egui::Context,
        folder_path: &str,
    ) -> Option<egui::TextureHandle> {
        let cache_key = folder_path.to_string();

        if self.failed_drive_icons.contains(&cache_key) {
            return None;
        }

        if let Some(icon) = self.drive_icon_cache.get(&cache_key) {
            return Some(icon.clone());
        }

        // Already loading in background - wait for result
        if self.loading_drive_icons.contains(&cache_key) {
            return None;
        }

        // Try SQLite cache first → instant icon without COM call.
        let cache_key_sql = format!("special:{}", folder_path);
        if let Some(dc) = &self.disk_cache {
            if let Some((pixels, w, h)) = dc.get_shell_icon(&cache_key_sql) {
                self.loading_drive_icons.insert(cache_key.clone());
                let tx = self.icon_result_tx.clone();
                let _ = tx.send(AsyncIconResult {
                    key: cache_key,
                    data: Some((pixels, w, h)),
                });
                return None;
            }
        }

        // Spawn background extraction (non-blocking)
        self.loading_drive_icons.insert(cache_key.clone());
        let tx = self.icon_result_tx.clone();
        let path_owned = folder_path.to_string();
        let dc = self.disk_cache.clone();
        std::thread::spawn(move || {
            let data = windows::extract_drive_icon(&path_owned, IconSize::Jumbo)
                .map_err(|e| log::trace!("[Icon] Folder icon extraction failed for {}: {}", path_owned, e))
                .ok();
            // Persist to SQLite for next launch
            if let (Some(dc), Some((ref pixels, w, h))) = (&dc, &data) {
                dc.put_shell_icon(&format!("special:{}", path_owned), pixels, *w, *h);
            }
            let _ = tx.send(AsyncIconResult {
                key: cache_key,
                data,
            });
        });

        None
    }

    /// Pre-load icons for known special folders.
    ///
    /// **Phase 1 (instant):** Loads cached icons from SQLite and sends them
    /// through the channel so they appear on the very first frame.
    ///
    /// **Phase 2 (background):** Spawns a single thread that re-extracts all
    /// icons via COM/Shell API.  If any icon changed (e.g. theme switch) the
    /// updated pixels are persisted to SQLite and sent through the channel to
    /// replace the stale texture.
    pub fn preload_special_folder_icons(&mut self) {
        let paths = crate::infrastructure::onedrive::special_folder_paths();
        if paths.is_empty() {
            return;
        }

        // Mark all as loading to prevent duplicate requests from the render loop.
        for p in &paths {
            self.loading_drive_icons.insert(p.clone());
        }

        // Phase 1: bulk-load from SQLite (synchronous, <5 ms).
        let cached = self
            .disk_cache
            .as_ref()
            .map(|dc| dc.get_all_shell_icons())
            .unwrap_or_default();

        let mut cache_hits = 0usize;
        let mut miss_paths: Vec<String> = Vec::new();

        for path in &paths {
            let sql_key = format!("special:{}", path);
            if let Some((pixels, w, h)) = cached.get(&sql_key) {
                // Send cached icon through channel → available on first poll_async_icons.
                let _ = self.icon_result_tx.send(AsyncIconResult {
                    key: path.clone(),
                    data: Some((pixels.clone(), *w, *h)),
                });
                cache_hits += 1;
            } else {
                miss_paths.push(path.clone());
            }
        }

        if cache_hits > 0 {
            log::info!(
                "[ShellIconCache] Loaded {} special folder icons from SQLite ({} cache misses)",
                cache_hits,
                miss_paths.len(),
            );
        }

        // Phase 2: background thread extracts ALL icons via COM.
        // - Misses are sent immediately through the channel.
        // - Hits are compared with the cached version; if pixels differ the
        //   updated icon is sent as a replacement and persisted.
        let tx = self.icon_result_tx.clone();
        let dc = self.disk_cache.clone();
        std::thread::spawn(move || {
            #[cfg(target_os = "windows")]
            let _com_guard = super::ComStaGuard::new();

            let mut revalidated = 0usize;
            let mut changed = 0usize;

            for path in &paths {
                let fresh = windows::extract_drive_icon(path, IconSize::Jumbo)
                    .map_err(|e| log::trace!("[Icon] Revalidation extraction failed for {}: {}", path, e))
                    .ok();

                let sql_key = format!("special:{}", path);
                let was_cached = cached.contains_key(&sql_key);

                match (&fresh, was_cached) {
                    // Cache miss → send extracted icon + persist.
                    (Some((pixels, w, h)), false) => {
                        if let Some(ref dc) = dc {
                            dc.put_shell_icon(&sql_key, pixels, *w, *h);
                        }
                        let _ = tx.send(AsyncIconResult {
                            key: path.clone(),
                            data: Some((pixels.clone(), *w, *h)),
                        });
                    }
                    // Cache hit → compare. Only send + persist when changed.
                    (Some((pixels, w, h)), true) => {
                        revalidated += 1;
                        let old = &cached[&sql_key];
                        if old.0 != *pixels || old.1 != *w || old.2 != *h {
                            changed += 1;
                            if let Some(ref dc) = dc {
                                dc.put_shell_icon(&sql_key, pixels, *w, *h);
                            }
                            let _ = tx.send(AsyncIconResult {
                                key: path.clone(),
                                data: Some((pixels.clone(), *w, *h)),
                            });
                        }
                    }
                    // Extraction failed and wasn't cached → mark as failed.
                    (None, false) => {
                        let _ = tx.send(AsyncIconResult {
                            key: path.clone(),
                            data: None,
                        });
                    }
                    // Extraction failed but was cached → keep cached version.
                    (None, true) => {}
                }
            }

            if revalidated > 0 {
                log::info!(
                    "[ShellIconCache] Revalidated {} special folder icons, {} changed",
                    revalidated,
                    changed,
                );
            }
            // ComStaGuard drops here, balancing CoUninitialize automatically.
        });
    }
}
