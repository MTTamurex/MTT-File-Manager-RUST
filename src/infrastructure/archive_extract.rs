//! Native archive extraction fallback.
//!
//! When Windows Shell `IFileOperation` fails to copy files from inside archives
//! (ZIP, 7z, RAR, TAR variants), this module extracts them directly using
//! native crates. This bypasses Shell namespace limitations (encoding issues,
//! invalid names, path length).

use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::io::{self, BufWriter};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::domain::file_entry::{ends_with_ignore_case, split_archive_path};

// ---------------------------------------------------------------------------
// Entry matching (files + folder prefixes)
// ---------------------------------------------------------------------------

/// How an archive entry matched a requested path.
enum MatchKind {
    /// No match.
    None,
    /// Direct file match → extract flat (just the filename).
    ExactFile,
    /// Entry is inside a requested folder → preserve relative path starting at `rel_start` bytes.
    FolderChild { rel_start: usize },
}

/// Builds lookup sets from the requested internal paths and provides an O(1) / O(n_prefixes)
/// match check for every archive entry.
struct EntryMatcher {
    full_paths: HashSet<String>,
    bare_names: HashSet<String>,
    /// (lowered prefix with trailing "/", byte offset where the folder name begins)
    folder_prefixes: Vec<(String, usize)>,
}

impl EntryMatcher {
    fn new(internal_paths: &[&str]) -> Self {
        let mut full_paths = HashSet::with_capacity(internal_paths.len());
        let mut bare_names = HashSet::with_capacity(internal_paths.len());
        let mut folder_prefixes = Vec::with_capacity(internal_paths.len());

        for &p in internal_paths {
            let normalized = p.replace('\\', "/").to_ascii_lowercase();
            let bare = normalized
                .rsplit('/')
                .next()
                .unwrap_or(&normalized)
                .to_string();
            bare_names.insert(bare);
            full_paths.insert(normalized.clone());
            // Every requested path might be a folder; add as prefix candidate.
            let prefix = format!("{}/", normalized);
            let rel_start = normalized.rfind('/').map(|i| i + 1).unwrap_or(0);
            folder_prefixes.push((prefix, rel_start));
        }

        Self {
            full_paths,
            bare_names,
            folder_prefixes,
        }
    }

    /// Check if a lowered archive entry path matches any requested path.
    fn match_entry(&self, entry_path_lower: &str) -> MatchKind {
        // 1) Exact full-path match → flat extraction.
        if self.full_paths.contains(entry_path_lower) {
            return MatchKind::ExactFile;
        }

        // 2) Bare filename match → flat extraction.
        let bare = entry_path_lower
            .rsplit('/')
            .next()
            .unwrap_or(entry_path_lower);
        if self.bare_names.contains(bare) {
            return MatchKind::ExactFile;
        }

        // 3) Folder prefix match → preserve relative directory structure.
        for (prefix, rel_start) in &self.folder_prefixes {
            if entry_path_lower.starts_with(prefix.as_str()) {
                return MatchKind::FolderChild {
                    rel_start: *rel_start,
                };
            }
        }

        MatchKind::None
    }
}

/// Derives the destination file path for an archive entry based on match kind.
fn derive_output_path(
    entry_name: &str,
    match_kind: &MatchKind,
    dest_folder: &Path,
) -> io::Result<PathBuf> {
    match match_kind {
        MatchKind::ExactFile => {
            let file_name = Path::new(entry_name)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| entry_name.to_string());
            Ok(dest_folder.join(sanitize_filename(&file_name)))
        }
        MatchKind::FolderChild { rel_start } => {
            let relative = &entry_name[*rel_start..];
            let dest_path = dest_folder.join(sanitize_relative_path(relative));
            if let Some(parent) = dest_path.parent() {
                fs::create_dir_all(parent)?;
            }
            Ok(dest_path)
        }
        MatchKind::None => unreachable!(),
    }
}

/// Sanitizes each component of a relative path (preserving directory structure).
fn sanitize_relative_path(rel_path: &str) -> PathBuf {
    let components = rel_path.replace('\\', "/");
    let mut result = PathBuf::new();
    for component in components.split('/') {
        if !component.is_empty() {
            result.push(sanitize_filename(component));
        }
    }
    result
}

