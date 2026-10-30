use super::{File, FileStamp, ReadOutcome};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub(in crate::app::disk_analysis_duplicates) fn open_for_deletion(
    path: &Path,
    _preserve: bool,
) -> Result<(File, FileStamp), ReadOutcome> {
    open(path)
}

fn ticks(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}

pub(in crate::app::disk_analysis_duplicates) fn capture(
    file: &File,
) -> Result<FileStamp, ReadOutcome> {
    let meta = file.metadata().map_err(|_| ReadOutcome::Inaccessible)?;
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        [meta.dev(), meta.ino()]
    };
    #[cfg(not(unix))]
    return Err(ReadOutcome::Inaccessible);
    #[cfg(unix)]
    Ok(FileStamp {
        identity,
        size: meta.len(),
        creation: meta.created().map(ticks).unwrap_or(0),
        modified: ticks(meta.modified().map_err(|_| ReadOutcome::Inaccessible)?),
    })
}

pub(in crate::app::disk_analysis_duplicates) fn open(
    path: &Path,
) -> Result<(File, FileStamp), ReadOutcome> {
    for ancestor in path.ancestors() {
        let meta = ancestor
            .symlink_metadata()
            .map_err(|_| ReadOutcome::Inaccessible)?;
        if meta.file_type().is_symlink() {
            return Err(ReadOutcome::Changed);
        }
    }
    let file = File::open(path).map_err(|_| ReadOutcome::Inaccessible)?;
    let stamp = capture(&file)?;
    Ok((file, stamp))
}
