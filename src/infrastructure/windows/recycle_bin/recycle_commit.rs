//! Fast same-volume recycling for items whose Windows bin configuration is known.
//! Unknown or unsupported cases remain with the Shell implementation.
mod eligibility;
mod metadata_record;

#[cfg(test)]
mod regression_tests;

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{
    MoveFileW, SetFileAttributesW, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM,
};

pub(crate) struct RecycleCommitBatch {
    eligibility: eligibility::EligibilityBatch,
}

impl RecycleCommitBatch {
    /// `true` only after MTT has obtained confirmation for this selection.
    pub(crate) fn new(confirmation_already_shown: bool) -> Self {
        Self {
            eligibility: eligibility::EligibilityBatch::new(confirmation_already_shown),
        }
    }

    /// Shell may change both occupancy and settings, including on partial failure.
    pub(crate) fn invalidate_after_shell(&mut self) {
        self.eligibility.invalidate_after_shell();
    }

    pub(crate) fn try_recycle(&mut self, path: &Path, is_cancelled: impl Fn() -> bool) -> bool {
        if is_cancelled() {
            return false;
        }
        let Some(context) = self.eligibility.prepare(path) else {
            return false;
        };
        if is_cancelled() {
            return false;
        }
        let Some(index_data) = metadata_record::version_two_index(path, context.item_size) else {
            return false;
        };
        if commit_item_with_cancel(path, &context.recycle_directory, &index_data, &is_cancelled)
            .is_some()
        {
            self.eligibility.record_recycled(&context);
            true
        } else {
            false
        }
    }
}

fn commit_item_with_cancel(
    path: &Path,
    recycle_directory: &Path,
    index_data: &[u8],
    is_cancelled: &impl Fn() -> bool,
) -> Option<std::path::PathBuf> {
    for _ in 0..4 {
        if is_cancelled() {
            return None;
        }
        let Ok(guid) = (unsafe { windows::Win32::System::Com::CoCreateGuid() }) else {
            return None;
        };
        let mut token = std::ffi::OsString::from(format!("{:032x}", guid.to_u128()));
        if let Some(extension) = path.extension() {
            token.push(".");
            token.push(extension);
        }
        let mut index_name = std::ffi::OsString::from("$I");
        index_name.push(&token);
        let mut content_name = std::ffi::OsString::from("$R");
        content_name.push(&token);
        let index_path = recycle_directory.join(index_name);
        let content_path = recycle_directory.join(content_name);

        let mut index = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&index_path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return None,
        };
        if index.write_all(&index_data).is_err() || index.sync_all().is_err() {
            drop(index);
            let _ = fs::remove_file(&index_path);
            return None;
        }
        drop(index);

        let index_wide: Vec<u16> = index_path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        if unsafe {
            SetFileAttributesW(
                PCWSTR(index_wide.as_ptr()),
                FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM,
            )
        }
        .is_err()
        {
            let _ = fs::remove_file(&index_path);
            return None;
        }
        if is_cancelled() {
            let _ = fs::remove_file(&index_path);
            return None;
        }

        // The index is complete before moving the content. MoveFileW refuses to
        // replace an existing destination, unlike APIs that allow replacement.
        let source_wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let content_wide: Vec<u16> = content_path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        if unsafe { MoveFileW(PCWSTR(source_wide.as_ptr()), PCWSTR(content_wide.as_ptr())) }.is_ok()
        {
            return Some(content_path);
        }
        let _ = fs::remove_file(&index_path);
        return None;
    }
    None
}
