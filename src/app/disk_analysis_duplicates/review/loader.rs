//! Read-only metadata and bounded thumbnails from the same validated handle.
use super::*;
use crate::app::disk_analysis_duplicates::reader::{capture, open, ReadOutcome};
use std::io::{Cursor, Read};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

mod preview;

fn date(time: std::io::Result<SystemTime>) -> Option<u64> {
    time.ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

fn failure(path: &Path, outcome: ReadOutcome) -> CopyInfo {
    let status = match outcome {
        ReadOutcome::Unavailable => CopyStatus::Unavailable,
        ReadOutcome::Changed => CopyStatus::Changed,
        _ if path
            .symlink_metadata()
            .err()
            .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            CopyStatus::Missing
        }
        _ => CopyStatus::Inaccessible,
    };
    CopyInfo::unavailable(status)
}

pub(super) fn load(
    path: &Path,
    member: &DuplicateMember,
    cancelled: impl Fn() -> bool,
) -> CopyInfo {
    if cancelled() {
        return CopyInfo::unavailable(CopyStatus::Inaccessible);
    }
    let (mut file, stamp) = match open(path) {
        Ok(opened) => opened,
        Err(outcome) => return failure(path, outcome),
    };
    if stamp != member.stamp {
        return CopyInfo::unavailable(CopyStatus::Changed);
    }
    let metadata = match file.metadata() {
        Ok(metadata) => metadata,
        Err(_) => return CopyInfo::unavailable(CopyStatus::Inaccessible),
    };
    let mut info = CopyInfo {
        status: CopyStatus::Current,
        size: Some(stamp.size),
        created: date(metadata.created()),
        modified: date(metadata.modified()),
        preview_status: PreviewStatus::NotImage,
        image: None,
    };
    if preview::supported(path) {
        let (status, image) = preview::load(&mut file, stamp.size, &cancelled);
        info.preview_status = status;
        info.image = image;
    }
    if cancelled() {
        return CopyInfo::unavailable(CopyStatus::Inaccessible);
    }
    match capture(&file) {
        Ok(after) if after == stamp => {}
        Ok(_) => return CopyInfo::unavailable(CopyStatus::Changed),
        Err(outcome) => return failure(path, outcome),
    }
    // The handle can still refer to an old file after a rename/replacement.
    // Reopen without reading content to validate the path shown to the user.
    match open(path) {
        Ok((_, after)) if after == stamp => info,
        Ok(_) => CopyInfo::unavailable(CopyStatus::Changed),
        Err(outcome) => failure(path, outcome),
    }
}
