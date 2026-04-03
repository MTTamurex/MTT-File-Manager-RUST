//! Native archive extraction fallback.
//!
//! When Windows Shell `IFileOperation` fails to copy files from inside archives
//! (ZIP, 7z), this module extracts them directly using pure-Rust crates.
//! This bypasses Shell namespace limitations (encoding issues, invalid names, path length).

use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::io::{self, BufWriter};
use std::path::{Path, PathBuf};

use crate::domain::file_entry::{ends_with_ignore_case, split_archive_path};

/// Returns true if ALL the given virtual paths point to archive formats
/// that can be extracted natively (ZIP, 7z) without relying on Windows Shell.
pub fn has_native_support(paths: &[PathBuf]) -> bool {
    paths.iter().all(|p| {
        match split_archive_path(p) {
            Some((archive, _)) => {
                let name = archive.to_string_lossy();
                ends_with_ignore_case(&name, ".zip") || ends_with_ignore_case(&name, ".7z")
            }
            None => false,
        }
    })
}

/// Attempts to extract files from archives to `dest_folder`.
///
/// `paths` are virtual paths like `C:\dl\archive.zip\folder\VR.nfo`.
/// Each path is split into (archive_file, internal_path), grouped by archive,
/// then extracted using the appropriate native crate.
///
/// Returns `true` if all files were extracted successfully.
pub fn extract_files_from_archive(paths: &[PathBuf], dest_folder: &Path) -> bool {
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

    let mut all_ok = true;
    for (archive_path, internal_paths) in &groups {
        let refs: Vec<&str> = internal_paths.iter().map(|s| s.as_str()).collect();
        let result = extract_from_archive(archive_path, &refs, dest_folder);
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

    all_ok
}

/// Dispatches extraction to the format-specific handler. Returns the number of files extracted.
fn extract_from_archive(
    archive_path: &Path,
    internal_paths: &[&str],
    dest_folder: &Path,
) -> io::Result<usize> {
    let name = archive_path.to_string_lossy();

    if ends_with_ignore_case(&name, ".zip") {
        extract_from_zip(archive_path, internal_paths, dest_folder)
    } else if ends_with_ignore_case(&name, ".7z") {
        extract_from_7z(archive_path, internal_paths, dest_folder)
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

/// Extracts specific files from a ZIP archive.
fn extract_from_zip(
    archive_path: &Path,
    internal_paths: &[&str],
    dest_folder: &Path,
) -> io::Result<usize> {
    let file = fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

    let mut extracted = 0;

    for &requested in internal_paths {
        // Normalize separators: Windows uses `\`, ZIP uses `/`.
        let normalized = requested.replace('\\', "/");

        // Try exact match first, then case-insensitive scan.
        let index = archive.index_for_name(&normalized).or_else(|| {
            let lower = normalized.to_ascii_lowercase();
            archive
                .file_names()
                .enumerate()
                .find(|(_, name)| name.to_ascii_lowercase() == lower)
                .map(|(i, _)| i)
        });

        let Some(idx) = index else {
            log::warn!(
                "[ArchiveExtract/ZIP] Entry not found in {}: '{}'",
                archive_path.display(),
                requested
            );
            continue;
        };

        let mut entry = archive
            .by_index(idx)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

        if entry.is_dir() {
            continue;
        }

        // Derive output file name (just the file name component, flat extraction).
        let entry_name = entry.name().to_string();
        let file_name = Path::new(&entry_name)
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new(&entry_name));
        let sanitized = sanitize_filename(&file_name.to_string_lossy());
        let dest_path = dest_folder.join(&sanitized);

        let mut out_file = BufWriter::new(fs::File::create(&dest_path)?);
        io::copy(&mut entry, &mut out_file)?;
        extracted += 1;

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
) -> io::Result<usize> {
    let total_requested = internal_paths.len();

    // Build lookup sets for O(1) matching: full normalized paths + bare filenames.
    let mut full_paths: HashSet<String> = HashSet::with_capacity(total_requested);
    let mut bare_names: HashSet<String> = HashSet::with_capacity(total_requested);
    for &p in internal_paths {
        let normalized = p.replace('\\', "/").to_ascii_lowercase();
        if let Some(name) = normalized.rsplit('/').next() {
            bare_names.insert(name.to_string());
        }
        bare_names.insert(normalized.rsplit('/').next().unwrap_or(&normalized).to_string());
        full_paths.insert(normalized);
    }

    let mut extracted = 0usize;

    sevenz_rust::decompress_file_with_extract_fn(archive_path, dest_folder, |entry, reader, _| {
        if entry.is_directory() {
            return Ok(true);
        }

        let entry_name = entry.name().replace('\\', "/");
        let entry_lower = entry_name.to_ascii_lowercase();

        // O(1) check: full path match or bare filename match.
        let is_requested = full_paths.contains(&entry_lower) || {
            let bare = entry_lower.rsplit('/').next().unwrap_or(&entry_lower);
            bare_names.contains(bare)
        };

        if !is_requested {
            return Ok(true);
        }

        let file_name = Path::new(&entry_name)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| entry_name.clone());
        let sanitized = sanitize_filename(&file_name);
        let dest_path = dest_folder.join(&sanitized);

        let mut out_file = BufWriter::new(fs::File::create(&dest_path)
            .map_err(|e| sevenz_rust::Error::other(format!("Failed to create {}: {}", dest_path.display(), e)))?);
        io::copy(reader, &mut out_file)
            .map_err(|e| sevenz_rust::Error::other(format!("Failed to write {}: {}", dest_path.display(), e)))?;
        extracted += 1;

        log::debug!(
            "[ArchiveExtract/7z] Extracted '{}' → {}",
            entry_name,
            dest_path.display()
        );

        Ok(true)
    })
    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

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
