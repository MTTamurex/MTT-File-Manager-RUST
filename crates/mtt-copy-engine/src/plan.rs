use crate::{check_cancel, Progress};
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub(crate) struct File {
    pub replacement: Option<crate::replacement::Snapshot>,
    pub source: PathBuf,
    pub destination: PathBuf,
    pub size: u64,
    pub modified: Option<std::time::SystemTime>,
}

pub(crate) struct Plan {
    pub merges: HashSet<PathBuf>,
    pub files: Vec<File>,
    pub directories: Vec<PathBuf>,
    pub roots: Vec<PathBuf>,
    pub bytes: u64,
}

pub(crate) fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

/// Check ancestors before canonicalization, which otherwise hides links.
pub(crate) fn check_path(path: &Path) -> io::Result<()> {
    for ancestor in path.ancestors() {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        if is_link(&fs::symlink_metadata(ancestor)?) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Links and reparse points are not supported",
            ));
        }
    }
    Ok(())
}

pub(crate) fn require_absent(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Copy destination already exists",
        )),
    }
}

pub(crate) fn build(
    sources: &[PathBuf],
    destination: &Path,
    progress: &Progress,
) -> io::Result<Plan> {
    build_with(
        sources,
        destination,
        progress,
        crate::path_resolution::resolve,
    )
}

pub(crate) fn build_with(
    sources: &[PathBuf],
    destination: &Path,
    progress: &Progress,
    resolve: impl Fn(&Path) -> io::Result<crate::path_resolution::Resolved>,
) -> io::Result<Plan> {
    build_impl(sources, destination, progress, resolve, false)
}

pub(crate) fn build_interactive(
    sources: &[PathBuf],
    destination: &Path,
    progress: &Progress,
) -> io::Result<Plan> {
    build_impl(
        sources,
        destination,
        progress,
        crate::path_resolution::resolve,
        true,
    )
}

fn build_impl(
    sources: &[PathBuf],
    destination: &Path,
    progress: &Progress,
    resolve: impl Fn(&Path) -> io::Result<crate::path_resolution::Resolved>,
    interactive: bool,
) -> io::Result<Plan> {
    check_cancel(progress)?;
    if sources.is_empty() {
        return Err(io::Error::other("No copy sources"));
    }
    check_path(destination)?;
    let destination = resolve(destination).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("Destination path resolution failed: {error}"),
        )
    })?;
    let destination_virtual = destination.virtual_volume;
    let destination = destination.path;
    if !destination.is_dir() {
        return Err(io::Error::other("Destination must be a directory"));
    }
    let mut plan = Plan {
        merges: HashSet::new(),
        files: Vec::new(),
        directories: Vec::new(),
        roots: Vec::new(),
        bytes: 0,
    };
    let mut names = HashSet::new();
    let mut stack = Vec::new();
    for source in sources {
        check_cancel(progress)?;
        check_path(source)?;
        let resolved = resolve(source).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("Source path resolution failed: {error}"),
            )
        })?;
        let source = resolved.path;
        let name = source
            .file_name()
            .ok_or_else(|| io::Error::other("Cannot copy a volume root"))?;
        // Windows targets are normally case insensitive. Reject ambiguous
        // top-level names even on case-sensitive directories.
        if !interactive && !names.insert(name.to_string_lossy().to_lowercase()) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "Duplicate copy targets",
            ));
        }
        if destination.starts_with(&source)
            || ((resolved.virtual_volume || destination_virtual)
                && crate::path_resolution::contains_directory(&source, &destination)?)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Cannot copy a directory into itself",
            ));
        }
        let target = destination.join(name);
        if !interactive {
            require_absent(&target)?;
        }
        stack.push((source, target, true));
    }
    stack.reverse();
    let mut reserved = HashSet::new();
    while let Some((source, destination, root)) = stack.pop() {
        check_cancel(progress)?;
        let metadata = fs::symlink_metadata(&source)?;
        if is_link(&metadata) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Links and reparse points are not supported",
            ));
        }
        let (destination, replacement) = if interactive {
            match crate::target::choose(
                &source,
                &destination,
                metadata.is_dir(),
                &reserved,
                progress,
            )? {
                crate::target::Target::Skip => continue,
                crate::target::Target::Absent(path) => (path, None),
                crate::target::Target::Merge(path) => {
                    plan.merges.insert(path.clone());
                    (path, None)
                }
                crate::target::Target::Replace(path, snapshot) => (path, Some(snapshot)),
            }
        } else {
            (destination, None)
        };
        reserved.insert(crate::target::key(&destination));
        if root {
            plan.roots.push(destination.clone());
        }
        if metadata.is_dir() {
            plan.directories.push(destination.clone());
            for entry in fs::read_dir(source)? {
                check_cancel(progress)?;
                let entry = entry?;
                stack.push((entry.path(), destination.join(entry.file_name()), false));
            }
        } else if metadata.is_file() {
            plan.bytes = plan
                .bytes
                .checked_add(metadata.len())
                .ok_or_else(|| io::Error::other("Copy size overflow"))?;
            plan.files.push(File {
                replacement,
                source,
                destination,
                size: metadata.len(),
                modified: metadata.modified().ok(),
            });
        } else {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Not a regular file",
            ));
        }
    }
    Ok(plan)
}
