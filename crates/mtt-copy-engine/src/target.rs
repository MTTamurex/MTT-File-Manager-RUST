//! Preflight conflict resolution and reserved names. Links are refused before
//! any question, including when a remembered answer is available.
use crate::{
    conflicts::{Conflict, ConflictAction, ConflictKind},
    plan, Progress,
};
use std::collections::HashSet;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

pub(crate) enum Target {
    Absent(PathBuf),
    Merge(PathBuf),
    Replace(PathBuf, crate::replacement::Snapshot),
    Skip,
}

pub(crate) fn key(path: &Path) -> String {
    path.as_os_str().to_string_lossy().to_lowercase()
}

pub(crate) fn choose(
    source: &Path,
    destination: &Path,
    directory: bool,
    reserved: &HashSet<String>,
    progress: &Progress,
) -> io::Result<Target> {
    let queued = reserved.contains(&key(destination));
    let existing = match std::fs::symlink_metadata(destination) {
        Ok(metadata) => {
            plan::check_path(destination)?;
            Some(metadata)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    if !queued && existing.is_none() {
        return Ok(Target::Absent(destination.to_path_buf()));
    }
    let kind = if queued {
        ConflictKind::Queued
    } else if key(source) == key(destination) {
        ConflictKind::SamePath
    } else if let Some(metadata) = &existing {
        if directory && metadata.is_dir() {
            if crate::path_resolution::contains_directory(source, destination)? {
                ConflictKind::SamePath
            } else {
                ConflictKind::Directory
            }
        } else if !directory && metadata.is_file() {
            if crate::replacement::same_file(source, destination)? {
                ConflictKind::SamePath
            } else {
                ConflictKind::File
            }
        } else {
            ConflictKind::TypeMismatch
        }
    } else {
        ConflictKind::Queued
    };
    let request = Conflict {
        id: 0,
        source: source.to_path_buf(),
        destination: destination.to_path_buf(),
        kind,
    };
    let replacement = if kind == ConflictKind::File {
        Some(crate::replacement::Snapshot::capture(destination)?)
    } else {
        None
    };
    match progress.conflicts.ask(request, progress)? {
        ConflictAction::Replace => {
            let snapshot =
                replacement.ok_or_else(|| io::Error::other("Replacement was not authorized"))?;
            snapshot.validate(destination)?;
            Ok(Target::Replace(destination.to_path_buf(), snapshot))
        }
        ConflictAction::Merge => Ok(Target::Merge(destination.to_path_buf())),
        ConflictAction::KeepBoth => Ok(Target::Absent(unique_name(
            destination,
            directory,
            reserved,
            progress,
        )?)),
        ConflictAction::Skip => {
            progress.items_skipped.fetch_add(1, Ordering::Relaxed);
            Ok(Target::Skip)
        }
        ConflictAction::Cancel => Err(io::ErrorKind::Interrupted.into()),
    }
}

fn unique_name(
    path: &Path,
    directory: bool,
    reserved: &HashSet<String>,
    progress: &Progress,
) -> io::Result<PathBuf> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("Invalid target parent"))?;
    let stem = if directory {
        path.file_name()
    } else {
        path.file_stem()
    }
    .ok_or_else(|| io::Error::other("Invalid target name"))?;
    for index in 2u64.. {
        crate::check_cancel(progress)?;
        let mut name = OsString::from(stem);
        name.push(format!(" ({index})"));
        if !directory {
            if let Some(extension) = path.extension() {
                name.push(".");
                name.push(extension);
            }
        }
        let candidate = parent.join(name);
        if reserved.contains(&key(&candidate)) {
            continue;
        }
        match plan::require_absent(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::other("No available target name"))
}