/// Shared progress state for archive extraction, read by the UI thread.
#[derive(Clone, Debug)]
pub struct ExtractionProgress {
    pub archive_name: String,
    pub current_file: String,
    pub extracted: usize,
    /// Known total of files to extract. `0` means total is unknown (e.g. TAR).
    pub total: usize,
}

/// Thread-safe handle for extraction progress. `None` means no extraction in progress.
pub type SharedExtractionProgress = Arc<Mutex<Option<ExtractionProgress>>>;

/// Creates a new shared progress handle (initialized to `None`).
pub fn new_shared_progress() -> SharedExtractionProgress {
    Arc::new(Mutex::new(None))
}

/// Returns true if ALL the given virtual paths point to archive formats
/// that can be extracted natively (ZIP, 7z, RAR, TAR variants) without relying on Windows Shell.
pub fn has_native_support(paths: &[PathBuf]) -> bool {
    paths.iter().all(|p| {
        match split_archive_path(p) {
            Some((archive, _)) => is_natively_supported(&archive),
            None => false,
        }
    })
}

/// Checks whether a given archive file has a natively-supported format.
fn is_natively_supported(archive_path: &Path) -> bool {
    let name = archive_path.to_string_lossy();
    // Order matters: compound extensions (.tar.gz) must be checked before simple ones (.gz).
    ends_with_ignore_case(&name, ".zip")
        || ends_with_ignore_case(&name, ".7z")
        || ends_with_ignore_case(&name, ".rar")
        || ends_with_ignore_case(&name, ".tar.gz")
        || ends_with_ignore_case(&name, ".tgz")
        || ends_with_ignore_case(&name, ".tar.bz2")
        || ends_with_ignore_case(&name, ".tbz2")
        || ends_with_ignore_case(&name, ".tar.xz")
        || ends_with_ignore_case(&name, ".txz")
        || ends_with_ignore_case(&name, ".tar.zst")
        || ends_with_ignore_case(&name, ".tzst")
        || ends_with_ignore_case(&name, ".tar")
}

/// Attempts to extract files from archives to `dest_folder`.
///
/// `paths` are virtual paths like `C:\dl\archive.zip\folder\VR.nfo`.
/// Each path is split into (archive_file, internal_path), grouped by archive,
/// then extracted using the appropriate native crate.
///
/// Returns `true` if all files were extracted successfully.
pub fn extract_files_from_archive(
    paths: &[PathBuf],
    dest_folder: &Path,
    progress: &SharedExtractionProgress,
) -> bool {
    // Group paths by their parent archive file.
    let mut groups: HashMap<PathBuf, Vec<String>> = HashMap::new();
    let mut non_archive_paths = Vec::new();

    for path in paths {
        match split_archive_path(path) {
            Some((archive, internal)) => {
                groups.entry(archive).or_default().push(internal);
            }
            None => {
                non_archive_paths.push(path.clone());
                log::warn!(
                    "[ArchiveExtract] Path is not inside an archive, skipping: {}",
                    path.display()
                );
            }
        }
    }

    if groups.is_empty() {
        return false;
    }

    // Initialize progress (total=0 means unknown; each format handler will set it via pre-scan).
    let first_archive = groups.keys().next().unwrap();
    let archive_display = first_archive
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if let Ok(mut p) = progress.lock() {
        *p = Some(ExtractionProgress {
            archive_name: archive_display,
            current_file: String::new(),
            extracted: 0,
            total: 0,
        });
    }

    let mut global_extracted = 0usize;
    let mut all_ok = true;
    for (archive_path, internal_paths) in &groups {
        let archive_display = archive_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        // Update archive name when switching archives
        if let Ok(mut p) = progress.lock() {
            if let Some(ref mut state) = *p {
                state.archive_name = archive_display;
            }
        }

        let refs: Vec<&str> = internal_paths.iter().map(|s| s.as_str()).collect();
        let result =
            extract_from_archive(archive_path, &refs, dest_folder, progress, &mut global_extracted);
        match result {
            Ok(count) => {
                log::info!(
                    "[ArchiveExtract] Extracted {}/{} files from {}",
                    count,
                    refs.len(),
                    archive_path.display()
                );
                if count == 0 {
                    all_ok = false;
                }
            }
            Err(e) => {
                log::error!(
                    "[ArchiveExtract] Failed to extract from {}: {}",
                    archive_path.display(),
                    e
                );
                all_ok = false;
            }
        }
    }

    // Clear progress when done
    if let Ok(mut p) = progress.lock() {
        *p = None;
    }

    all_ok
}

