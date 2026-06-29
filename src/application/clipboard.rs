use crate::application::file_operations;
use crate::infrastructure::windows_clipboard;
use std::cell::Cell;
use std::path::PathBuf;

/// Clipboard operation type
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ClipboardOp {
    Copy,
    Move,
}

/// Manages clipboard content and operations
#[derive(Clone, Debug, Default)]
pub struct ClipboardManager {
    /// Internal clipboard state (fallback/cache)
    internal_files: Vec<PathBuf>,
    internal_op: Option<ClipboardOp>,
    /// Clipboard sequence when internal file state was last synced to system clipboard.
    internal_sync_sequence: Option<u32>,
    /// Cached answer for whether the current clipboard sequence contains file payloads.
    cached_system_has_files: Cell<Option<bool>>,
    cached_system_has_files_sequence: Cell<Option<u32>>,
}

impl ClipboardManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Helper to get internal state (read-only)
    pub fn internal_state(&self) -> (&[PathBuf], Option<ClipboardOp>) {
        (&self.internal_files, self.internal_op)
    }

    /// Checks if there is content to paste (System or Internal)
    pub fn has_content(&self) -> bool {
        let current_sequence = windows_clipboard::clipboard_sequence_number();

        if self.cached_system_has_files_for_sequence(current_sequence) {
            return true;
        }

        let has_system_files = windows_clipboard::has_files_in_clipboard();
        self.update_system_files_cache(current_sequence, has_system_files);

        if has_system_files {
            return true;
        }

        self.has_internal_content_for_sequence(current_sequence)
    }

    /// Clears the internal clipboard state
    pub fn clear(&mut self) {
        self.internal_files.clear();
        self.internal_op = None;
        self.internal_sync_sequence = None;
    }

    /// Copy files to clipboard (System + Internal)
    pub fn copy(&mut self, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }

        // 1. System Clipboard (prefer native file payload; fallback to text path)
        let system_file_payload_written = windows_clipboard::copy_files_to_clipboard(paths).is_ok();
        if !system_file_payload_written {
            if let Some(first) = paths.first() {
                let _ = file_operations::copy_path_to_clipboard(first);
            }
        }

        self.internal_sync_sequence = windows_clipboard::clipboard_sequence_number();
        self.update_system_files_cache(self.internal_sync_sequence, system_file_payload_written);

        // 2. Internal State
        self.internal_files = paths.to_vec();
        self.internal_op = Some(ClipboardOp::Copy);
    }

    /// Cut files (System + Internal)
    pub fn cut(&mut self, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }

        // 1. System Clipboard (prefer native file payload; fallback to text path)
        let system_file_payload_written = windows_clipboard::cut_files_to_clipboard(paths).is_ok();
        if !system_file_payload_written {
            if let Some(first) = paths.first() {
                let _ = file_operations::copy_path_to_clipboard(first);
            }
        }

        self.internal_sync_sequence = windows_clipboard::clipboard_sequence_number();
        self.update_system_files_cache(self.internal_sync_sequence, system_file_payload_written);

        // 2. Internal State
        self.internal_files = paths.to_vec();
        self.internal_op = Some(ClipboardOp::Move);
    }

    /// Returns files and operation type (is_move) for pasting.
    /// Does NOT perform the operation. Use this to prepare an async operation.
    pub fn get_files_to_paste(&self) -> Option<(Vec<PathBuf>, bool)> {
        let current_sequence = windows_clipboard::clipboard_sequence_number();

        // For copy/cut initiated inside the app, avoid synchronously reading
        // CF_HDROP from the Windows clipboard on the UI thread. Some shell
        // providers can delay-render clipboard data and stall paste startup.
        if self.internal_sync_sequence.is_some() {
            if let Some(files) = self.internal_files_to_paste_for_sequence(current_sequence) {
                return Some(files);
            }
        }

        // 1. Try System Clipboard first
        if let Some(files) = windows_clipboard::get_files_from_clipboard() {
            let op = windows_clipboard::get_clipboard_operation();
            let is_move = matches!(op, Some(windows_clipboard::ClipboardFileOp::Move));
            return Some((files, is_move));
        }

        // 2. Fallback to Internal
        self.internal_files_to_paste_for_sequence(current_sequence)
    }

    fn has_internal_content_for_sequence(&self, current_sequence: Option<u32>) -> bool {
        self.internal_files_to_paste_for_sequence(current_sequence)
            .is_some_and(|(files, _)| !files.is_empty())
    }

    fn cached_system_has_files_for_sequence(&self, current_sequence: Option<u32>) -> bool {
        match current_sequence {
            Some(sequence) if self.cached_system_has_files_sequence.get() == Some(sequence) => {
                self.cached_system_has_files.get().unwrap_or(false)
            }
            _ => false,
        }
    }

    fn update_system_files_cache(&self, current_sequence: Option<u32>, has_files: bool) {
        self.cached_system_has_files.set(Some(has_files));
        self.cached_system_has_files_sequence.set(current_sequence);
    }

    fn internal_files_to_paste_for_sequence(
        &self,
        current_sequence: Option<u32>,
    ) -> Option<(Vec<PathBuf>, bool)> {
        if self.internal_files.is_empty() {
            return None;
        }

        if let (Some(internal_seq), Some(current_seq)) =
            (self.internal_sync_sequence, current_sequence)
        {
            // Clipboard changed since our last file copy/cut (e.g. user copied text).
            // Ignore stale internal fallback to avoid unintended file paste.
            if internal_seq != current_seq {
                return None;
            }
        }

        let is_move = matches!(self.internal_op, Some(ClipboardOp::Move));
        Some((self.internal_files.clone(), is_move))
    }
}

#[cfg(test)]
mod tests {
    use super::{ClipboardManager, ClipboardOp};
    use std::path::PathBuf;

    #[test]
    fn internal_fallback_is_ignored_when_clipboard_sequence_changes() {
        let manager = ClipboardManager {
            internal_files: vec![PathBuf::from(r"C:\\temp\\a.txt")],
            internal_op: Some(ClipboardOp::Copy),
            internal_sync_sequence: Some(10),
            ..ClipboardManager::default()
        };

        assert!(manager
            .internal_files_to_paste_for_sequence(Some(11))
            .is_none());
        assert!(!manager.has_internal_content_for_sequence(Some(11)));
    }

    #[test]
    fn internal_fallback_is_kept_when_clipboard_sequence_matches() {
        let manager = ClipboardManager {
            internal_files: vec![PathBuf::from(r"C:\\temp\\a.txt")],
            internal_op: Some(ClipboardOp::Move),
            internal_sync_sequence: Some(22),
            ..ClipboardManager::default()
        };

        let (files, is_move) = manager
            .internal_files_to_paste_for_sequence(Some(22))
            .expect("expected internal clipboard fallback when sequence matches");

        assert_eq!(files, vec![PathBuf::from(r"C:\\temp\\a.txt")]);
        assert!(is_move);
        assert!(manager.has_internal_content_for_sequence(Some(22)));
    }

    #[test]
    fn internal_fallback_works_without_sequence_information() {
        let manager = ClipboardManager {
            internal_files: vec![PathBuf::from(r"C:\\temp\\a.txt")],
            internal_op: Some(ClipboardOp::Copy),
            internal_sync_sequence: None,
            ..ClipboardManager::default()
        };

        let (files, is_move) = manager
            .internal_files_to_paste_for_sequence(None)
            .expect("expected fallback when no sequence info is available");

        assert_eq!(files.len(), 1);
        assert!(!is_move);
    }
}
