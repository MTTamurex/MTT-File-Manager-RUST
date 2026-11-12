//! Depth-first permanent deletion, with cancellation between filesystem calls.
//! Directory handles deny rename/delete while children are traversed. Reparse
//! points are removed as entries, never descended into.
use crate::infrastructure::copy_engine::CopyOperation;
use std::fs::{self, File, OpenOptions, ReadDir};
use std::io;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_SHARE_READ, FILE_SHARE_WRITE,
};

struct Directory {
    path: PathBuf,
    entries: ReadDir,
    pin: File,
}

fn cancelled(operation: &CopyOperation) -> io::Result<()> {
    if operation.progress.is_cancelled() {
        Err(io::ErrorKind::Interrupted.into())
    } else {
        Ok(())
    }
}

fn remove_entry(
    path: &Path,
    operation: &CopyOperation,
    stack: &mut Vec<Directory>,
    removed: &super::RemovalObserver,
) -> io::Result<()> {
    cancelled(operation)?;
    let progress = operation.deletion.as_ref().expect("deletion operation");
    progress.set_current(Some(path));
    // OPEN_REPARSE_POINT inspects the entry itself, not its target.
    let pin = OpenOptions::new()
        .access_mode(0)
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)?;
    let metadata = pin.metadata()?;
    if metadata.is_dir() && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0 {
        stack.push(Directory {
            path: path.to_path_buf(),
            entries: fs::read_dir(path)?,
            pin,
        });
    } else {
        drop(pin);
        cancelled(operation)?;
        if metadata.file_attributes() & 0x10 != 0 {
            fs::remove_dir(path)?;
        } else {
            fs::remove_file(path)?;
        }
        progress.removed();
        removed(path);
    }
    Ok(())
}

pub(crate) fn delete_with_observer(
    path: &Path,
    operation: &CopyOperation,
    removed: &super::RemovalObserver,
) -> io::Result<()> {
    // Pin every ancestor before traversal, top-down. This prevents an ancestor
    // being renamed/replaced with a junction after validation.
    let mut ancestors: Vec<_> = path.ancestors().skip(1).collect();
    ancestors.reverse();
    let mut pins = Vec::with_capacity(ancestors.len());
    for ancestor in ancestors {
        cancelled(operation)?;
        let pin = OpenOptions::new()
            .access_mode(0)
            .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(ancestor)?;
        if pin.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Reparse ancestor refused",
            ));
        }
        pins.push(pin);
    }
    let mut stack = Vec::new();
    remove_entry(path, operation, &mut stack, removed)?;
    while !stack.is_empty() {
        cancelled(operation)?;
        let next = stack.last_mut().unwrap().entries.next();
        match next {
            Some(Ok(entry)) => {
                if let Err(error) = remove_entry(&entry.path(), operation, &mut stack, removed) {
                    if error.kind() == io::ErrorKind::Interrupted {
                        return Err(error);
                    }
                    operation.deletion.as_ref().unwrap().failed();
                }
            }
            Some(Err(_)) => {
                operation.deletion.as_ref().unwrap().failed();
            }
            None => {
                let Directory { path, pin, .. } = stack.pop().unwrap();
                drop(pin);
                cancelled(operation)?;
                operation
                    .deletion
                    .as_ref()
                    .unwrap()
                    .set_current(Some(&path));
                match fs::remove_dir(&path) {
                    Ok(()) => {
                        operation.deletion.as_ref().unwrap().removed();
                        removed(&path);
                    }
                    Err(error) if stack.is_empty() => return Err(error),
                    Err(_) => operation.deletion.as_ref().unwrap().failed(),
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
fn delete(path: &Path, operation: &CopyOperation) -> io::Result<()> {
    delete_with_observer(
        path,
        operation,
        &(std::sync::Arc::new(|_: &Path| {}) as super::RemovalObserver),
    )
}

#[cfg(test)]
mod tests;