/// Dispatches extraction to the format-specific handler. Returns the number of files extracted.
fn extract_from_archive(
    archive_path: &Path,
    internal_paths: &[&str],
    dest_folder: &Path,
    progress: &SharedExtractionProgress,
    global_extracted: &mut usize,
) -> io::Result<usize> {
    let name = archive_path.to_string_lossy();

    if ends_with_ignore_case(&name, ".zip") {
        extract_from_zip(archive_path, internal_paths, dest_folder, progress, global_extracted)
    } else if ends_with_ignore_case(&name, ".7z") {
        extract_from_7z(archive_path, internal_paths, dest_folder, progress, global_extracted)
    } else if ends_with_ignore_case(&name, ".rar") {
        extract_from_rar(archive_path, internal_paths, dest_folder, progress, global_extracted)
    } else if is_tar_variant(&name) {
        extract_from_tar(archive_path, internal_paths, dest_folder, progress, global_extracted)
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "Native extraction not supported for archive format: {}",
                name
            ),
        ))
    }
}

/// Returns true if the archive name is a TAR variant (.tar, .tar.gz, .tgz, etc.).
fn is_tar_variant(name: &str) -> bool {
    ends_with_ignore_case(name, ".tar")
        || ends_with_ignore_case(name, ".tar.gz")
        || ends_with_ignore_case(name, ".tgz")
        || ends_with_ignore_case(name, ".tar.bz2")
        || ends_with_ignore_case(name, ".tbz2")
        || ends_with_ignore_case(name, ".tar.xz")
        || ends_with_ignore_case(name, ".txz")
        || ends_with_ignore_case(name, ".tar.zst")
        || ends_with_ignore_case(name, ".tzst")
}

/// Helper to update shared progress state.
fn update_progress(
    progress: &SharedExtractionProgress,
    current_file: &str,
    extracted: usize,
) {
    if let Ok(mut p) = progress.lock() {
        if let Some(ref mut state) = *p {
            state.current_file = current_file.to_string();
            state.extracted = extracted;
        }
    }
}

/// Sets the known total for progress (called after pre-scan).
fn set_progress_total(progress: &SharedExtractionProgress, total: usize) {
    if let Ok(mut p) = progress.lock() {
        if let Some(ref mut state) = *p {
            state.total += total;
        }
    }
}

/// Clears progress so the UI toast disappears immediately.
fn clear_progress(progress: &SharedExtractionProgress) {
    if let Ok(mut p) = progress.lock() {
        *p = None;
    }
}

/// Extracts specific files from a ZIP archive.
fn extract_from_zip(
    archive_path: &Path,
    internal_paths: &[&str],
    dest_folder: &Path,
    progress: &SharedExtractionProgress,
    global_extracted: &mut usize,
) -> io::Result<usize> {
    let file = fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

    let matcher = EntryMatcher::new(internal_paths);

    // Pre-scan: count matching entries (cheap — reads central directory only).
    let mut total_matching = 0usize;
    for i in 0..archive.len() {
        if let Ok(entry) = archive.by_index(i) {
            if !entry.is_dir() {
                let name = entry.name().replace('\\', "/");
                let lower = name.to_ascii_lowercase();
                if !matches!(matcher.match_entry(&lower), MatchKind::None) {
                    total_matching += 1;
                }
            }
        }
    }
    set_progress_total(progress, total_matching);

    // Extract matching entries.
    let mut extracted = 0;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

        if entry.is_dir() {
            continue;
        }

        let entry_name = entry.name().replace('\\', "/");
        let entry_lower = entry_name.to_ascii_lowercase();

        let match_kind = matcher.match_entry(&entry_lower);
        if matches!(match_kind, MatchKind::None) {
            continue;
        }

        let dest_path = derive_output_path(&entry_name, &match_kind, dest_folder)?;
        let sanitized = dest_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        let mut out_file = BufWriter::new(fs::File::create(&dest_path)?);
        io::copy(&mut entry, &mut out_file)?;
        extracted += 1;
        *global_extracted += 1;
        update_progress(progress, &sanitized, *global_extracted);

        log::debug!(
            "[ArchiveExtract/ZIP] Extracted '{}' → {}",
            entry_name,
            dest_path.display()
        );
    }

    Ok(extracted)
}

