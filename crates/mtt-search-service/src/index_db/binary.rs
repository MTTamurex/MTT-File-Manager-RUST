//! Binary index format for fast save/load of the in-memory VolumeIndex.
//!
//! Layout:
//!   [Header]                     — 88 bytes
//!   [NameArena]                  — arena_size bytes
//!   [Records]                    — record_count × 32 bytes (8-byte FRN + 24-byte FileRecord)
//!   [Hardlinks]                  — hardlink_entry_count × 16 bytes (8-byte child FRN + 8-byte parent FRN)
//!   [Reparse Points]             — reparse_count × 8 bytes (8-byte FRN)
//!   [CRC32]                      — 4 bytes (covers everything above)

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::file_index::{FileRecord, VolumeIndex};

const MAGIC: &[u8; 8] = b"MTTIDX01";
const VERSION: u32 = 1;

#[repr(C, packed)]
struct Header {
    magic: [u8; 8],
    version: u32,
    drive_letter: u8,
    _pad: [u8; 3],
    journal_id: u64,
    last_usn: i64,
    record_count: u64,
    arena_size: u64,
    hardlink_entry_count: u64,
    reparse_count: u64,
    /// Bit 0: hardlink_data_complete, Bit 1: reparse_data_complete, Bit 2: sizes_complete
    flags: u64,
}

const HEADER_SIZE: usize = std::mem::size_of::<Header>();
const _: () = assert!(HEADER_SIZE == 72);

/// Returns the path for the binary index file for a given drive letter.
pub fn index_path(drive_letter: char) -> PathBuf {
    let data_dir = std::env::var("PROGRAMDATA")
        .unwrap_or_else(|_| r"C:\ProgramData".to_string());
    Path::new(&data_dir)
        .join("MTT-File-Manager")
        .join(format!("index_{}.bin", drive_letter))
}

/// Save a VolumeIndex to a binary file atomically (write temp + rename).
pub fn save(index: &VolumeIndex) -> Result<(), String> {
    let path = index_path(index.drive_letter);
    let tmp_path = path.with_extension("bin.tmp");

    // Ensure directory exists.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create index dir: {}", e))?;
    }

    let start = std::time::Instant::now();

    let mut file = std::fs::File::create(&tmp_path)
        .map_err(|e| format!("Failed to create temp index file: {}", e))?;

    let mut crc = Crc32::new();

    // Flatten hardlink_parents into (child_frn, parent_frn) pairs.
    let hardlink_pairs: Vec<(u64, u64)> = index
        .hardlink_parents
        .iter()
        .flat_map(|(&child, parents)| parents.iter().map(move |&p| (child, p)))
        .collect();

    // Build header.
    let mut flags: u64 = 0;
    if index.hardlink_data_complete {
        flags |= 1;
    }
    if index.reparse_data_complete {
        flags |= 2;
    }
    if index.sizes_loaded {
        flags |= 4;
    }

    let header = Header {
        magic: *MAGIC,
        version: VERSION,
        drive_letter: index.drive_letter as u8,
        _pad: [0; 3],
        journal_id: index.journal_id,
        last_usn: index.last_usn,
        record_count: index.records.len() as u64,
        arena_size: index.names.len() as u64,
        hardlink_entry_count: hardlink_pairs.len() as u64,
        reparse_count: index.reparse_points.len() as u64,
        flags,
    };

    // Write header.
    let header_bytes: &[u8] =
        unsafe { std::slice::from_raw_parts(&header as *const Header as *const u8, HEADER_SIZE) };
    file.write_all(header_bytes)
        .map_err(|e| format!("Write header: {}", e))?;
    crc.update(header_bytes);

    // Write NameArena.
    let arena_bytes = index.names.as_bytes();
    file.write_all(arena_bytes)
        .map_err(|e| format!("Write arena: {}", e))?;
    crc.update(arena_bytes);

    // Write Records (sorted by FRN for deterministic output).
    let mut sorted_frns: Vec<u64> = index.records.keys().copied().collect();
    sorted_frns.sort_unstable();

    // Write in a buffer to reduce syscalls.
    const RECORD_SIZE: usize = 8 + 24; // FRN + FileRecord
    let mut buf = Vec::with_capacity(RECORD_SIZE * 8192.min(sorted_frns.len()));
    for (i, &frn) in sorted_frns.iter().enumerate() {
        let rec = &index.records[&frn];
        buf.extend_from_slice(&frn.to_le_bytes());
        let rec_bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(rec as *const FileRecord as *const u8, 24)
        };
        buf.extend_from_slice(rec_bytes);

        if buf.len() >= RECORD_SIZE * 8192 || i == sorted_frns.len() - 1 {
            file.write_all(&buf)
                .map_err(|e| format!("Write records: {}", e))?;
            crc.update(&buf);
            buf.clear();
        }
    }

    // Write Hardlink pairs.
    for &(child, parent) in &hardlink_pairs {
        buf.extend_from_slice(&child.to_le_bytes());
        buf.extend_from_slice(&parent.to_le_bytes());
        if buf.len() >= 16 * 8192 {
            file.write_all(&buf)
                .map_err(|e| format!("Write hardlinks: {}", e))?;
            crc.update(&buf);
            buf.clear();
        }
    }
    if !buf.is_empty() {
        file.write_all(&buf)
            .map_err(|e| format!("Write hardlinks tail: {}", e))?;
        crc.update(&buf);
        buf.clear();
    }

    // Write Reparse points.
    let mut sorted_reparse: Vec<u64> = index.reparse_points.iter().copied().collect();
    sorted_reparse.sort_unstable();
    for &frn in &sorted_reparse {
        buf.extend_from_slice(&frn.to_le_bytes());
        if buf.len() >= 8 * 8192 {
            file.write_all(&buf)
                .map_err(|e| format!("Write reparse: {}", e))?;
            crc.update(&buf);
            buf.clear();
        }
    }
    if !buf.is_empty() {
        file.write_all(&buf)
            .map_err(|e| format!("Write reparse tail: {}", e))?;
        crc.update(&buf);
        buf.clear();
    }

    // Write CRC32.
    let checksum = crc.finalize();
    file.write_all(&checksum.to_le_bytes())
        .map_err(|e| format!("Write checksum: {}", e))?;

    file.flush()
        .map_err(|e| format!("Flush: {}", e))?;
    drop(file);

    // Atomic rename.
    std::fs::rename(&tmp_path, &path)
        .map_err(|e| format!("Rename temp index: {}", e))?;

    let elapsed = start.elapsed();
    eprintln!(
        "[BINARY-IDX] {}:\\ Saved {} records + {} arena bytes in {:.3}s",
        index.drive_letter,
        index.records.len(),
        index.names.len(),
        elapsed.as_secs_f64()
    );
    Ok(())
}

