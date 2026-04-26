//! Configuration for virtual drive disk type overrides
//!
//! Allows users to manually configure whether virtual drives (Cryptomator, etc.)
//! should be treated as SSD or HDD for I/O optimization purposes.

use rustc_hash::FxHashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use crate::infrastructure::windows::drives::get_all_drives;

/// Disk type override for a virtual drive
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskTypeOverride {
    /// Treat as SSD (no seek penalty optimization)
    SSD,
    /// Treat as HDD (enable seek penalty optimization)
    HDD,
}

/// Configuration for virtual drive disk types
#[derive(Debug, Clone, Default)]
pub struct VirtualDriveConfig {
    /// Map of drive letter to disk type override
    /// Key is uppercase drive letter (e.g., 'X', 'Y', 'Z')
    pub overrides: FxHashMap<char, DiskTypeOverride>,
}

/// Metadata about a detected virtual drive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedVirtualDrive {
    pub letter: char,
    pub label: String,
    pub file_system: String,
}

impl VirtualDriveConfig {
    /// Get the config file path in per-user application data.
    fn config_path() -> PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("MTT-File-Manager")
            .join("virtual_drive_config.json")
    }

    /// Load configuration from file
    pub fn load() -> Self {
        let path = Self::config_path();

        if !path.exists() {
            return Self::default();
        }

        match fs::read_to_string(&path) {
            Ok(contents) => match serde_json::from_str::<serde_json::Value>(&contents) {
                Ok(json) => {
                    let mut config = Self::default();

                    if let Some(overrides) = json.get("overrides").and_then(|v| v.as_object()) {
                        for (key, value) in overrides {
                            if let Some(drive_letter) = key.chars().next() {
                                if let Some(disk_type_str) = value.as_str() {
                                    let disk_type = match disk_type_str {
                                        "HDD" => DiskTypeOverride::HDD,
                                        _ => DiskTypeOverride::SSD,
                                    };
                                    config
                                        .overrides
                                        .insert(drive_letter.to_ascii_uppercase(), disk_type);
                                }
                            }
                        }
                    }

                    log::info!(
                        "[Config] Loaded virtual drive configuration from {:?}",
                        path
                    );
                    config
                }
                Err(e) => {
                    log::warn!(
                        "[Config] Failed to parse config file: {} - using defaults",
                        e
                    );
                    Self::default()
                }
            },
            Err(e) => {
                log::warn!(
                    "[Config] Failed to read config file: {} - using defaults",
                    e
                );
                Self::default()
            }
        }
    }

    /// Save configuration to file
    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_path();

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config directory: {}", e))?;
        }

        // Build JSON manually
        let mut overrides_map = serde_json::Map::new();
        for (drive_letter, disk_type) in &self.overrides {
            let type_str = match disk_type {
                DiskTypeOverride::SSD => "SSD",
                DiskTypeOverride::HDD => "HDD",
            };
            overrides_map.insert(
                drive_letter.to_string(),
                serde_json::Value::String(type_str.to_string()),
            );
        }

        let json_obj = serde_json::json!({
            "overrides": overrides_map
        });

        let json = serde_json::to_string_pretty(&json_obj)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;

        fs::write(&path, json).map_err(|e| format!("Failed to write config file: {}", e))?;

        log::info!("[Config] Saved virtual drive configuration to {:?}", path);
        Ok(())
    }

    /// Get override for a drive letter
    pub fn get_override(&self, drive_letter: char) -> Option<DiskTypeOverride> {
        self.overrides
            .get(&drive_letter.to_ascii_uppercase())
            .copied()
    }

    /// Set override for a drive letter
    pub fn set_override(&mut self, drive_letter: char, disk_type: DiskTypeOverride) {
        self.overrides
            .insert(drive_letter.to_ascii_uppercase(), disk_type);
    }

    /// Remove override for a drive letter
    pub fn remove_override(&mut self, drive_letter: char) {
        self.overrides.remove(&drive_letter.to_ascii_uppercase());
    }

    /// Clear all overrides
    pub fn clear(&mut self) {
        self.overrides.clear();
    }

    fn from_detected_virtual_drives(drives: &[DetectedVirtualDrive]) -> Self {
        let mut config = Self::default();

        for drive in drives {
            // Safe default for virtual drives until the user overrides it.
            config.set_override(drive.letter, DiskTypeOverride::SSD);
        }

        config
    }
}

fn has_virtual_markers(value: &str) -> bool {
    let lower = value.to_lowercase();
    lower.contains("cryptomator")
        || lower.contains("cryptofs")
        || lower.contains("dokan")
        || lower.contains("winfsp")
        || lower == "fuse"
        || lower.contains("cryptomator-vault")
}

fn mapped_provider_name(drive_letter: char) -> Option<String> {
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::{ERROR_MORE_DATA, NO_ERROR};
    use windows::Win32::NetworkManagement::WNet::WNetGetConnectionW;

    let local = format!("{}:", drive_letter);
    let local_wide: Vec<u16> = local.encode_utf16().chain(std::iter::once(0)).collect();
    let mut required_len: u32 = 0;

    unsafe {
        let probe = WNetGetConnectionW(
            PCWSTR(local_wide.as_ptr()),
            None,
            &mut required_len as *mut u32,
        );

        if probe != NO_ERROR && probe != ERROR_MORE_DATA {
            return None;
        }

        if required_len == 0 {
            return None;
        }

        let mut buffer: Vec<u16> = vec![0; required_len as usize + 1];
        let status = WNetGetConnectionW(
            PCWSTR(local_wide.as_ptr()),
            Some(PWSTR(buffer.as_mut_ptr())),
            &mut required_len as *mut u32,
        );

        if status != NO_ERROR {
            return None;
        }

        let end = buffer
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(required_len as usize)
            .min(buffer.len());

        if end == 0 {
            return None;
        }

        Some(String::from_utf16_lossy(&buffer[..end]))
    }
}

