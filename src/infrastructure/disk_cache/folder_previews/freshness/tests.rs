use super::*;
use crate::domain::folder_appearance::FolderAppearance;
use crate::domain::folder_preview_validity::{FolderPreviewRequests, FolderPreviewValidity};
use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use std::fs::{File, FileTimes};
use std::time::Duration;

fn set_cover_time(cover: &Path, millis: u64) {
    File::options()
        .write(true)
        .open(cover)
        .unwrap()
        .set_times(FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_millis(millis)))
        .unwrap();
}

#[test]
fn folder_preview_editing_cover_in_same_second_invalidates_every_color() {
    let dir = tempfile::tempdir().unwrap();
    let cache = ThumbnailDiskCache::new(dir.path().to_path_buf()).unwrap();
    let folder = dir.path().join("folder");
    std::fs::create_dir(&folder).unwrap();
    let cover = folder.join("cover.jpg");
    std::fs::write(&cover, b"image").unwrap();
    set_cover_time(&cover, 100_100);
    let folder_modified = std::fs::metadata(&folder).unwrap().modified().unwrap();
    let old = FolderPreviewSource::capture(&folder, Some(&cover)).unwrap();
    let validity = FolderPreviewValidity::default();
    let pixels = [230, 210, 140, 255].repeat(32 * 32);
    for appearance in FolderAppearance::ALL {
        cache.put_folder_preview_checked(&folder, 32, &pixels, 32, 32, appearance, &old, &validity);
        assert!(cache
            .get_folder_preview_for_source(&folder, 32, appearance, &old)
            .is_some());
    }

    set_cover_time(&cover, 100_200);
    assert_eq!(
        std::fs::metadata(&folder).unwrap().modified().unwrap(),
        folder_modified
    );
    assert!(!old.is_current());
    let current = FolderPreviewSource::capture(&folder, Some(&cover)).unwrap();
    for appearance in FolderAppearance::ALL {
        assert!(cache
            .get_folder_preview_for_source(&folder, 16, appearance, &current)
            .is_none());
        assert!(cache
            .get_folder_preview_for_source(&folder, 16, appearance, &old)
            .is_none());
    }

    // Fresh smaller artwork replaces a larger preview of the old content.
    let replacement = [80, 100, 130, 255].repeat(16 * 16);
    cache.put_folder_preview_checked(
        &folder,
        16,
        &replacement,
        16,
        16,
        FolderAppearance::Original,
        &current,
        &validity,
    );
    let (decoded, _, _, _) = cache
        .get_folder_preview_for_source(&folder, 16, FolderAppearance::Original, &current)
        .unwrap();
    assert_eq!(decoded, replacement);
}

#[test]
fn folder_preview_cover_selection_is_part_of_cache_validity() {
    let dir = tempfile::tempdir().unwrap();
    let cache = ThumbnailDiskCache::new(dir.path().to_path_buf()).unwrap();
    let folder = dir.path().join("folder");
    std::fs::create_dir(&folder).unwrap();
    let first = folder.join("first.jpg");
    let second = folder.join("second.jpg");
    std::fs::write(&first, b"first").unwrap();
    std::fs::write(&second, b"second").unwrap();
    let source = FolderPreviewSource::capture(&folder, Some(&first)).unwrap();
    let other = FolderPreviewSource::capture(&folder, Some(&second)).unwrap();
    let pixels = [230, 210, 140, 255].repeat(16 * 16);
    for appearance in FolderAppearance::ALL {
        cache.put_folder_preview_checked(
            &folder,
            16,
            &pixels,
            16,
            16,
            appearance,
            &source,
            &FolderPreviewValidity::default(),
        );
        assert!(cache
            .get_folder_preview_for_source(&folder, 16, appearance, &other)
            .is_none());
    }
}

#[test]
fn folder_preview_obsolete_jobs_cannot_repopulate_invalidated_variants() {
    let dir = tempfile::tempdir().unwrap();
    let cache = ThumbnailDiskCache::new(dir.path().to_path_buf()).unwrap();
    let folder = dir.path().join("folder");
    std::fs::create_dir(&folder).unwrap();
    let source = FolderPreviewSource::capture(&folder, None).unwrap();
    let mut requests = FolderPreviewRequests::default();
    let old_job = requests.begin(&folder);
    requests.invalidate(&folder);
    cache.remove_folder_preview_cache(&folder);
    let pixels = [230, 210, 140, 255].repeat(16 * 16);
    for appearance in FolderAppearance::ALL {
        cache.put_folder_preview_checked(
            &folder, 16, &pixels, 16, 16, appearance, &source, &old_job,
        );
        assert!(cache
            .get_folder_preview_for_source(&folder, 16, appearance, &source)
            .is_none());
    }
    let new_job = requests.begin(&folder);
    cache.put_folder_preview_checked(
        &folder,
        16,
        &pixels,
        16,
        16,
        FolderAppearance::Original,
        &source,
        &new_job,
    );
    assert!(cache
        .get_folder_preview_for_source(&folder, 16, FolderAppearance::Original, &source)
        .is_some());
}

#[test]
fn folder_preview_source_changes_during_composition_prevent_commit() {
    let dir = tempfile::tempdir().unwrap();
    let cache = ThumbnailDiskCache::new(dir.path().to_path_buf()).unwrap();
    let folder = dir.path().join("folder");
    std::fs::create_dir(&folder).unwrap();
    let cover = folder.join("cover.jpg");
    std::fs::write(&cover, b"image").unwrap();
    set_cover_time(&cover, 100_100);
    let old = FolderPreviewSource::capture(&folder, Some(&cover)).unwrap();
    set_cover_time(&cover, 100_200);
    let current = FolderPreviewSource::capture(&folder, Some(&cover)).unwrap();
    let pixels = [230, 210, 140, 255].repeat(16 * 16);
    cache.put_folder_preview_checked(
        &folder,
        16,
        &pixels,
        16,
        16,
        FolderAppearance::Original,
        &old,
        &FolderPreviewValidity::default(),
    );
    assert!(cache
        .get_folder_preview_for_source(&folder, 16, FolderAppearance::Original, &current)
        .is_none());
}

#[test]
fn folder_preview_invalidation_while_writer_is_busy_prevents_late_commit() {
    let dir = tempfile::tempdir().unwrap();
    let cache = std::sync::Arc::new(ThumbnailDiskCache::new(dir.path().to_path_buf()).unwrap());
    let folder = dir.path().join("folder");
    std::fs::create_dir(&folder).unwrap();
    let source = FolderPreviewSource::capture(&folder, None).unwrap();
    let mut requests = FolderPreviewRequests::default();
    let old = requests.begin(&folder);
    let lock = cache.writer.lock();
    let worker_cache = cache.clone();
    let worker_folder = folder.clone();
    let worker_source = source.clone();
    let worker = std::thread::spawn(move || {
        let pixels = [230, 210, 140, 255].repeat(16 * 16);
        worker_cache.put_folder_preview_checked(
            &worker_folder,
            16,
            &pixels,
            16,
            16,
            FolderAppearance::Original,
            &worker_source,
            &old,
        );
    });
    requests.invalidate(&folder);
    drop(lock);
    worker.join().unwrap();
    assert!(cache
        .get_folder_preview_for_source(&folder, 16, FolderAppearance::Original, &source)
        .is_none());
}
