//! Native archive creation (ZIP / 7Z).
//!
//! Writes archives with entry-count progress and cooperative cancellation.
//! Archives are written to a sibling `.part` file first and renamed on
//! success, so a cancelled or failed job never leaves a truncated archive
//! that looks valid.

mod seven_zip;
mod zip_archive;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Supported archive formats for creation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompressionFormat {
    Zip,
    SevenZip,
}

impl CompressionFormat {
    /// Canonical lowercase extension (without leading dot).
    pub fn extension(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::SevenZip => "7z",
        }
    }

    /// Parses a context-menu command suffix ("zip" / "7z").
    pub fn from_command(value: &str) -> Option<Self> {
        match value {
            "zip" => Some(Self::Zip),
            "7z" => Some(Self::SevenZip),
            _ => None,
        }
    }
}

/// Shared progress state for archive creation, read by the UI thread.
#[derive(Clone, Debug)]
pub struct CompressionProgress {
    pub archive_name: String,
    pub current_file: String,
    pub processed: usize,
    /// Total number of file entries to compress.
    pub total: usize,
    /// Bytes of source data already consumed by the encoder. Because both
    /// writers are single-threaded and read inline, this is real-time
    /// compression progress, not a read-ahead artifact.
    pub processed_bytes: u64,
    /// Total bytes of source data to compress (sum of file sizes).
    pub total_bytes: u64,
}

/// Thread-safe handle for compression progress. `None` means no job running.
pub type SharedCompressionProgress = Arc<Mutex<Option<CompressionProgress>>>;

/// Thread-safe cancellation flag. Set to `true` by the UI to request abort.
pub type CompressionCancelFlag = Arc<AtomicBool>;

/// Creates a new shared progress handle (initialized to `None`).
pub fn new_shared_progress() -> SharedCompressionProgress {
    Arc::new(Mutex::new(None))
}

/// Creates a new cancellation flag (initialized to `false`).
pub fn new_cancel_flag() -> CompressionCancelFlag {
    Arc::new(AtomicBool::new(false))
}

/// A single input item collected for archiving.
pub(crate) struct PendingEntry {
    /// Absolute source path.
    pub source: PathBuf,
    /// Entry name inside the archive (forward slashes, no leading slash).
    pub name: String,
    /// Directory entries are stored without content.
    pub is_dir: bool,
    /// Source size in bytes (0 for directories).
    pub size: u64,
}

pub(crate) fn is_cancelled(cancel: &CompressionCancelFlag) -> bool {
    cancel.load(Ordering::Relaxed)
}

pub(crate) fn cancelled_error() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "compression cancelled")
}

pub(crate) fn update_progress(
    progress: &SharedCompressionProgress,
    archive_name: &str,
    current_file: &str,
    processed: usize,
    total: usize,
    processed_bytes: u64,
    total_bytes: u64,
) {
    if let Ok(mut guard) = progress.lock() {
        *guard = Some(CompressionProgress {
            archive_name: archive_name.to_string(),
            current_file: current_file.to_string(),
            processed,
            total,
            processed_bytes,
            total_bytes,
        });
    }
}

pub(crate) fn clear_progress(progress: &SharedCompressionProgress) {
    if let Ok(mut guard) = progress.lock() {
        *guard = None;
    }
}

/// Reader wrapper that publishes byte-level progress on every chunk consumed
/// by the encoder, and aborts mid-file once cancellation is requested.
/// Both writers read inline through this wrapper (single-threaded encoder, or
/// the multithreaded encoder's feeder), so bytes read track compression work.
///
/// NOTE: multithreaded 7z jobs can read ahead of the encoder (the crate's
/// work queue is unbounded), so on the MT path the byte count may reach the
/// total before the final chunks finish encoding.
pub(crate) struct ProgressReader<R> {
    inner: R,
    progress: SharedCompressionProgress,
    cancel: CompressionCancelFlag,
    /// Entries completed before the file being read.
    processed: usize,
    /// Bytes of entries completed before the file being read.
    base_bytes: u64,
    bytes_read: u64,
}

impl<R> ProgressReader<R> {
    pub(crate) fn new(
        inner: R,
        processed: usize,
        base_bytes: u64,
        progress: SharedCompressionProgress,
        cancel: CompressionCancelFlag,
    ) -> Self {
        Self {
            inner,
            progress,
            cancel,
            processed,
            base_bytes,
            bytes_read: 0,
        }
    }

    fn publish_bytes(&self) {
        if let Ok(mut guard) = self.progress.lock() {
            // Only advance an existing snapshot: entry transitions (which own
            // the string fields) always run `update_progress` first.
            if let Some(state) = guard.as_mut() {
                state.processed = self.processed;
                state.processed_bytes = self.base_bytes + self.bytes_read;
            }
        }
    }
}

