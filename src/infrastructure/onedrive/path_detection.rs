use std::path::Path;

pub(super) fn init_onedrive_paths() {
    super::ONEDRIVE_ROOTS.get_or_init(|| {
        let mut roots = Vec::new();
        for var in ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"] {
            if let Ok(path) = std::env::var(var) {
                if !path.is_empty() {
                    roots.push(path.to_lowercase());
                }
            }
        }
        log::info!("[OneDrive] Detected roots: {:?}", roots);
        roots
    });
}

pub(super) fn is_onedrive_path(path: &Path) -> bool {
    let path_lower = path.to_string_lossy().to_lowercase();
    super::ONEDRIVE_ROOTS
        .get()
        .map(|roots| roots.iter().any(|r| path_lower.starts_with(r)))
        .unwrap_or(false)
}
