//! Cover resolution, validation and metadata writes belong off the UI thread.
use crate::infrastructure::app_state_db::AppStateDb;
use crate::infrastructure::windows::{find_folder_preview_item, is_mpeg_ts_file};
use eframe::egui;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};

const MAX_BATCH: usize = 32;

pub(in crate::app) fn spawn_cover_worker(
    db: Arc<AppStateDb>,
    ctx: egui::Context,
) -> (
    mpsc::Sender<PathBuf>,
    mpsc::Receiver<(PathBuf, Option<PathBuf>)>,
) {
    let (requests, rx) = mpsc::channel::<PathBuf>();
    let (results, received) = mpsc::channel();
    if std::thread::Builder::new()
        .name("folder-covers".into())
        .spawn(move || {
            crate::infrastructure::io_priority::set_thread_priority(
                crate::infrastructure::io_priority::IOPriority::Background,
            );
            while let Ok(first) = rx.recv() {
                let batch = collect_batch(first, &rx);
                // Waiting for the reader is safe here. Treating a busy cache as a
                // miss would cause unnecessary filesystem scans under contention.
                let cached = db.get_folder_covers(&batch);
                for folder in batch {
                    let cover = resolve_cover(&db, &folder, cached.get(&folder));
                    if results.send((folder, cover)).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            }
        })
        .is_err()
    {
        log::error!("[FOLDER-COVERS] Failed to start worker");
    }
    (requests, received)
}

fn collect_batch(first: PathBuf, rx: &mpsc::Receiver<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    std::iter::once(first)
        .chain(rx.try_iter().take(MAX_BATCH - 1))
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

fn valid_media(path: &Path) -> bool {
    !path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("ts"))
        || is_mpeg_ts_file(path)
}

fn resolve_cover(db: &AppStateDb, folder: &Path, cached: Option<&PathBuf>) -> Option<PathBuf> {
    if let Some(cover) = cached {
        if crate::infrastructure::onedrive::fast_path_exists(cover) && valid_media(cover) {
            return Some(cover.clone());
        }
    }
    let cover = find_folder_preview_item(folder).filter(|path| valid_media(path));
    if let Some(cover) = &cover {
        db.set_folder_cover(folder, cover);
    } else if cached.is_some() {
        db.remove_folder_cover(folder);
    }
    cover
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn batch_is_bounded_deduplicated_and_preserves_order() {
        let (tx, rx) = mpsc::channel();
        tx.send(PathBuf::from("folder-a")).unwrap();
        for id in 0..100 {
            tx.send(PathBuf::from(format!("folder-{id}"))).unwrap();
        }
        let batch = collect_batch(PathBuf::from("folder-a"), &rx);
        assert_eq!(batch[0], PathBuf::from("folder-a"));
        assert_eq!(batch.len(), MAX_BATCH - 1);
        assert!(rx.try_recv().is_ok());
    }
    #[test]
    fn cached_typescript_is_rejected_but_real_transport_stream_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let cover = dir.path().join("file.ts");
        std::fs::write(&cover, b"const value = 1;").unwrap();
        let db = AppStateDb::new_in_memory().unwrap();
        db.set_folder_cover(dir.path(), &cover);
        assert!(resolve_cover(&db, dir.path(), Some(&cover)).is_none());
        assert!(db.get_folder_covers(&[dir.path().to_path_buf()]).is_empty());
        std::fs::write(&cover, [0x47, 0, 0]).unwrap();
        assert_eq!(resolve_cover(&db, dir.path(), Some(&cover)), Some(cover));
    }
}
