//! Move files by identity-bound handles. Copy failures never authorize removal.
pub(crate) mod source;
use crate::{cache::CacheMode, plan, scheduler::Permit, telemetry::TimingStage, Progress, Summary};
use std::io;
use std::sync::atomic::Ordering;

pub(crate) fn try_rename_single_directory(
    sources: &[std::path::PathBuf],
    destination: &std::path::Path,
    progress: &Progress,
) -> io::Result<Option<Summary>> {
    try_rename_single_directory_with(sources, destination, progress, |file, target| {
        source::rename(file, target, false)
    })
}

fn try_rename_single_directory_with(
    sources: &[std::path::PathBuf],
    destination: &std::path::Path,
    progress: &Progress,
    rename: impl FnOnce(&std::fs::File, &std::path::Path) -> io::Result<()>,
) -> io::Result<Option<Summary>> {
    if sources.len() != 1 {
        return Ok(None);
    }
    progress.checkpoint()?;

    let submitted_source = &sources[0];
    plan::check_path(submitted_source)?;
    plan::check_path(destination)?;
    let resolved_source = crate::path_resolution::resolve(submitted_source).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("Source path resolution failed: {error}"),
        )
    })?;
    let resolved_destination = crate::path_resolution::resolve(destination).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("Destination path resolution failed: {error}"),
        )
    })?;
    let source_path = resolved_source.path;
    let destination_path = resolved_destination.path;
    plan::check_path(&source_path)?;
    plan::check_path(&destination_path)?;

    if !destination_path.is_dir() {
        return Err(io::Error::other("Destination must be a directory"));
    }
    let metadata = std::fs::symlink_metadata(&source_path)?;
    if plan::is_link(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Links and reparse points are not supported",
        ));
    }
    if !metadata.is_dir() {
        return Ok(None);
    }
    if crate::path_resolution::contains_directory(&source_path, &destination_path)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Cannot move a directory into itself",
        ));
    }

    let name = source_path
        .file_name()
        .ok_or_else(|| io::Error::other("Cannot move a volume root"))?;
    let target = destination_path.join(name);
    match plan::require_absent(&target) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(None),
        Err(error) => return Err(error),
    }

    let expected_source = source::Directory::capture(&source_path, &target)?;
    let expected_destination = source::Directory::capture(&destination_path, &target)?;
    let source_parent = source_path
        .parent()
        .ok_or_else(|| io::Error::other("Invalid source parent"))?;
    let _source_pins = source::pin_parents(source_parent)?;
    let destination_pins = source::pin_parents(&destination_path)?;
    expected_destination.validate_open_handle(
        destination_pins
            .last()
            .ok_or_else(|| io::Error::other("Destination directory was not pinned"))?,
    )?;

    // Parent handles prevent path substitution. Recheck the target after pinning;
    // the eventual handle rename also refuses replacement if it appears later.
    match plan::require_absent(&target) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(None),
        Err(error) => return Err(error),
    }

    let file = source::open(&source_path, true)?;
    expected_source.validate_open_handle(&file)?;
    match plan::require_absent(&target) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(None),
        Err(error) => return Err(error),
    }
    progress.checkpoint()?;

    let rename_result = {
        let _rename_timer = progress
            .telemetry
            .measure(progress, TimingStage::MoveRename);
        rename(&file, &target)
    };
    match rename_result {
        Ok(()) => {
            let reported_target = destination.join(name);
            progress.directories_moved.fetch_add(1, Ordering::Relaxed);
            progress.copying.store(true, Ordering::Relaxed);
            Ok(Some(Summary {
                directories_moved: 1,
                moved_pairs: vec![(submitted_source.clone(), reported_target.clone())],
                fast_moved_directory: Some((submitted_source.clone(), reported_target.clone())),
                destinations: vec![reported_target],
                cancelled: progress.is_cancelled(),
                ..Summary::default()
            }))
        }
        // The handle rename is atomic and does not replace an existing target.
        // Let the established inventory/conflict flow handle cross-volume and
        // other rename failures. It revalidates sources before transferring.
        Err(error) => {
            if error.raw_os_error() == Some(17) {
                progress.telemetry.record_cross_volume_directory_fallback();
            }
            Ok(None)
        }
    }
}

pub(crate) fn validate_selection(
    sources: &[std::path::PathBuf],
    destination: &std::path::Path,
    progress: &Progress,
) -> io::Result<()> {
    let destination = crate::path_resolution::resolve(destination)?.path;
    let destination_key = crate::target::key(&destination);
    let mut roots = Vec::<String>::new();
    for path in sources {
        progress.checkpoint()?;
        plan::check_path(path)?;
        let path = crate::path_resolution::resolve(path)?.path;
        if path
            .parent()
            .is_some_and(|parent| crate::target::key(parent) == destination_key)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Cannot move an item into its current folder",
            ));
        }
        // Trailing separators make an ancestor sort immediately before its
        // subtree, without confusing sibling names such as "folder-extra".
        roots.push(format!(
            "{}\\",
            crate::target::key(&path).trim_end_matches('\\')
        ));
    }
    roots.sort_unstable();
    if roots.windows(2).any(|pair| pair[1].starts_with(&pair[0])) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Overlapping move sources are not supported",
        ));
    }
    Ok(())
}

/// Err means no destination was published. Ok(Some(error)) means a destination
/// was published but its source was retained; callers must refresh both sides.
pub(crate) fn transfer<'a>(
    item: &plan::File,
    progress: &'a Progress,
    permit: Option<&'a Permit<'a>>,
    cache: CacheMode,
) -> io::Result<Option<io::Error>> {
    transfer_with(item, progress, permit, cache, false, || Ok(()))
}

