//! Cloud-root icon extraction and refresh when provider resources change in place.

use super::async_ops::cloud_root_icon_cache_key;
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

fn folder_desktop_ini_icon_resource(folder: &std::path::Path) -> Option<String> {
    let path = folder.join("desktop.ini");
    let bytes = std::fs::read(path).ok()?;

    let text = if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let words: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        String::from_utf16_lossy(&words)
    } else if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let words: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
            .collect();
        String::from_utf16_lossy(&words)
    } else if bytes.iter().skip(1).step_by(2).take(16).any(|b| *b == 0) {
        let words: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        String::from_utf16_lossy(&words)
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };

    text.lines().find_map(|line| {
        let trimmed = line.trim();
        let (key, value) = trimmed.split_once('=')?;
        key.trim()
            .eq_ignore_ascii_case("IconResource")
            .then(|| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

impl IconLoader {
    /// Google Drive can replace a shortcut/icon without changing its path.
    /// Re-extract after each drive scan, preserving the displayed texture.
    pub fn refresh_google_drive_root_icons(
        &mut self,
        roots: &[crate::domain::cloud_root::CloudRoot],
    ) {
        for root in roots.iter().filter(|root| root.is_google_drive_shortcut()) {
            let key = cloud_root_icon_cache_key(&root.path, root.icon_resource.as_deref());
            self.failed_drive_icons.pop(&key);
            if self.drive_icon_cache.peek(&key).is_some() {
                self.stale_drive_icons.insert(key);
            }
        }
    }

    /// Gets or loads a Cloud Files sync-root icon (non-blocking).
    ///
    /// Prefer the provider's registered icon over the backing folder's icon.
    pub fn get_or_load_cloud_root_icon(
        &mut self,
        _ctx: &egui::Context,
        root_path: &str,
        icon_resource: Option<&str>,
    ) -> Option<egui::TextureHandle> {
        let cache_key = cloud_root_icon_cache_key(root_path, icon_resource);
        let cached_icon = self.drive_icon_cache.get(&cache_key).cloned();
        if cached_icon.is_some() && !self.stale_drive_icons.contains(&cache_key) {
            return cached_icon;
        }
        if self.failed_drive_icons.peek(&cache_key).is_some()
            || self.loading_drive_icons.contains(&cache_key)
        {
            return cached_icon;
        }

        let tx = self.icon_result_tx.clone();
        let path_owned = root_path.to_string();
        let resource = icon_resource
            .map(str::trim)
            .filter(|resource| !resource.is_empty())
            .map(str::to_string);
        let resource_path = resource.as_deref().and_then(icon_resource_path);
        let active = self.auxiliary_icon_threads.clone();

        if !super::try_reserve_auxiliary_icon_thread(&active) {
            return cached_icon;
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
                    if let Some(resource) = resource.as_ref() {
                        if let Ok(icon) = windows::extract_icon_resource(resource, IconSize::Jumbo)
                        {
                            return Ok(icon);
                        }
                    }

                    if let Some(path) = resource_path.as_ref() {
                        if path.is_dir() {
                            if let Some(resource) = folder_desktop_ini_icon_resource(path) {
                                if let Ok(icon) =
                                    windows::extract_icon_resource(&resource, IconSize::Jumbo)
                                {
                                    return Ok(icon);
                                }
                            }

                            if let Ok(icon) = windows::extract_drive_icon(
                                &path.to_string_lossy(),
                                IconSize::Jumbo,
                            ) {
                                return Ok(icon);
                            }
                        }

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

        cached_icon
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::cloud_root::CloudRoot;

    #[test]
    fn unchanged_google_drive_resource_refreshes_without_dropping_texture() {
        let ctx = egui::Context::default();
        let mut loader = IconLoader::new();
        let root = CloudRoot::google_drive_shortcut(
            r"D:\Mirror".into(),
            "Google Drive".into(),
            Some(r"G:\folder.lnk".into()),
            r"G:\".into(),
        );
        let key = cloud_root_icon_cache_key(&root.path, root.icon_resource.as_deref());
        loader.set_cloud_root_icon_resources(std::slice::from_ref(&root));
        // Seed after initial registration, which intentionally invalidates changed keys.
        let texture = ctx.load_texture(
            "cached-cloud-icon",
            egui::ColorImage::new([1, 1], vec![egui::Color32::WHITE]),
            egui::TextureOptions::LINEAR,
        );
        let old_id = texture.id();
        loader.drive_icon_cache.put(key.clone(), texture);
        loader.failed_drive_icons.put(key.clone(), ());
        loader.set_cloud_root_icon_resources(std::slice::from_ref(&root));
        loader.refresh_google_drive_root_icons(std::slice::from_ref(&root));
        assert!(loader.stale_drive_icons.contains(&key));
        assert!(loader.failed_drive_icons.peek(&key).is_none());
        // Simulate an extraction already in flight: keep the old texture visible.
        loader.loading_drive_icons.insert(key.clone());
        assert_eq!(
            loader
                .get_or_load_cloud_root_icon(&ctx, &root.path, root.icon_resource.as_deref())
                .unwrap()
                .id(),
            old_id
        );
        loader
            .icon_result_tx
            .send(AsyncIconResult {
                key: key.clone(),
                data: Some((vec![0, 255, 0, 255], 1, 1)),
            })
            .unwrap();
        loader.poll_async_icons(&ctx);
        assert!(!loader.stale_drive_icons.contains(&key));
        assert!(!loader.loading_drive_icons.contains(&key));
        assert_ne!(loader.drive_icon_cache.peek(&key).unwrap().id(), old_id);
    }

    #[test]
    fn google_drive_refresh_leaves_other_cloud_providers_cached() {
        let ctx = egui::Context::default();
        let mut loader = IconLoader::new();
        let root =
            CloudRoot::windows_cloud_files(r"D:\Cloud".into(), "Cloud Provider".into(), None);
        let key = cloud_root_icon_cache_key(&root.path, None);
        let texture = ctx.load_texture(
            "provider-icon",
            egui::ColorImage::new([1, 1], vec![egui::Color32::WHITE]),
            egui::TextureOptions::LINEAR,
        );
        loader.drive_icon_cache.put(key.clone(), texture);
        loader.refresh_google_drive_root_icons(&[root]);
        assert!(loader.drive_icon_cache.peek(&key).is_some());
        assert!(!loader.stale_drive_icons.contains(&key));
    }
}
