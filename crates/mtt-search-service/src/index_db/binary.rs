//! Binary index format for fast save/load of the in-memory VolumeIndex.
//!
//! Layout (`MTTIDX02`):
//!   [Header]                     — 72 bytes
//!   [NameArena]                  — arena_size bytes
//!   [Records]                    — record_count × 32 bytes (8-byte FRN + 24-byte FileRecord)
//!   [Hardlinks]                  — hardlink_entry_count × 16 bytes (8-byte child FRN + 8-byte parent FRN)
//!   [Reparse Points]             — reparse_count × 8 bytes (8-byte FRN)
//!   [HMAC-SHA256]                — 32 bytes (covers everything above; key is per-machine, DPAPI-sealed)
//!
//! SEC: The trailer was upgraded from CRC32 → HMAC-SHA256 with a per-machine
//! key sealed by DPAPI. CRC32 only protects against accidental corruption; an
//! attacker who can write to the index file can trivially recompute it. HMAC
//! requires the per-machine key (see [`super::integrity`]). Files written by
//! the legacy `MTTIDX01` format are treated as missing on load and rebuilt.

use std::io::{BufReader, BufWriter, Read, Write};
use std::path::PathBuf;

use memmap2::MmapOptions;

use super::integrity::{self, HMAC_OUTPUT_SIZE};
use crate::file_index::{FileRecord, VolumeIndex};
use crate::record_store::RecordStore;

const MAGIC: &[u8; 8] = b"MTTIDX02";
const LEGACY_MAGIC: &[u8; 8] = b"MTTIDX01";
const VERSION: u32 = 2;
const TRAILER_SIZE: usize = HMAC_OUTPUT_SIZE;
const MMAP_ARENA_MIN_BYTES: usize = 64 * 1024 * 1024;
const HMAC_STREAM_BUF_SIZE: usize = 64 * 1024;

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
const FRN_SIZE: usize = std::mem::size_of::<u64>();
const FILE_RECORD_SIZE: usize = std::mem::size_of::<FileRecord>();
const RECORD_SIZE: usize = FRN_SIZE + FILE_RECORD_SIZE;
const HARDLINK_ENTRY_SIZE: usize = FRN_SIZE * 2;
const REPARSE_ENTRY_SIZE: usize = FRN_SIZE;
const _: () = {
    assert!(HEADER_SIZE == 72);
    assert!(std::mem::offset_of!(Header, magic) == 0);
    assert!(std::mem::offset_of!(Header, version) == 8);
    assert!(std::mem::offset_of!(Header, drive_letter) == 12);
    assert!(std::mem::offset_of!(Header, _pad) == 13);
    assert!(std::mem::offset_of!(Header, journal_id) == 16);
    assert!(std::mem::offset_of!(Header, last_usn) == 24);
    assert!(std::mem::offset_of!(Header, record_count) == 32);
    assert!(std::mem::offset_of!(Header, arena_size) == 40);
    assert!(std::mem::offset_of!(Header, hardlink_entry_count) == 48);
    assert!(std::mem::offset_of!(Header, reparse_count) == 56);
    assert!(std::mem::offset_of!(Header, flags) == 64);

    assert!(FILE_RECORD_SIZE == 24);
    assert!(std::mem::offset_of!(FileRecord, parent_ref) == 0);
    assert!(std::mem::offset_of!(FileRecord, size) == 8);
    assert!(std::mem::offset_of!(FileRecord, name_offset) == 16);
    assert!(std::mem::offset_of!(FileRecord, name_len) == 20);
    assert!(std::mem::offset_of!(FileRecord, is_dir) == 22);
    assert!(std::mem::offset_of!(FileRecord, _pad) == 23);
};

fn read_u16_le(bytes: &[u8], offset: usize, label: &str) -> Result<u16, String> {
    let raw: [u8; 2] = bytes
        .get(offset..offset + 2)
        .and_then(|slice| slice.try_into().ok())
        .ok_or_else(|| format!("Corrupt binary index: short {}", label))?;
    Ok(u16::from_le_bytes(raw))
}

fn read_u32_le(bytes: &[u8], offset: usize, label: &str) -> Result<u32, String> {
    let raw: [u8; 4] = bytes
        .get(offset..offset + 4)
        .and_then(|slice| slice.try_into().ok())
        .ok_or_else(|| format!("Corrupt binary index: short {}", label))?;
    Ok(u32::from_le_bytes(raw))
}

fn read_u64_le(bytes: &[u8], offset: usize, label: &str) -> Result<u64, String> {
    let raw: [u8; 8] = bytes
        .get(offset..offset + 8)
        .and_then(|slice| slice.try_into().ok())
        .ok_or_else(|| format!("Corrupt binary index: short {}", label))?;
    Ok(u64::from_le_bytes(raw))
}

