use super::{
    progress::Reporter, DeleteFailure, DeleteProgress, DeleteStage, DeleteStatus, Request,
};
use crate::app::disk_analysis_duplicates::{
    reader::{self, FileStamp},
    READ_CHUNK,
};
use std::{
    fs::File,
    io::Read,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};

fn guarded(path: &Path, stamp: FileStamp, preserve: bool) -> Result<File, ()> {
    let (file, actual) = reader::open_for_deletion(path, preserve).map_err(|_| ())?;
    if actual != stamp || !file.metadata().map_err(|_| ())?.is_file() {
        return Err(());
    }
    Ok(file)
}

fn hash(
    file: &mut File,
    stamp: FileStamp,
    cancel: &AtomicBool,
    progress: &mut Reporter<'_>,
) -> Result<[u8; 32], ()> {
    let mut buffer = vec![0; READ_CHUNK];
    let mut remaining = stamp.size;
    let mut hasher = blake3::Hasher::new();
    while remaining > 0 {
        if cancel.load(Ordering::Acquire) {
            return Err(());
        }
        let chunk = remaining.min(buffer.len() as u64) as usize;
        let count = file.read(&mut buffer[..chunk]).map_err(|_| ())?;
        if count == 0 {
            return Err(());
        }
        hasher.update(&buffer[..count]);
        remaining -= count as u64;
        progress.read(stamp.size - remaining);
    }
    if reader::capture(file).map_err(|_| ())? != stamp {
        return Err(());
    }
    Ok(hasher.finalize().into())
}

fn still_matches(path: &Path, stamp: FileStamp) -> bool {
    reader::open(path).is_ok_and(|(_, actual)| actual == stamp)
}

#[cfg(test)]
pub(super) fn execute(
    request: &Request,
    cancel: &AtomicBool,
    shell: impl FnOnce(&Path) -> bool,
) -> (DeleteStatus, bool) {
    execute_with_progress(
        request,
        cancel,
        &Mutex::new(DeleteProgress::default()),
        shell,
    )
}

pub(super) fn execute_with_progress(
    request: &Request,
    cancel: &AtomicBool,
    shared: &Mutex<DeleteProgress>,
    shell: impl FnOnce(&Path) -> bool,
) -> (DeleteStatus, bool) {
    let mut progress = Reporter::new(shared);
    let invalid = |progress: &Reporter<'_>, reason| {
        if cancel.load(Ordering::Acquire) {
            (DeleteStatus::CancelledOrFailed, false)
        } else {
            progress.fail(reason);
            (DeleteStatus::Unsafe, false)
        }
    };
    progress.begin(DeleteStage::Pair, 0);
    if cancel.load(Ordering::Acquire) {
        return (DeleteStatus::CancelledOrFailed, false);
    }
    if request.target.idx == request.survivor.idx
        || super::super::candidates::is_recycle_path(&request.target_path)
        || super::super::candidates::is_recycle_path(&request.survivor_path)
        || request.target_path == request.survivor_path
        || request.target.physical_identity() == request.survivor.physical_identity()
        || request.target.stamp.size != request.survivor.stamp.size
    {
        return invalid(&progress, DeleteFailure::InvalidPair);
    }
    // Deny writes to both files and deny deletion of the independent survivor
    // throughout validation and the native Shell operation (including dialogs).
    progress.begin(DeleteStage::Metadata, 0);
    let Ok(mut survivor) = guarded(&request.survivor_path, request.survivor.stamp, true) else {
        return invalid(&progress, DeleteFailure::SurvivorUnavailableOrChanged);
    };
    let Ok(mut target) = guarded(&request.target_path, request.target.stamp, false) else {
        return invalid(&progress, DeleteFailure::TargetUnavailableOrChanged);
    };
    progress.begin(DeleteStage::TargetContent, request.target.stamp.size);
    let Ok(target_hash) = hash(&mut target, request.target.stamp, cancel, &mut progress) else {
        return invalid(&progress, DeleteFailure::TargetReadFailed);
    };
    progress.begin(DeleteStage::SurvivorContent, request.survivor.stamp.size);
    let Ok(survivor_hash) = hash(&mut survivor, request.survivor.stamp, cancel, &mut progress)
    else {
        return invalid(&progress, DeleteFailure::SurvivorReadFailed);
    };
    if cancel.load(Ordering::Acquire) {
        return (DeleteStatus::CancelledOrFailed, false);
    }
    progress.begin(DeleteStage::Revalidate, 0);
    if target_hash != survivor_hash {
        return invalid(&progress, DeleteFailure::ContentMismatch);
    }
    if !still_matches(&request.target_path, request.target.stamp)
        || !still_matches(&request.survivor_path, request.survivor.stamp)
    {
        return invalid(&progress, DeleteFailure::PathsChanged);
    }
    // SHFileOperationW accepts a path, not a handle: these checks cannot turn
    // it into an atomic compare-and-delete. There is intentionally no raw
    // filesystem deletion fallback if the Shell cancels or fails.
    if cancel.load(Ordering::Acquire) {
        return (DeleteStatus::CancelledOrFailed, false);
    }
    progress.begin(DeleteStage::Windows, 0);
    let _ = shell(&request.target_path);
    let missing = request
        .target_path
        .symlink_metadata()
        .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound);
    let status = if missing {
        DeleteStatus::Removed
    } else {
        DeleteStatus::CancelledOrFailed
    };
    // Explicit drops keep the survivor protected until the native call returns.
    drop(target);
    drop(survivor);
    (status, true)
}

#[cfg(test)]
mod tests;
