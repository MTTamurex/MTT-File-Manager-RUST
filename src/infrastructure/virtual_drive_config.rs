//! Configuration for virtual drive disk type overrides
//!
//! Allows users to manually configure whether virtual drives (Cryptomator, etc.)
//! should be treated as SSD or HDD for I/O optimization purposes.

use rustc_hash::FxHashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

/// Disk type override for a virtual drive
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskTypeOverride {
    /// Treat as SSD (no seek penalty optimization)
    SSD,
    /// Treat as HDD (enable seek penalty optimization)
    HDD,
}

/// Configuration for virtual drive disk types
#[derive(Debug, Clone)]
pub struct VirtualDriveConfig {
    /// Map of drive letter to disk type override
    /// Key is uppercase drive letter (e.g., 'X', 'Y', 'Z')
    pub overrides: FxHashMap<char, DiskTypeOverride>,
}

impl Default for VirtualDriveConfig {
    fn default() -> Self {
        Self {
            overrides: FxHashMap::default(),
        }
    }
}

impl VirtualDriveConfig {
    /// Get the config file path (in current directory)
    fn config_path() -> PathBuf {
        PathBuf::from("virtual_drive_config.json")
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
                                    config.overrides.insert(drive_letter.to_ascii_uppercase(), disk_type);
                                }
                            }
                        }
                    }
                    
                    eprintln!("[Config] Loaded virtual drive configuration from {:?}", path);
                    config
                }
                Err(e) => {
                    eprintln!("[Config] Failed to parse config file: {} - using defaults", e);
                    Self::default()
                }
            },
            Err(e) => {
                eprintln!("[Config] Failed to read config file: {} - using defaults", e);
                Self::default()
            }
        }
    }

    /// Save configuration to file
    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_path();
        
        // Build JSON manually
        let mut overrides_map = serde_json::Map::new();
        for (drive_letter, disk_type) in &self.overrides {
            let type_str = match disk_type {
                DiskTypeOverride::SSD => "SSD",
                DiskTypeOverride::HDD => "HDD",
            };
            overrides_map.insert(drive_letter.to_string(), serde_json::Value::String(type_str.to_string()));
        }
        
        let json_obj = serde_json::json!({
            "overrides": overrides_map
        });
        
        let json = serde_json::to_string_pretty(&json_obj)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;

        fs::write(&path, json)
            .map_err(|e| format!("Failed to write config file: {}", e))?;

        eprintln!("[Config] Saved virtual drive configuration to {:?}", path);
        Ok(())
    }

    /// Get override for a drive letter
    pub fn get_override(&self, drive_letter: char) -> Option<DiskTypeOverride> {
        self.overrides.get(&drive_letter.to_ascii_uppercase()).copied()
    }

    /// Set override for a drive letter
    pub fn set_override(&mut self, drive_letter: char, disk_type: DiskTypeOverride) {
        self.overrides.insert(drive_letter.to_ascii_uppercase(), disk_type);
    }

    /// Remove override for a drive letter
    pub fn remove_override(&mut self, drive_letter: char) {
        self.overrides.remove(&drive_letter.to_ascii_uppercase());
    }

    /// Clear all overrides
    pub fn clear(&mut self) {
        self.overrides.clear();
    }
}

/// Global configuration instance
static CONFIG: OnceLock<Arc<Mutex<VirtualDriveConfig>>> = OnceLock::new();

fn get_config() -> &'static Arc<Mutex<VirtualDriveConfig>> {
    CONFIG.get_or_init(|| Arc::new(Mutex::new(VirtualDriveConfig::load())))
}

/// Get override for a specific drive letter
pub fn get_drive_override(drive_letter: char) -> Option<DiskTypeOverride> {
    get_config()
        .lock()
        .ok()?
        .get_override(drive_letter)
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

/// Reload configuration from disk (useful after external changes)
pub fn reload() {
    let config = get_config();
    if let Ok(mut cfg) = config.lock() {
        *cfg = VirtualDriveConfig::load();
    }
}