fn decode_file_record(bytes: &[u8]) -> Result<FileRecord, String> {
    if bytes.len() != FILE_RECORD_SIZE {
        return Err(format!(
            "Corrupt binary index: record payload size {} != {}",
            bytes.len(),
            FILE_RECORD_SIZE
        ));
    }

    let is_dir = match bytes[22] {
        0 => false,
        1 => true,
        other => {
            return Err(format!(
                "Corrupt binary index: invalid FileRecord.is_dir byte {}",
                other
            ));
        }
    };

    Ok(FileRecord {
        parent_ref: read_u64_le(bytes, 0, "record parent_ref")?,
        size: read_u64_le(bytes, 8, "record size")?,
        name_offset: read_u32_le(bytes, 16, "record name_offset")?,
        name_len: read_u16_le(bytes, 20, "record name_len")?,
        is_dir,
        _pad: bytes[23],
    })
}

/// Returns the path for the binary index file for a given drive letter.
/// Uses the shared data directory set at startup by `get_db_path`,
/// so binary and SQLite caches always live together.
pub fn index_path(drive_letter: char) -> PathBuf {
    super::data_dir().join(format!("index_{}.bin", drive_letter))
}

fn write_authenticated_chunk<W: Write>(
    writer: &mut W,
    hmac: &mut integrity::HmacSha256,
    bytes: &[u8],
) -> Result<(), String> {
    writer
        .write_all(bytes)
        .map_err(|e| format!("Write payload: {}", e))?;
    hmac.update(bytes)
        .map_err(|e| format!("HMAC update: {}", e))
}

fn read_authenticated_chunk<R: Read>(
    reader: &mut R,
    hmac: &mut integrity::HmacSha256,
    buf: &mut [u8],
    label: &str,
) -> Result<(), String> {
    reader
        .read_exact(buf)
        .map_err(|e| format!("Read {}: {}", label, e))?;
    hmac.update(buf).map_err(|e| format!("HMAC update: {}", e))
}

fn read_authenticated_bytes<R: Read>(
    reader: &mut R,
    hmac: &mut integrity::HmacSha256,
    mut len: usize,
    label: &str,
) -> Result<(), String> {
    let mut buf = vec![0u8; HMAC_STREAM_BUF_SIZE.min(len.max(1))];
    while len > 0 {
        let chunk_len = len.min(buf.len());
        let chunk = &mut buf[..chunk_len];
        reader
            .read_exact(chunk)
            .map_err(|e| format!("Read {}: {}", label, e))?;
        hmac.update(chunk)
            .map_err(|e| format!("HMAC update: {}", e))?;
        len -= chunk_len;
    }
    Ok(())
}

