//! Shared Windows fallback, with checkpoints before accepting/starting a stage.
use super::{
    log_extraction_perf, log_extraction_stage_failure, stage3_shell_api, stage5_media_foundation,
    ThumbnailExtractionOutcome as Outcome,
};
use crate::infrastructure::{
    onedrive,
    windows::file_flags::{classify_file_read_safety, FileReadSafety},
};
use std::{path::Path, time::Instant};

pub(super) fn check_source(path: &Path, cancelled: &dyn Fn() -> bool) -> Option<Outcome> {
    if cancelled() {
        return Some(Outcome::Cancelled);
    }
    if !onedrive::fast_path_exists(path) {
        return Some(Outcome::Failed);
    }
    let safety = classify_file_read_safety(path);
    if safety != FileReadSafety::Safe {
        return Some(Outcome::UnsafeToRead(safety));
    }
    // Attribute/safety probes can take time on network or provider-backed files.
    cancelled().then_some(Outcome::Cancelled)
}

pub(super) fn extract(path: &Path, max_side: Option<u32>, cancelled: &dyn Fn() -> bool) -> Outcome {
    run(
        || {
            let start = Instant::now();
            match stage3_shell_api::extract_with_size(path, max_side) {
                Ok(result) => {
                    log_extraction_perf(path, "stage3_shell", start, &result);
                    Some(result)
                }
                Err(_) => {
                    log_extraction_stage_failure(path, "stage3_shell", start);
                    None
                }
            }
        },
        || {
            let start = Instant::now();
            let result = stage5_media_foundation::extract(path);
            if let Some(ref result) = result {
                log_extraction_perf(path, "stage5_mf", start, result);
            } else {
                log_extraction_stage_failure(path, "stage5_mf", start);
            }
            result
        },
        || check_source(path, cancelled),
    )
}

fn run(
    shell: impl FnOnce() -> Option<(Vec<u8>, u32, u32)>,
    media_foundation: impl FnOnce() -> Option<(Vec<u8>, u32, u32)>,
    check: impl Fn() -> Option<Outcome>,
) -> Outcome {
    if let Some(outcome) = check() {
        return outcome;
    }
    let shell_result = shell();
    if let Some(outcome) = check() {
        return outcome;
    }
    if let Some(result) = shell_result {
        return Outcome::Success(result);
    }
    // MF stays behind Shell providers: a decoded but poor frame is still possible.
    let result = media_foundation();
    if let Some(outcome) = check() {
        return outcome;
    }
    result.map_or(Outcome::Failed, Outcome::Success)
}

#[cfg(test)]
mod tests;