pub fn detect_virtual_drive(drive_letter: char) -> Option<DetectedVirtualDrive> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetVolumeInformationW;

    let normalized_letter = drive_letter.to_ascii_uppercase();
    let root_path = format!("{}:\\", normalized_letter);
    let wide_path: Vec<u16> = root_path.encode_utf16().chain(std::iter::once(0)).collect();

    let mut volume_name = [0u16; 261];
    let mut file_system_name = [0u16; 261];
    let mut serial_number: u32 = 0;
    let mut max_component_len: u32 = 0;
    let mut fs_flags: u32 = 0;

    let ok = unsafe {
        GetVolumeInformationW(
            PCWSTR(wide_path.as_ptr()),
            Some(&mut volume_name),
            Some(&mut serial_number),
            Some(&mut max_component_len),
            Some(&mut fs_flags),
            Some(&mut file_system_name),
        )
    };

    let provider_name = mapped_provider_name(normalized_letter);
    let provider_is_virtual = provider_name.as_deref().is_some_and(has_virtual_markers);

    if ok.is_err() {
        if !provider_is_virtual {
            return None;
        }

        return Some(DetectedVirtualDrive {
            letter: normalized_letter,
            label: provider_name
                .clone()
                .unwrap_or_else(|| format!("{}:\\", normalized_letter)),
            file_system: provider_name.unwrap_or_else(|| "Virtual".to_string()),
        });
    }

    let volume_len = volume_name
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(volume_name.len());
    let fs_len = file_system_name
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(file_system_name.len());

    let volume = String::from_utf16_lossy(&volume_name[..volume_len]);
    let file_system = String::from_utf16_lossy(&file_system_name[..fs_len]);
    let is_virtual =
        has_virtual_markers(&volume) || has_virtual_markers(&file_system) || provider_is_virtual;

    if !is_virtual {
        return None;
    }

    Some(DetectedVirtualDrive {
        letter: normalized_letter,
        label: volume,
        file_system,
    })
}

pub fn detect_virtual_drives() -> Vec<DetectedVirtualDrive> {
    let mut virtual_drives = Vec::new();

    for (path, label) in get_all_drives() {
        if let Some(drive_letter) = path.chars().next() {
            if let Some(mut drive) = detect_virtual_drive(drive_letter) {
                if drive.label.trim().is_empty() {
                    drive.label = label;
                }
                virtual_drives.push(drive);
            }
        }
    }

    virtual_drives.sort_by_key(|drive| drive.letter);
    virtual_drives
}

fn create_default_config_from_detected_drives() -> Result<VirtualDriveConfig, String> {
    let detected_drives = detect_virtual_drives();
    let config = VirtualDriveConfig::from_detected_virtual_drives(&detected_drives);
    config.save()?;
    Ok(config)
}

/// Global configuration instance
static CONFIG: OnceLock<Arc<Mutex<VirtualDriveConfig>>> = OnceLock::new();

fn get_config() -> &'static Arc<Mutex<VirtualDriveConfig>> {
    CONFIG.get_or_init(|| Arc::new(Mutex::new(VirtualDriveConfig::load())))
}

/// Get override for a specific drive letter
pub fn get_drive_override(drive_letter: char) -> Option<DiskTypeOverride> {
    get_config().lock().ok()?.get_override(drive_letter)
}

/// Set override for a drive letter and save to disk
pub fn set_drive_override(drive_letter: char, disk_type: DiskTypeOverride) -> Result<(), String> {
    let config = get_config();
    let mut cfg = config.lock().map_err(|e| format!("Lock error: {}", e))?;
    cfg.set_override(drive_letter, disk_type);
    cfg.save()
}

/// Remove override for a drive letter and save to disk
pub fn remove_drive_override(drive_letter: char) -> Result<(), String> {
    let config = get_config();
    let mut cfg = config.lock().map_err(|e| format!("Lock error: {}", e))?;
    cfg.remove_override(drive_letter);
    cfg.save()
}

/// Get all configured overrides
pub fn get_all_overrides() -> FxHashMap<char, DiskTypeOverride> {
    get_config()
        .lock()
        .map(|cfg| cfg.overrides.clone())
        .unwrap_or_default()
}

/// Ensure the config file exists on disk.
///
/// When the file is missing, the app detects current virtual drives and creates
/// a default config with SSD entries so users can adjust them later in settings.
pub fn ensure_config_exists() -> Result<(), String> {
    let path = VirtualDriveConfig::config_path();
    if path.exists() {
        return Ok(());
    }

    let config = create_default_config_from_detected_drives()?;

    if let Some(global) = CONFIG.get() {
        let mut cfg = global.lock().map_err(|e| format!("Lock error: {}", e))?;
        *cfg = config;
    }

    log::info!("[Config] Created virtual drive configuration at {:?}", path);
    Ok(())
}

/// Reload configuration from disk (useful after external changes)
pub fn reload() {
    let config = get_config();
    if let Ok(mut cfg) = config.lock() {
        *cfg = VirtualDriveConfig::load();
    }
}