/// Load a VolumeIndex from binary file. Returns None if file doesn't exist.
/// Returns Err on corruption (CRC mismatch, truncation, bad magic).
pub fn load(drive_letter: char) -> Result<Option<(VolumeIndex, PersistedBinaryState)>, String> {
    let path = index_path(drive_letter);
    if !path.exists() {
        return Ok(None);
    }

    let start = std::time::Instant::now();

    let data = std::fs::read(&path)
        .map_err(|e| format!("Read binary index: {}", e))?;

    if data.len() < HEADER_SIZE + 4 {
        return Err("Binary index too small".into());
    }

    // Verify CRC over everything except the last 4 bytes.
    let payload = &data[..data.len() - 4];
    let stored_crc = u32::from_le_bytes([
        data[data.len() - 4],
        data[data.len() - 3],
        data[data.len() - 2],
        data[data.len() - 1],
    ]);
    let mut crc = Crc32::new();
    crc.update(payload);
    let computed_crc = crc.finalize();
    if stored_crc != computed_crc {
        // Delete corrupted file so next startup does a full scan.
        let _ = std::fs::remove_file(&path);
        return Err(format!(
            "CRC mismatch: stored={:#010x} computed={:#010x}",
            stored_crc, computed_crc
        ));
    }

    // Parse header.
    let header: Header = unsafe {
        std::ptr::read_unaligned(data.as_ptr() as *const Header)
    };
    if &header.magic != MAGIC {
        return Err("Bad magic".into());
    }
    // Copy packed fields to aligned locals to avoid UB from unaligned references.
    let h_version = header.version;
    let h_drive_letter = header.drive_letter;
    let h_journal_id = header.journal_id;
    let h_last_usn = header.last_usn;
    let h_record_count = header.record_count;
    let h_arena_size = header.arena_size;
    let h_hardlink_count = header.hardlink_entry_count;
    let h_reparse_count = header.reparse_count;
    let h_flags = header.flags;

    if h_version != VERSION {
        return Err(format!("Unsupported version: {}", h_version));
    }
    if h_drive_letter as char != drive_letter {
        return Err(format!(
            "Drive letter mismatch: expected {}, got {}",
            drive_letter, h_drive_letter as char
        ));
    }

    let record_count = h_record_count as usize;
    let arena_size = h_arena_size as usize;
    let hardlink_count = h_hardlink_count as usize;
    let reparse_count = h_reparse_count as usize;

    // Validate expected size.
    let expected = HEADER_SIZE
        + arena_size
        + record_count * 32
        + hardlink_count * 16
        + reparse_count * 8
        + 4; // CRC
    if data.len() != expected {
        return Err(format!(
            "Size mismatch: expected {} got {}",
            expected,
            data.len()
        ));
    }

    let mut offset = HEADER_SIZE;

    // Load NameArena.
    let mut names = crate::name_arena::NameArena::with_capacity(arena_size);
    // Reconstruct by bulk-inserting the raw arena bytes.
    // NameArena::insert() validates UTF-8 per-entry, but we already CRC-verified
    // the whole file. Use a direct reconstruction approach instead.
    let arena_slice = &data[offset..offset + arena_size];
    offset += arena_size;

    // Load Records.
    let mut records = std::collections::HashMap::with_capacity(record_count);
    for _ in 0..record_count {
        let frn = u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap());
        offset += 8;
        let rec: FileRecord = unsafe {
            std::ptr::read_unaligned(data[offset..].as_ptr() as *const FileRecord)
        };
        offset += 24;
        records.insert(frn, rec);
    }

    // Load Hardlinks.
    let mut hardlink_parents: std::collections::HashMap<u64, Vec<u64>> =
        std::collections::HashMap::new();
    for _ in 0..hardlink_count {
        let child = u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap());
        offset += 8;
        let parent = u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap());
        offset += 8;
        hardlink_parents.entry(child).or_default().push(parent);
    }

    // Load Reparse points.
    let mut reparse_points = std::collections::HashSet::with_capacity(reparse_count);
    for _ in 0..reparse_count {
        let frn = u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap());
        offset += 8;
        reparse_points.insert(frn);
    }

    // Build VolumeIndex.
    let mut index = VolumeIndex::new(drive_letter);
    // Replace the default arena with one containing our loaded data.
    index.names = crate::name_arena::NameArena::from_raw(arena_slice);
    index.records = records;
    index.hardlink_parents = hardlink_parents;
    index.reparse_points = reparse_points;
    index.journal_id = h_journal_id;
    index.last_usn = h_last_usn;
    index.hardlink_data_complete = h_flags & 1 != 0;
    index.reparse_data_complete = h_flags & 2 != 0;
    index.sizes_loaded = h_flags & 4 != 0;

    // Rebuild the children reverse index from loaded records + hardlinks.
    index.rebuild_children();

    let state = PersistedBinaryState {
        journal_id: h_journal_id,
        last_usn: h_last_usn,
        files_indexed: h_record_count,
        has_hardlink_parent_data: h_flags & 1 != 0,
        has_reparse_point_data: h_flags & 2 != 0,
    };

    let elapsed = start.elapsed();
    eprintln!(
        "[BINARY-IDX] {}:\\ Loaded {} records + {} arena bytes in {:.3}s",
        drive_letter, record_count, arena_size, elapsed.as_secs_f64()
    );

    Ok(Some((index, state)))
}

/// Delete the binary index file for a given drive letter.
pub fn delete(drive_letter: char) {
    let path = index_path(drive_letter);
    let _ = std::fs::remove_file(path);
}

/// Metadata from a loaded binary index (mirrors PersistedVolumeState).
pub struct PersistedBinaryState {
    pub journal_id: u64,
    pub last_usn: i64,
    pub files_indexed: u64,
    pub has_hardlink_parent_data: bool,
    pub has_reparse_point_data: bool,
}

// ──────────────────────── Minimal CRC32 (IEEE) ─────────────────────────

struct Crc32 {
    state: u32,
}

impl Crc32 {
    fn new() -> Self {
        Self { state: 0xFFFF_FFFF }
    }

    fn update(&mut self, data: &[u8]) {
        for &byte in data {
            let index = ((self.state ^ byte as u32) & 0xFF) as usize;
            self.state = CRC32_TABLE[index] ^ (self.state >> 8);
        }
    }

    fn finalize(self) -> u32 {
        self.state ^ 0xFFFF_FFFF
    }
}

const CRC32_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0u32;
    while i < 256 {
        let mut crc = i;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = 0xEDB8_8320 ^ (crc >> 1);
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i as usize] = crc;
        i += 1;
    }
    table
};