fn mmap_arena_enabled(arena_size: usize) -> bool {
    if arena_size < MMAP_ARENA_MIN_BYTES {
        return false;
    }

    match std::env::var("MTT_SEARCH_MMAP_ARENA") {
        Ok(value) => !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
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

    // SEC: Resolve the per-machine HMAC key BEFORE opening the temp file so a
    // missing/unreadable key never produces a half-written index on disk.
    let key = integrity::machine_key().map_err(|e| format!("HMAC key unavailable: {}", e))?;

    let file = std::fs::File::create(&tmp_path)
        .map_err(|e| format!("Failed to create temp index file: {}", e))?;
    let mut writer = BufWriter::new(file);
    let mut hmac = integrity::HmacSha256::new(&key).map_err(|e| format!("HMAC init: {}", e))?;

    let hardlink_entry_count: usize = index.hardlink_parents.values().map(Vec::len).sum();

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
        hardlink_entry_count: hardlink_entry_count as u64,
        reparse_count: index.reparse_points.len() as u64,
        flags,
    };

    // Write header.
    let header_bytes: &[u8] =
        unsafe { std::slice::from_raw_parts(&header as *const Header as *const u8, HEADER_SIZE) };
    write_authenticated_chunk(&mut writer, &mut hmac, header_bytes)?;

    // Write NameArena.
    index
        .names
        .try_for_each_slice(|slice| write_authenticated_chunk(&mut writer, &mut hmac, slice))?;

    // Write Records (sorted by FRN for deterministic output).
    let mut sorted_frns: Vec<u64> = index.records.keys().copied().collect();
    sorted_frns.sort_unstable();
    for &frn in &sorted_frns {
        let rec = index
            .records
            .get(&frn)
            .ok_or_else(|| format!("Record FRN {} disappeared during binary save", frn))?;
        write_authenticated_chunk(&mut writer, &mut hmac, &frn.to_le_bytes())?;
        let rec_bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(rec as *const FileRecord as *const u8, FILE_RECORD_SIZE)
        };
        write_authenticated_chunk(&mut writer, &mut hmac, rec_bytes)?;
    }

    // Write Hardlink pairs.
    for (&child, parents) in &index.hardlink_parents {
        for &parent in parents {
            write_authenticated_chunk(&mut writer, &mut hmac, &child.to_le_bytes())?;
            write_authenticated_chunk(&mut writer, &mut hmac, &parent.to_le_bytes())?;
        }
    }

    // Write Reparse points.
    let mut sorted_reparse: Vec<u64> = index.reparse_points.iter().copied().collect();
    sorted_reparse.sort_unstable();
    for &frn in &sorted_reparse {
        write_authenticated_chunk(&mut writer, &mut hmac, &frn.to_le_bytes())?;
    }

    // SEC: Compute HMAC-SHA256 incrementally over the serialized payload.
    let tag = hmac
        .finalize()
        .map_err(|e| format!("HMAC compute: {}", e))?;

    writer
        .write_all(&tag)
        .map_err(|e| format!("Write HMAC trailer: {}", e))?;
    writer.flush().map_err(|e| format!("Flush: {}", e))?;
    let file = writer
        .into_inner()
        .map_err(|e| format!("Finish buffered write: {}", e))?;
    file.sync_all().map_err(|e| format!("Sync: {}", e))?;
    drop(file);

    // Atomic rename.
    std::fs::rename(&tmp_path, &path).map_err(|e| format!("Rename temp index: {}", e))?;

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

    let file = std::fs::File::open(&path).map_err(|e| format!("Read binary index: {}", e))?;
    let file_len = file
        .metadata()
        .map_err(|e| format!("Read binary metadata: {}", e))?
        .len() as usize;
    let mut reader = BufReader::new(&file);

    if file_len < HEADER_SIZE + TRAILER_SIZE {
        return Err("Binary index too small".into());
    }

    let mut header_bytes = [0u8; HEADER_SIZE];
    reader
        .read_exact(&mut header_bytes)
        .map_err(|e| format!("Read header: {}", e))?;

    // SEC: Magic check FIRST so we can recognize legacy files and silently
    // request a rebuild instead of erroring out.
    if header_bytes[..8] == *LEGACY_MAGIC {
        eprintln!(
            "[BINARY-IDX] {}:\\ Legacy MTTIDX01 (CRC32) format detected; \
             discarding and rebuilding under MTTIDX02 (HMAC-SHA256).",
            drive_letter
        );
        let _ = std::fs::remove_file(&path);
        return Ok(None);
    }
    if header_bytes[..8] != *MAGIC {
        let _ = std::fs::remove_file(&path);
        return Err("Bad magic".into());
    }

    // Parse header.
    let header: Header =
        unsafe { std::ptr::read_unaligned(header_bytes.as_ptr() as *const Header) };
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

    // SEC: Sanity caps to prevent OOM via huge HashMap pre-allocation and to
    // make the size-equality check below meaningful even on attacker-crafted
    // headers. NTFS supports up to ~2^48 file records per volume but no
    // realistic deployment exceeds 100M files; arenas above 2 GB would
    // already be rejected by the u32 NameRef offset domain.
    const MAX_RECORDS: usize = 100_000_000;
    const MAX_ARENA_BYTES: usize = u32::MAX as usize; // 4 GB hard cap from NameRef
    const MAX_HARDLINK_PAIRS: usize = 200_000_000;
    const MAX_REPARSE: usize = 10_000_000;
    if record_count > MAX_RECORDS {
        return Err(format!("record_count too large: {}", record_count));
    }
    if arena_size > MAX_ARENA_BYTES {
        return Err(format!("arena_size too large: {}", arena_size));
    }
    if hardlink_count > MAX_HARDLINK_PAIRS {
        return Err(format!(
            "hardlink_entry_count too large: {}",
            hardlink_count
        ));
    }
    if reparse_count > MAX_REPARSE {
        return Err(format!("reparse_count too large: {}", reparse_count));
    }

    // SEC: Use checked arithmetic so a crafted header cannot wrap `expected`
    // back to `data.len()` and bypass the size validation. Any overflow at
    // this stage means the header is hostile or corrupt.
    let expected = HEADER_SIZE
        .checked_add(arena_size)
        .and_then(|s| s.checked_add(record_count.checked_mul(RECORD_SIZE)?))
        .and_then(|s| s.checked_add(hardlink_count.checked_mul(HARDLINK_ENTRY_SIZE)?))
        .and_then(|s| s.checked_add(reparse_count.checked_mul(REPARSE_ENTRY_SIZE)?))
        .and_then(|s| s.checked_add(TRAILER_SIZE)) // HMAC-SHA256
        .ok_or_else(|| "Size overflow in header arithmetic".to_string())?;
    if file_len != expected {
        return Err(format!(
            "Size mismatch: expected {} got {}",
            expected, file_len
        ));
    }

    let key = integrity::machine_key().map_err(|e| format!("HMAC key unavailable: {}", e))?;
    let mut hmac = integrity::HmacSha256::new(&key).map_err(|e| format!("HMAC init: {}", e))?;
    hmac.update(&header_bytes)
        .map_err(|e| format!("HMAC update: {}", e))?;

    // Authenticate NameArena. Large arenas are streamed through HMAC and then
    // mapped read-only, avoiding a private Vec allocation while preserving the
    // same whole-file integrity check.
    let arena_bytes = if mmap_arena_enabled(arena_size) {
        read_authenticated_bytes(&mut reader, &mut hmac, arena_size, "name arena")?;
        None
    } else {
        let mut arena_bytes = vec![0u8; arena_size];
        read_authenticated_chunk(&mut reader, &mut hmac, &mut arena_bytes, "name arena")?;
        Some(arena_bytes)
    };

    // Load Records. The binary writer stores records sorted by FRN, so load
    // directly into the compact stable representation instead of rebuilding a
    // large transient HashMap.
    let mut record_frns = Vec::with_capacity(record_count);
    let mut record_values = Vec::with_capacity(record_count);
    let mut record_buf = [0u8; RECORD_SIZE];
    for _ in 0..record_count {
        read_authenticated_chunk(&mut reader, &mut hmac, &mut record_buf, "record")?;
        let frn = read_u64_le(&record_buf, 0, "record frn")?;
        let rec = decode_file_record(&record_buf[FRN_SIZE..])?;
        record_frns.push(frn);
        record_values.push(rec);
    }
    let records = RecordStore::from_sorted_parts(record_frns, record_values)?;

    // Load Hardlinks.
    let mut hardlink_parents: std::collections::HashMap<u64, Vec<u64>> =
        std::collections::HashMap::with_capacity(hardlink_count.min(record_count));
    let mut hardlink_buf = [0u8; HARDLINK_ENTRY_SIZE];
    for _ in 0..hardlink_count {
        read_authenticated_chunk(&mut reader, &mut hmac, &mut hardlink_buf, "hardlink pair")?;
        let child = read_u64_le(&hardlink_buf, 0, "hardlink child")?;
        let parent = read_u64_le(&hardlink_buf, FRN_SIZE, "hardlink parent")?;
        hardlink_parents.entry(child).or_default().push(parent);
    }

    // Load Reparse points.
    let mut reparse_points = std::collections::HashSet::with_capacity(reparse_count);
    let mut reparse_buf = [0u8; REPARSE_ENTRY_SIZE];
    for _ in 0..reparse_count {
        read_authenticated_chunk(&mut reader, &mut hmac, &mut reparse_buf, "reparse point")?;
        let frn = read_u64_le(&reparse_buf, 0, "reparse frn")?;
        reparse_points.insert(frn);
    }

    let mut stored_tag = [0u8; TRAILER_SIZE];
    reader
        .read_exact(&mut stored_tag)
        .map_err(|e| format!("Read HMAC trailer: {}", e))?;

    let computed_tag = hmac
        .finalize()
        .map_err(|e| format!("HMAC compute: {}", e))?;
    if !integrity::ct_eq(&stored_tag, &computed_tag) {
        let _ = std::fs::remove_file(&path);
        return Err("HMAC mismatch (tampering or corruption)".into());
    }

    // Build VolumeIndex.
    let mut index = VolumeIndex::empty(drive_letter);
    index.names = if let Some(arena_bytes) = arena_bytes {
        crate::name_arena::NameArena::from_vec(arena_bytes)
    } else {
        let mmap = unsafe {
            MmapOptions::new()
                .offset(HEADER_SIZE as u64)
                .len(arena_size)
                .map(&file)
        }
        .map_err(|e| format!("Map name arena: {}", e))?;
        crate::name_arena::NameArena::from_mmap(mmap)
    };
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
    index.shrink_to_fit();

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
        drive_letter,
        record_count,
        arena_size,
        elapsed.as_secs_f64()
    );

    Ok(Some((index, state)))
}

/// Metadata from a loaded binary index (mirrors PersistedVolumeState).
pub struct PersistedBinaryState {
    pub journal_id: u64,
    pub last_usn: i64,
    pub files_indexed: u64,
    pub has_hardlink_parent_data: bool,
    pub has_reparse_point_data: bool,
}
