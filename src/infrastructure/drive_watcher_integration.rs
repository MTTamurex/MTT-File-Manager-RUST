//! Integration layer for DriveWatcher with the existing app architecture
//!
//! This module bridges the new drive-wide watcher (using ReadDirectoryChangesW)
//! with the existing app state and message handling system.
//!
//! Key differences from the old notify-based watcher:
//! - Watches entire drive instead of individual folders
//! - No watcher recreation on navigation (instant folder switches)
//! - Lower overhead and no missed events during transitions

use std::collections::HashMap;
use std::path::PathBuf;

use crate::infrastructure::drive_watcher::{DriveWatcher, DriveWatcherEvent};

/// Drive watcher manager that handles watchers for multiple drives
///
/// Since a user can navigate between different drives (C:\, D:\, etc.),
/// we maintain a watcher per drive and activate the appropriate one
/// based on the current path.
pub struct DriveWatcherManager {
    /// One watcher per drive root (e.g., "C:\" -> DriveWatcher)
    watchers: HashMap<PathBuf, DriveWatcher>,
    /// Current active prefix being watched
    current_prefix: PathBuf,
    /// Current drive root being watched
    current_drive: Option<PathBuf>,
}

impl DriveWatcherManager {
    /// Create a new drive watcher manager
    pub fn new() -> Self {
        Self {
            watchers: HashMap::new(),
            current_prefix: PathBuf::new(),
            current_drive: None,
        }
    }
    
    /// Set up or update watching for the given path
    ///
    /// This is called when the user navigates to a new folder.
    /// If the path is on a different drive, we ensure that drive is being watched.
    pub fn watch_path(&mut self, path: PathBuf) {
        // Extract drive root from path
        let drive_root = match DriveWatcher::extract_drive_root(&path) {
            Some(root) => root,
            None => {
                eprintln!("[DRIVE-WATCHER-MGR] Could not extract drive root from: {:?}", path);
                return;
            }
        };
        
        // Create watcher for this drive if not exists
        if !self.watchers.contains_key(&drive_root) {
            eprintln!("[DRIVE-WATCHER-MGR] Creating new watcher for drive: {:?}", drive_root);
            match DriveWatcher::new(drive_root.clone(), path.clone()) {
                Some(watcher) => {
                    self.watchers.insert(drive_root.clone(), watcher);
                }
                None => {
                    eprintln!("[DRIVE-WATCHER-MGR] Failed to create watcher for: {:?}", drive_root);
                    return;
                }
            }
        } else {
            // Update prefix on existing watcher
            if let Some(watcher) = self.watchers.get(&drive_root) {
                eprintln!("[DRIVE-WATCHER-MGR] Updating prefix for drive {:?} to: {:?}", 
                    drive_root, path);
                watcher.update_prefix(path.clone());
            }
        }
        
        self.current_prefix = path;
        self.current_drive = Some(drive_root);
    }
    
    /// Poll for file system events
    ///
    /// Returns events from the currently active drive's watcher.
    /// This should be called regularly (e.g., every frame or every 100ms).
    pub fn poll_events(&self) -> Vec<DriveWatcherEvent> {
        let Some(ref drive) = self.current_drive else {
            eprintln!("[DRIVE-WATCHER-MGR] No current drive, skipping poll");
            return Vec::new();
        };
        
        let Some(watcher) = self.watchers.get(drive) else {
            eprintln!("[DRIVE-WATCHER-MGR] No watcher for drive {:?}, skipping poll", drive);
            return Vec::new();
        };
        
        watcher.poll_events()
    }
    
    /// Check if the watcher system is active
    pub fn is_active(&self) -> bool {
        self.current_drive.as_ref()
            .and_then(|d| self.watchers.get(d))
            .map(|w| w.is_running())
            .unwrap_or(false)
    }
    
    /// Get the current watched path
    pub fn current_path(&self) -> &PathBuf {
        &self.current_prefix
    }
    
    /// Clean up watchers that haven't been used recently (optional memory management)
    pub fn cleanup_unused_watchers(&mut self, except_drive: Option<&PathBuf>) {
        self.watchers.retain(|drive, _| {
            except_drive.map(|e| e == drive).unwrap_or(false) || 
            self.current_drive.as_ref() == Some(drive)
        });
    }
}

impl Default for DriveWatcherManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Adapter to convert DriveWatcherEvent to the existing notify Event format
///
/// This allows gradual migration - existing code using notify::Event can
/// continue to work while the underlying implementation uses DriveWatcher.
pub fn convert_drive_event_to_notify_event(event: &DriveWatcherEvent) -> Option<notify::Event> {
    use notify::{Event, EventKind};
    
    let kind = match event {
        DriveWatcherEvent::Created(_) => EventKind::Create(notify::event::CreateKind::Any),
        DriveWatcherEvent::Deleted(_) => EventKind::Remove(notify::event::RemoveKind::Any),
        DriveWatcherEvent::Modified(_) => EventKind::Modify(notify::event::ModifyKind::Any),
        DriveWatcherEvent::Renamed(_, _) => EventKind::Modify(notify::event::ModifyKind::Name(notify::event::RenameMode::Any)),
        DriveWatcherEvent::Unknown(_) => EventKind::Any,
    };
    
    let paths = match event {
        DriveWatcherEvent::Created(p) => vec![p.clone()],
        DriveWatcherEvent::Deleted(p) => vec![p.clone()],
        DriveWatcherEvent::Modified(p) => vec![p.clone()],
        DriveWatcherEvent::Renamed(old, new) => vec![old.clone(), new.clone()],
        DriveWatcherEvent::Unknown(p) => vec![p.clone()],
    };
    
    Some(Event {
        kind,
        paths,
        attrs: notify::event::EventAttributes::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_drive_watcher_manager_creation() {
        let manager = DriveWatcherManager::new();
        assert!(!manager.is_active());
        assert!(manager.current_path().as_os_str().is_empty());
    }
}