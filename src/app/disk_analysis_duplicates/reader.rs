//! Validated reads shared by the prefix and full-confirmation passes.

use super::{FileIdentity, PathResolver, READ_CHUNK};
use std::fs::File;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows::{capture, open};
#[cfg(not(windows))]
mod portable;
#[cfg(not(windows))]
use portable::{capture, open};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FileStamp {
    pub identity: FileIdentity,
    pub size: u64,
    pub creation: u64,
    pub modified: u64,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ValidatedHash {
    pub digest: [u8; 32],
    pub stamp: FileStamp,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ReadJob {
    pub idx: u32,
    pub size: u64,
    /// Required when reopening a candidate selected by its earlier prefix.
    pub previous: Option<FileStamp>,
}

#[derive(Debug)]
pub(super) enum ReadOutcome {
    Hashed(ValidatedHash),
    Inaccessible,
    Unavailable,
    Changed,
    Cancelled,
}

pub(super) fn read_candidate(
    resolver: &PathResolver,
    job: ReadJob,
    limit: Option<u64>,
    cancel: &AtomicBool,
    mut on_read: impl FnMut(u64),
) -> ReadOutcome {
    if cancel.load(Ordering::Acquire) {
        return ReadOutcome::Cancelled;
    }
    let Some(path) = resolver(job.idx) else {
        return ReadOutcome::Inaccessible;
    };
    let (mut file, before) = match open(&path) {
        Ok(opened) => opened,
        Err(outcome) => return outcome,
    };
    if before.size != job.size || job.previous.is_some_and(|stamp| stamp != before) {
        return ReadOutcome::Changed;
    }
    let expected = limit.unwrap_or(job.size).min(job.size);
    let mut remaining = expected;
    let mut buffer = vec![0; expected.min(READ_CHUNK as u64) as usize];
    let mut hasher = blake3::Hasher::new();
    while remaining > 0 {
        if cancel.load(Ordering::Acquire) {
            return ReadOutcome::Cancelled;
        }
        let chunk = remaining.min(buffer.len() as u64) as usize;
        match file.read(&mut buffer[..chunk]) {
            Ok(0) => return ReadOutcome::Changed,
            Ok(count) => {
                hasher.update(&buffer[..count]);
                remaining -= count as u64;
                on_read(count as u64);
            }
            Err(_) => return ReadOutcome::Inaccessible,
        }
    }
    if cancel.load(Ordering::Acquire) {
        return ReadOutcome::Cancelled;
    }
    match capture(&file) {
        Ok(after) if after == before => ReadOutcome::Hashed(ValidatedHash {
            digest: hasher.finalize().into(),
            stamp: after,
        }),
        Ok(_) => ReadOutcome::Changed,
        Err(outcome) => outcome,
    }
}