/// Extracts specific files from a 7z archive.
fn extract_from_7z(
    archive_path: &Path,
    internal_paths: &[&str],
    dest_folder: &Path,
    progress: &SharedExtractionProgress,
    global_extracted: &mut usize,
) -> io::Result<usize> {
    let matcher = EntryMatcher::new(internal_paths);

    // Pre-scan: read archive metadata (no decompression) to count matching entries.
    let total_matching = match sevenz_rust::Archive::open(archive_path) {
        Ok(archive) => {
            let count = archive.files.iter().filter(|entry| {
                if entry.is_directory() {
                    return false;
                }
                let name = entry.name().replace('\\', "/");
                let lower = name.to_ascii_lowercase();
                !matches!(matcher.match_entry(&lower), MatchKind::None)
            }).count();
            set_progress_total(progress, count);
            count
        }
        Err(e) => {
            log::warn!("[ArchiveExtract/7z] Pre-scan failed, total unknown: {}", e);
            0
        }
    };

    let mut extracted = 0usize;

    sevenz_rust::decompress_file_with_extract_fn(archive_path, dest_folder, |entry, reader, _| {
        if entry.is_directory() {
            return Ok(true);
        }

        let entry_name = entry.name().replace('\\', "/");
        let entry_lower = entry_name.to_ascii_lowercase();

        let match_kind = matcher.match_entry(&entry_lower);
        if matches!(match_kind, MatchKind::None) {
            return Ok(true);
        }

        let dest_path = derive_output_path(&entry_name, &match_kind, dest_folder)
            .map_err(|e| sevenz_rust::Error::other(format!("Path error: {}", e)))?;
        let sanitized = dest_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        let mut out_file = BufWriter::new(fs::File::create(&dest_path)
            .map_err(|e| sevenz_rust::Error::other(format!("Failed to create {}: {}", dest_path.display(), e)))?);
        io::copy(reader, &mut out_file)
            .map_err(|e| sevenz_rust::Error::other(format!("Failed to write {}: {}", dest_path.display(), e)))?;
        extracted += 1;
        *global_extracted += 1;
        update_progress(progress, &sanitized, *global_extracted);

        log::debug!(
            "[ArchiveExtract/7z] Extracted '{}' → {}",
            entry_name,
            dest_path.display()
        );

        // Clear progress immediately after last matching file is extracted,
        // so the toast disappears right away instead of lingering during
        // 7z solid-block post-scan of remaining entries.
        if total_matching > 0 && extracted >= total_matching {
            clear_progress(progress);
        }

        Ok(true)
    })
    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

    Ok(extracted)
}