impl<R: io::Read> io::Read for ProgressReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if is_cancelled(&self.cancel) {
            return Err(cancelled_error());
        }
        let read = self.inner.read(buf)?;
        if read > 0 {
            self.bytes_read += read as u64;
            self.publish_bytes();
        }
        Ok(read)
    }
}

fn entry_name_for(root_name: &str, rel: &Path) -> String {
    let rel = rel.to_string_lossy().replace('\\', "/");
    if rel.is_empty() {
        root_name.to_string()
    } else {
        format!("{root_name}/{rel}")
    }
}

/// Collects every file and directory under `sources`, naming each entry after
/// its top-level item (e.g. selecting `Docs` produces `Docs/a.txt`).
fn collect_entries(
    sources: &[PathBuf],
    cancel: &CompressionCancelFlag,
) -> io::Result<Vec<PendingEntry>> {
    let mut entries = Vec::new();
    for source in sources {
        if is_cancelled(cancel) {
            return Err(cancelled_error());
        }
        let Some(root_name) = source
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        if source.is_dir() {
            for item in walkdir::WalkDir::new(source)
                .follow_links(false)
                .into_iter()
                .filter_map(|entry| entry.ok())
            {
                if is_cancelled(cancel) {
                    return Err(cancelled_error());
                }
                let rel = item.path().strip_prefix(source).unwrap_or(item.path());
                let name = entry_name_for(&root_name, rel);
                let is_dir = item.file_type().is_dir();
                let size = item.metadata().map(|meta| meta.len()).unwrap_or(0);
                entries.push(PendingEntry {
                    source: item.into_path(),
                    name,
                    is_dir,
                    size,
                });
            }
        } else {
            let size = fs::metadata(source).map(|meta| meta.len()).unwrap_or(0);
            entries.push(PendingEntry {
                source: source.clone(),
                name: root_name,
                is_dir: false,
                size,
            });
        }
    }
    Ok(entries)
}

/// Creates an archive at `dest` containing `sources`.
///
/// The archive is built at `dest.part` and renamed into place on success;
/// on failure or cancellation the partial file is removed.
pub fn create_archive(
    sources: &[PathBuf],
    dest: &Path,
    format: CompressionFormat,
    progress: &SharedCompressionProgress,
    cancel: &CompressionCancelFlag,
) -> io::Result<()> {
    let archive_name = dest
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let entries = collect_entries(sources, cancel)?;
    let total = entries.iter().filter(|entry| !entry.is_dir).count();
    let total_bytes = entries
        .iter()
        .filter(|entry| !entry.is_dir)
        .map(|entry| entry.size)
        .sum();
    update_progress(progress, &archive_name, "", 0, total, 0, total_bytes);

    let partial = part_path(dest);
    let result = match format {
        CompressionFormat::Zip => zip_archive::write_archive(
            &entries, &partial, &archive_name, total, total_bytes, progress, cancel,
        ),
        CompressionFormat::SevenZip => seven_zip::write_archive(
            &entries, &partial, &archive_name, total, total_bytes, progress, cancel,
        ),
    };

    match result {
        Ok(()) => {
            fs::rename(&partial, dest)?;
            clear_progress(progress);
            Ok(())
        }
        Err(error) => {
            let _ = fs::remove_file(&partial);
            clear_progress(progress);
            Err(error)
        }
    }
}

fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.as_os_str().to_os_string();
    name.push(".part");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn write(path: &Path, contents: &[u8]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    fn make_tree(root: &Path) -> Vec<PathBuf> {
        write(&root.join("a.txt"), b"alpha");
        write(&root.join("sub/b.txt"), b"beta");
        write(&root.join("sub/inner/c.txt"), b"gamma");
        vec![root.join("a.txt"), root.join("sub")]
    }

    #[test]
    fn format_extension_and_command_parsing_round_trip() {
        assert_eq!(CompressionFormat::Zip.extension(), "zip");
        assert_eq!(CompressionFormat::SevenZip.extension(), "7z");
        assert_eq!(
            CompressionFormat::from_command("zip"),
            Some(CompressionFormat::Zip)
        );
        assert_eq!(
            CompressionFormat::from_command("7z"),
            Some(CompressionFormat::SevenZip)
        );
        assert_eq!(CompressionFormat::from_command("rar"), None);
    }

    #[test]
    fn progress_reader_publishes_incremental_bytes() {
        let progress = new_shared_progress();
        update_progress(&progress, "a.zip", "one.bin", 0, 2, 0, 2048);

        let mut reader = ProgressReader::new(
            std::io::Cursor::new(vec![0u8; 1536]),
            0,
            0,
            progress.clone(),
            new_cancel_flag(),
        );

        let mut buffer = [0u8; 512];
        reader.read_exact(&mut buffer).unwrap();
        let snapshot = progress.lock().unwrap().clone().unwrap();
        assert_eq!(snapshot.processed_bytes, 512);
        assert_eq!(snapshot.archive_name, "a.zip");

        reader.read_exact(&mut buffer).unwrap();
        reader.read_exact(&mut buffer).unwrap();
        let snapshot = progress.lock().unwrap().clone().unwrap();
        assert_eq!(snapshot.processed_bytes, 1536);
        assert_eq!(snapshot.processed, 0);
        assert_eq!(snapshot.total_bytes, 2048);
    }

    #[test]
    fn progress_reader_accumulates_bytes_across_files_via_base() {
        let progress = new_shared_progress();
        update_progress(&progress, "a.zip", "two.bin", 1, 2, 1024, 2048);

        let mut reader = ProgressReader::new(
            std::io::Cursor::new(vec![0u8; 256]),
            1,
            1024,
            progress.clone(),
            new_cancel_flag(),
        );

        let mut buffer = [0u8; 128];
        reader.read_exact(&mut buffer).unwrap();
        assert_eq!(
            progress.lock().unwrap().as_ref().unwrap().processed_bytes,
            1024 + 128
        );
    }

    #[test]
    fn progress_reader_aborts_mid_file_when_cancelled() {
        let progress = new_shared_progress();
        update_progress(&progress, "a.zip", "one.bin", 0, 1, 0, 1024);

        let cancel = new_cancel_flag();
        let mut reader = ProgressReader::new(
            std::io::Cursor::new(vec![0u8; 1024]),
            0,
            0,
            progress.clone(),
            cancel.clone(),
        );

        cancel.store(true, Ordering::Relaxed);
        let error = reader.read(&mut [0u8; 64]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    }

    #[test]
    fn cancelled_mid_file_zip_reports_interrupted_and_removes_partial() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("big.bin");
        // Pseudo-random (incompressible) data: the ST deflate loop paces
        // reads, so the cancel lands mid-read. Chunk waits stay cheap because
        // tests run zip with a small copy buffer (see zip_archive.rs).
        let mut lcg = 0x2545F4914F6CDD1Du64;
        let mut data = vec![0u8; 128 * 1024];
        for byte in &mut data {
            lcg = lcg.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            *byte = (lcg >> 33) as u8;
        }
        write(&source, &data);
        let dest = temp.path().join("out.zip");
        let cancel = new_cancel_flag();

        // Simulates the UI cancel click landing after the job started reading:
        // wait for the first byte-level publish, then cancel mid-file.
        let progress = new_shared_progress();
        let progress_for_thread = progress.clone();
        let cancel_for_thread = cancel.clone();
        std::thread::spawn(move || {
            for _ in 0..10_000 {
                let reading = progress_for_thread
                    .lock()
                    .map(|g| g.as_ref().is_some_and(|p| p.processed_bytes > 0))
                    .unwrap_or(false);
                if reading {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            cancel_for_thread.store(true, Ordering::Relaxed);
        });

        let error = create_archive(
            &[source],
            &dest,
            CompressionFormat::Zip,
            &progress,
            &cancel,
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert!(!dest.exists());
        assert!(!temp.path().join("out.zip.part").exists());
    }

    #[test]
    fn cancelled_mid_file_sevenz_reports_interrupted_and_removes_partial() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("big.bin");
        // Pseudo-random (incompressible) data. The test hook forces the
        // single-threaded encoder, whose inline reads are paced by the (slow,
        // unoptimized) debug encoder, so cancellation lands mid-read. Kept
        // tiny so a worst-case missed-cancel tail stays cheap in debug.
        crate::infrastructure::archive_create::seven_zip::set_test_force_single_thread(true);
        let mut lcg = 0x2545F4914F6CDD1Du64;
        let mut data = vec![0u8; 64 * 1024];
        for byte in &mut data {
            lcg = lcg
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            *byte = (lcg >> 33) as u8;
        }
        write(&source, &data);
        let dest = temp.path().join("out.7z");
        let cancel = new_cancel_flag();

        // Simulates the UI cancel click landing after the job started reading:
        // wait for the first byte-level publish, then cancel mid-file.
        let progress = new_shared_progress();
        let progress_for_thread = progress.clone();
        let cancel_for_thread = cancel.clone();
        std::thread::spawn(move || {
            for _ in 0..10_000 {
                let reading = progress_for_thread
                    .lock()
                    .map(|g| g.as_ref().is_some_and(|p| p.processed_bytes > 0))
                    .unwrap_or(false);
                if reading {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            cancel_for_thread.store(true, Ordering::Relaxed);
        });

        let error = create_archive(
            &[source],
            &dest,
            CompressionFormat::SevenZip,
            &progress,
            &cancel,
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert!(!dest.exists());
        assert!(!temp.path().join("out.7z.part").exists());
    }

    #[test]
    fn collect_entries_names_after_top_level_items() {
        let temp = tempfile::tempdir().unwrap();
        let sources = make_tree(temp.path());
        let entries =
            collect_entries(&sources, &new_cancel_flag()).expect("collect should succeed");

        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"a.txt"));
        assert!(names.contains(&"sub"));
        assert!(names.contains(&"sub/b.txt"));
        assert!(names.contains(&"sub/inner/c.txt"));

        let sub_dir = entries.iter().find(|e| e.name == "sub").unwrap();
        assert!(sub_dir.is_dir);
    }

    #[test]
    fn create_zip_archive_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let sources = make_tree(temp.path());
        let dest = temp.path().join("out.zip");

        create_archive(
            &sources,
            &dest,
            CompressionFormat::Zip,
            &new_shared_progress(),
            &new_cancel_flag(),
        )
        .expect("zip creation should succeed");

        assert!(dest.exists());
        assert!(!temp.path().join("out.zip.part").exists());

        let file = fs::File::open(&dest).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut names: Vec<String> = archive.file_names().map(str::to_string).collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "a.txt".to_string(),
                "sub/".to_string(),
                "sub/b.txt".to_string(),
                "sub/inner/".to_string(),
                "sub/inner/c.txt".to_string()
            ]
        );

        let mut contents = String::new();
        archive.by_name("sub/inner/c.txt").unwrap().read_to_string(&mut contents).unwrap();
        assert_eq!(contents, "gamma");
    }

    #[test]
    fn create_seven_zip_archive_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let sources = make_tree(temp.path());
        let dest = temp.path().join("out.7z");

        create_archive(
            &sources,
            &dest,
            CompressionFormat::SevenZip,
            &new_shared_progress(),
            &new_cancel_flag(),
        )
        .expect("7z creation should succeed");

        assert!(dest.exists());
        assert!(!temp.path().join("out.7z.part").exists());

        let extracted_root = temp.path().join("extracted");
        sevenz_rust::decompress_file(&dest, &extracted_root)
            .expect("7z archive should decompress");

        assert_eq!(
            std::fs::read(extracted_root.join("a.txt")).unwrap(),
            b"alpha"
        );
        assert_eq!(
            std::fs::read(extracted_root.join("sub/b.txt")).unwrap(),
            b"beta"
        );
        assert_eq!(
            std::fs::read(extracted_root.join("sub/inner/c.txt")).unwrap(),
            b"gamma"
        );
    }

    #[test]
    fn cancelled_job_leaves_no_partial_archive() {
        let temp = tempfile::tempdir().unwrap();
        let sources = make_tree(temp.path());
        let dest = temp.path().join("out.zip");
        let cancel = new_cancel_flag();
        cancel.store(true, Ordering::Relaxed);

        let error = create_archive(
            &sources,
            &dest,
            CompressionFormat::Zip,
            &new_shared_progress(),
            &cancel,
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert!(!dest.exists());
        assert!(!temp.path().join("out.zip.part").exists());
    }

    #[test]
    fn failed_job_clears_progress() {
        let temp = tempfile::tempdir().unwrap();
        let sources = vec![temp.path().join("does-not-exist.txt")];
        let dest = temp.path().join("out.zip");
        let progress = new_shared_progress();

        // A missing source file is collected as a plain entry (metadata-free)
        // and fails when the writer tries to open it.
        let _ = create_archive(
            &sources,
            &dest,
            CompressionFormat::Zip,
            &progress,
            &new_cancel_flag(),
        );

        assert!(progress.lock().unwrap().is_none());
        assert!(!temp.path().join("out.zip.part").exists());
    }

    #[test]
    fn empty_folder_selection_produces_archive_with_dir_entry() {
        let temp = tempfile::tempdir().unwrap();
        let empty = temp.path().join("Empty");
        std::fs::create_dir_all(&empty).unwrap();
        let dest = temp.path().join("empty.zip");

        create_archive(
            &[empty],
            &dest,
            CompressionFormat::Zip,
            &new_shared_progress(),
            &new_cancel_flag(),
        )
        .expect("empty folder should archive");

        let file = fs::File::open(&dest).unwrap();
        let archive = zip::ZipArchive::new(file).unwrap();
        assert_eq!(archive.file_names().collect::<Vec<_>>(), vec!["Empty/"]);
    }
}
