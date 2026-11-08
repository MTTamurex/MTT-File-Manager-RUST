use super::{Error, Failure};
use crate::{
    infrastructure::windows::file_flags::FileReadSafety,
    workers::thumbnail::extraction::ThumbnailExtractionOutcome as Outcome,
};
use std::path::Path;

pub fn extract(path: &Path, max_side: u32, cancelled: &dyn Fn() -> bool) -> Outcome {
    let start = std::time::Instant::now();
    let result = super::extract(path, max_side, cancelled);
    // Never ship a late result (or persist it) after navigation/deletion.
    if cancelled() {
        return Outcome::Cancelled;
    }
    match result {
        Ok(frame) => {
            log::info!(
                "[MPV-THUMB] extracted elapsed_ms={} width={} height={}",
                start.elapsed().as_millis(),
                frame.width,
                frame.height
            );
            Outcome::Success((frame.rgba, frame.width, frame.height))
        }
        Err(Error::Cancelled) => Outcome::Cancelled,
        Err(Error::Helper(Failure::IncompleteDownload)) => {
            Outcome::UnsafeToRead(FileReadSafety::IncompleteDownload)
        }
        Err(Error::Helper(Failure::WriteLocked)) => {
            Outcome::UnsafeToRead(FileReadSafety::WriteLocked)
        }
        Err(Error::Helper(Failure::RecentlyChanging)) => {
            Outcome::UnsafeToRead(FileReadSafety::RecentlyChanging)
        }
        Err(Error::Decode | Error::Helper(Failure::Decode)) => Outcome::Failed,
        Err(error) => {
            log::warn!("[MPV-THUMB] supervisor failure kind={error:?}");
            Outcome::TransientFailure
        }
    }
}
