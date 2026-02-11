use crate::file_index::VolumeIndex;

/// The NTFS root directory's File Reference Number.
const NTFS_ROOT_FRN: u64 = 5;

/// Maximum depth for path resolution (safety limit).
const MAX_DEPTH: usize = 256;

/// Reconstruct full path from a file reference number by walking the parent chain.
/// Returns None if the path cannot be fully resolved (orphan record).
pub fn resolve_path(frn: u64, index: &VolumeIndex) -> Option<String> {
    let mut components = Vec::with_capacity(16);
    let mut current = frn;

    for _ in 0..MAX_DEPTH {
        let record = index.records.get(&current)?;

        // Skip adding the root directory name (usually "." or empty)
        if current == NTFS_ROOT_FRN {
            break;
        }

        components.push(record.name.as_str());

        let parent = record.parent_ref;

        // Reached root or self-referencing
        if parent == current || parent == 0 || parent == NTFS_ROOT_FRN {
            break;
        }

        current = parent;
    }

    if components.is_empty() {
        return None;
    }

    components.reverse();

    // Build path: "C:\component1\component2\filename"
    Some(format!("{}:\\{}", index.drive_letter, components.join("\\")))
}
