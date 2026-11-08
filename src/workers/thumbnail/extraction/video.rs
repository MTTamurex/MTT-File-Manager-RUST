//! Video-only production routing. Safety/cancellation are not decoder failures.
use super::{windows_fallback, ThumbnailExtractionOutcome as Outcome};
use crate::infrastructure::{
    mpv_thumbnails::{self, Error, Failure, Frame},
    windows::file_flags::FileReadSafety,
};
use std::{path::Path, time::Instant};

pub(super) fn extract(path: &Path, max_side: u32, cancelled: &dyn Fn() -> bool) -> Outcome {
    // Shutdown must not start Windows work after the libmpv service has stopped,
    // including folder-preview requests whose caller has no cancellation token.
    let should_cancel = || cancelled() || mpv_thumbnails::is_stopped();
    run(
        || {
            let start = Instant::now();
            let result = mpv_thumbnails::extract(path, max_side, &should_cancel);
            if let Ok(frame) = &result {
                log::info!(
                    "[MPV-THUMB] extracted elapsed_ms={} width={} height={}",
                    start.elapsed().as_millis(),
                    frame.width,
                    frame.height
                );
            }
            result
        },
        || windows_fallback::extract(path, Some(max_side), &should_cancel),
        &should_cancel,
        || windows_fallback::check_source(path, &should_cancel),
    )
}

fn run(
    primary: impl FnOnce() -> Result<Frame, Error>,
    fallback: impl FnOnce() -> Outcome,
    cancelled: &dyn Fn() -> bool,
    before_fallback: impl FnOnce() -> Option<Outcome>,
) -> Outcome {
    if cancelled() {
        return Outcome::Cancelled;
    }
    let result = primary();
    if cancelled() {
        return Outcome::Cancelled;
    }
    let error = match result {
        Ok(frame) => return Outcome::Success((frame.rgba, frame.width, frame.height)),
        Err(error) => error,
    };
    match error {
        Error::Cancelled => return Outcome::Cancelled,
        Error::QueueTimeout => return Outcome::TransientFailure,
        Error::Helper(Failure::IncompleteDownload) => {
            return Outcome::UnsafeToRead(FileReadSafety::IncompleteDownload)
        }
        Error::Helper(Failure::WriteLocked) => {
            return Outcome::UnsafeToRead(FileReadSafety::WriteLocked)
        }
        Error::Helper(Failure::RecentlyChanging) => {
            return Outcome::UnsafeToRead(FileReadSafety::RecentlyChanging)
        }
        _ => {}
    }
    if let Some(outcome) = before_fallback() {
        return outcome;
    }
    log::info!("[MPV-THUMB] trying Windows fallback kind={error:?}");
    let outcome = fallback();
    if cancelled() {
        return Outcome::Cancelled;
    }
    // An unavailable/crashed helper does not prove that the file is broken.
    // Keep existing retry/backoff semantics if Windows cannot decode it either.
    if matches!(outcome, Outcome::Failed)
        && !matches!(error, Error::Decode | Error::Helper(Failure::Decode))
    {
        Outcome::TransientFailure
    } else {
        outcome
    }
}

#[cfg(test)]
mod native_tests;
#[cfg(test)]
mod tests;