fn transfer_with<'a>(
    item: &plan::File,
    progress: &'a Progress,
    permit: Option<&'a Permit<'a>>,
    cache: CacheMode,
    force_copy: bool,
    after_publish: impl FnOnce() -> io::Result<()>,
) -> io::Result<Option<io::Error>> {
    let move_setup_timer = progress.telemetry.measure(progress, TimingStage::MoveSetup);
    let checkpoint = || match permit {
        Some(permit) => permit.checkpoint(),
        None => progress.checkpoint(),
    };
    checkpoint()?;
    let source_parent = item
        .source
        .parent()
        .ok_or_else(|| io::Error::other("Invalid source parent"))?;
    let target_parent = item
        .destination
        .parent()
        .ok_or_else(|| io::Error::other("Invalid target parent"))?;
    let _source_pins = source::pin_parents(source_parent)?;
    let _target_pins = source::pin_parents(target_parent)?;
    let file = source::open(&item.source, false)?;
    let expected = item
        .move_source
        .as_ref()
        .ok_or_else(|| io::Error::other("Missing move source identity"))?;
    if crate::replacement::Snapshot::from_file(&file)? != *expected {
        return Err(io::Error::other("Move source changed after preflight"));
    }
    let target_guard = if !force_copy {
        let target_guard = match &item.replacement {
            Some(snapshot) => Some(snapshot.hold(&item.destination)?),
            None => {
                plan::require_absent(&item.destination)?;
                None
            }
        };
        checkpoint()?;
        if let Some(snapshot) = &item.replacement {
            snapshot.validate(&item.destination)?;
        }
        target_guard
    } else {
        None
    };
    drop(target_guard);
    drop(move_setup_timer);
    if !force_copy {
        let _active = progress.track_file(&item.source);
        let rename_result = {
            let _rename_timer = progress
                .telemetry
                .measure(progress, TimingStage::MoveRename);
            source::rename(&file, &item.destination, item.replacement.is_some())
        };
        match rename_result {
            Ok(()) => {
                progress
                    .bytes_copied
                    .fetch_add(item.size, Ordering::Relaxed);
                return Ok(None);
            }
            // Only an explicit cross-device failure selects staged copying.
            Err(error) if error.raw_os_error() == Some(17) => {
                progress.telemetry.record_cross_volume_file_fallback();
            }
            Err(error) => return Err(error),
        }
    }
    let Some(published_file) = crate::platform::copy_file_impl(item, progress, permit, cache)?
    else {
        return Ok(Some(io::Error::other(
            "Published move identity unavailable; source retained",
        )));
    };
    // Bind the published destination to the staged identity before removing the
    // source. Deny writes/deletes until disposition completes.
    let finish = || -> io::Result<()> {
        let finalize_timer = progress
            .telemetry
            .measure(progress, TimingStage::MoveFinalize);
        after_publish()?;
        checkpoint()?;
        let destination = source::protect_destination(&item.destination)?;
        let published = crate::replacement::Snapshot::from_file(&destination)?;
        let changed_fields = published_file
            .snapshot
            .published_move_difference_fields(&published);
        if !changed_fields.is_empty() {
            return Err(io::Error::other(format!(
                "Published move target changed ({}); source retained",
                changed_fields.join(", ")
            )));
        }
        checkpoint()?;
        drop(finalize_timer);
        let _remove_timer = progress
            .telemetry
            .measure(progress, TimingStage::SourceRemove);
        source::remove(&file)
    };
    Ok(finish().err())
}

pub(crate) fn cleanup(
    directories: &[source::Directory],
    progress: &Progress,
    summary: &mut Summary,
) {
    let _participant = progress.participant();
    for directory in directories.iter().rev() {
        if progress.checkpoint().is_err() {
            break;
        }
        match directory.remove_empty() {
            Ok(()) => {
                progress.directories_moved.fetch_add(1, Ordering::Relaxed);
                summary
                    .moved_pairs
                    .push((directory.path.clone(), directory.destination.clone()));
            }
            // Nonempty directories include skipped items and new arrivals. Do
            // not enumerate/delete them recursively or claim they were moved.
            Err(error) if error.raw_os_error() == Some(145) => {
                progress.sources_retained.fetch_add(1, Ordering::Relaxed);
            }
            Err(error) => {
                progress.sources_retained.fetch_add(1, Ordering::Relaxed);
                summary.failures += 1;
                summary.first_error.get_or_insert(error);
            }
        }
    }
}

/// Keep OS-resolved/verbatim paths internal. Consumers index tags/caches using
/// the namespace submitted by the caller, which may be a mounted-volume alias.
pub(crate) fn report_paths(plan: &plan::Plan, summary: &mut Summary) {
    let destination = |path: &std::path::Path| {
        plan.move_destination
            .as_ref()
            .and_then(|(resolved, submitted)| {
                path.strip_prefix(resolved)
                    .ok()
                    .map(|relative| submitted.join(relative))
            })
            .unwrap_or_else(|| path.to_owned())
    };
    for (source, target) in &mut summary.moved_pairs {
        if let Some((resolved, submitted)) = source.ancestors().find_map(|ancestor| {
            plan.move_roots
                .get(ancestor)
                .map(|submitted| (ancestor, submitted))
        }) {
            let relative = source
                .strip_prefix(resolved)
                .expect("Matched source ancestor");
            *source = if relative.as_os_str().is_empty() {
                submitted.clone()
            } else {
                submitted.join(relative)
            };
        }
        *target = destination(target);
    }
    for path in &mut summary.destinations {
        *path = destination(path);
    }
}

#[cfg(all(test, windows))]
mod tests;