/// Extracts specific files from a RAR archive.
fn extract_from_rar(
    archive_path: &Path,
    internal_paths: &[&str],
    dest_folder: &Path,
    progress: &SharedExtractionProgress,
    global_extracted: &mut usize,
) -> io::Result<usize> {
    let matcher = EntryMatcher::new(internal_paths);

    // Pre-scan: skip-through to count matching entries (no decompression).
    {
        let mut scan = unrar::Archive::new(archive_path)
            .open_for_processing()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        let mut total_matching = 0usize;
        loop {
            let header = scan
                .read_header()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
            let Some(header) = header else { break };
            let entry = header.entry();
            if !entry.is_directory() {
                let name = entry.filename.to_string_lossy().replace('\\', "/");
                let lower = name.to_ascii_lowercase();
                if !matches!(matcher.match_entry(&lower), MatchKind::None) {
                    total_matching += 1;
                }
            }
            scan = header
                .skip()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        }
        set_progress_total(progress, total_matching);
    }

    // Extract matching entries.
    let mut extracted = 0usize;
    let mut archive = unrar::Archive::new(archive_path)
        .open_for_processing()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

    loop {
        let header = archive
            .read_header()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

        let Some(header) = header else { break };

        let entry = header.entry();
        let entry_name = entry.filename.to_string_lossy().replace('\\', "/");
        let entry_lower = entry_name.to_ascii_lowercase();

        let match_kind = if entry.is_directory() {
            MatchKind::None
        } else {
            matcher.match_entry(&entry_lower)
        };

        if matches!(match_kind, MatchKind::None) {
            archive = header
                .skip()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
            continue;
        }

        // Read file content into memory, then write with derived output path.
        let (data, next) = header
            .read()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        archive = next;

        let dest_path = derive_output_path(&entry_name, &match_kind, dest_folder)?;
        let sanitized = dest_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        fs::write(&dest_path, &data)?;
        extracted += 1;
        *global_extracted += 1;
        update_progress(progress, &sanitized, *global_extracted);

        log::debug!(
            "[ArchiveExtract/RAR] Extracted '{}' → {}",
            entry_name,
            dest_path.display()
        );
    }

    Ok(extracted)
}

/// Extracts specific files from a TAR archive (plain or compressed).
/// Supports: .tar, .tar.gz/.tgz, .tar.bz2/.tbz2, .tar.xz/.txz, .tar.zst/.tzst
fn extract_from_tar(
    archive_path: &Path,
    internal_paths: &[&str],
    dest_folder: &Path,
    progress: &SharedExtractionProgress,
    global_extracted: &mut usize,
) -> io::Result<usize> {
    let file = fs::File::open(archive_path)?;
    let name_lower = archive_path.to_string_lossy().to_ascii_lowercase();

    // Chain the appropriate decompressor based on file extension.
    let reader: Box<dyn io::Read> = if name_lower.ends_with(".tar.gz") || name_lower.ends_with(".tgz") {
        Box::new(flate2::read::GzDecoder::new(file))
    } else if name_lower.ends_with(".tar.bz2") || name_lower.ends_with(".tbz2") {
        Box::new(bzip2::read::BzDecoder::new(file))
    } else if name_lower.ends_with(".tar.xz") || name_lower.ends_with(".txz") {
        Box::new(xz2::read::XzDecoder::new(file))
    } else if name_lower.ends_with(".tar.zst") || name_lower.ends_with(".tzst") {
        Box::new(
            zstd::stream::read::Decoder::new(file)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?,
        )
    } else {
        // Plain .tar
        Box::new(file)
    };

    let matcher = EntryMatcher::new(internal_paths);
    let mut archive = tar::Archive::new(reader);
    let mut extracted = 0usize;

    for entry_result in archive.entries()? {
        let mut entry = entry_result?;

        if entry.header().entry_type().is_dir() {
            continue;
        }

        let entry_path = entry.path()?.to_string_lossy().replace('\\', "/");
        let entry_lower = entry_path.to_ascii_lowercase();

        let match_kind = matcher.match_entry(&entry_lower);
        if matches!(match_kind, MatchKind::None) {
            continue;
        }

        let dest_path = derive_output_path(&entry_path, &match_kind, dest_folder)?;
        let sanitized = dest_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        let mut out_file = BufWriter::new(fs::File::create(&dest_path)?);
        io::copy(&mut entry, &mut out_file)?;
        extracted += 1;
        *global_extracted += 1;
        update_progress(progress, &sanitized, *global_extracted);

        log::debug!(
            "[ArchiveExtract/TAR] Extracted '{}' → {}",
            entry_path,
            dest_path.display()
        );
    }

    Ok(extracted)
}

/// Sanitizes a filename by replacing characters that are invalid on Windows.
fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            _ => c,
        })
        .collect();

    // Remove trailing dots and spaces (invalid on Windows).
    let trimmed = cleaned.trim_end_matches(|c| c == '.' || c == ' ');
    if trimmed.is_empty() {
        "_extracted".to_string()
    } else {
        trimmed.to_string()
    }
}
