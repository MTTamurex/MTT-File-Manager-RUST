//! Binary index format for fast save/load of the in-memory VolumeIndex.
//!
//! Layout (`MTTIDX06`):
//!   [Header]                     — 80 bytes
//!   [Records]                    — record_count × 40 bytes (8-byte FRN + 32-byte FileRecord)
//!   [Hardlinks]                  — hardlink_entry_count × 16 bytes (8-byte child FRN + 8-byte parent FRN)
//!   [Reparse Points]             — reparse_count × 8 bytes (8-byte FRN)
//!   [NameArena]                  — arena_size raw bytes, compacted from live records
//!   [HMAC-SHA256]                — 32 bytes (covers everything above; key is per-machine, DPAPI-sealed)
//!
//! The records section is placed immediately after the fixed 80-byte header so
//! it starts at an 8-byte-aligned file offset and can be reinterpreted as
//! `&[RecordEntry]` via a read-only memory map (see [`crate::record_store`]).
//! New snapshots always store a raw NameArena containing only names referenced
//! by live records. The variable-size arena is written last so it does not shift
//! the records offset. Existing v6 zstd snapshots remain readable. Older layouts
//! do not contain allocated sizes and are rebuilt.
//!
//! SEC: The trailer is HMAC-SHA256 with a per-machine key sealed by DPAPI. HMAC
//! requires the per-machine key (see [`super::integrity`]).

use std::io::{BufReader, BufWriter, Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use memmap2::MmapOptions;
use windows::Win32::Storage::FileSystem::{FILE_SHARE_DELETE, FILE_SHARE_READ};

use super::integrity::{self, HMAC_OUTPUT_SIZE};
use crate::file_index::{FileRecord, VolumeIndex};
use crate::record_store::RecordStore;

const MAGIC: &[u8; 8] = b"MTTIDX06";
const LEGACY_V5_MAGIC: &[u8; 8] = b"MTTIDX05";
const LEGACY_V4_MAGIC: &[u8; 8] = b"MTTIDX04";
const LEGACY_V3_MAGIC: &[u8; 8] = b"MTTIDX03";
const LEGACY_V2_MAGIC: &[u8; 8] = b"MTTIDX02";
const VERSION: u32 = 6;
const TRAILER_SIZE: usize = HMAC_OUTPUT_SIZE;
const MMAP_ARENA_MIN_BYTES: usize = 64 * 1024 * 1024;
/// Minimum records-section size to memory-map instead of reading into private
/// heap. Below this the RAM saved is negligible and owning avoids mmap setup
/// overhead. ~400k records at 40 bytes each.
const MMAP_RECORDS_MIN_BYTES: usize = 16 * 1024 * 1024;
const HMAC_STREAM_BUF_SIZE: usize = 64 * 1024;
const MAX_RECORDS: usize = 100_000_000;
const MAX_ARENA_BYTES: usize = u32::MAX as usize;
const MAX_HARDLINK_PAIRS: usize = 200_000_000;
const MAX_REPARSE: usize = 10_000_000;

const ARENA_COMPRESSION_NONE: u8 = 0;
const ARENA_COMPRESSION_ZSTD: u8 = 1;

#[repr(C, packed)]
struct Header {
    magic: [u8; 8],
    version: u32,
    drive_letter: u8,
    /// 0 = raw arena, 1 = zstd-compressed arena, 2+ reserved.
    arena_compression: u8,
    _pad: [u8; 2],
    journal_id: u64,
    last_usn: i64,
    record_count: u64,
    /// Uncompressed arena size in bytes.
    arena_size: u64,
    hardlink_entry_count: u64,
    reparse_count: u64,
    /// Bit 0: hardlink_data_complete, Bit 1: reparse_data_complete, Bit 2: sizes_complete
    flags: u64,
    /// On-disk compressed arena size in bytes. Equal to arena_size when uncompressed.
    compressed_arena_size: u64,
}

const HEADER_SIZE: usize = std::mem::size_of::<Header>();
const FRN_SIZE: usize = std::mem::size_of::<u64>();
const FILE_RECORD_SIZE: usize = std::mem::size_of::<FileRecord>();
const RECORD_SIZE: usize = FRN_SIZE + FILE_RECORD_SIZE;
const HARDLINK_ENTRY_SIZE: usize = FRN_SIZE * 2;
const REPARSE_ENTRY_SIZE: usize = FRN_SIZE;
const _: () = {
    assert!(HEADER_SIZE == 80);
    assert!(std::mem::offset_of!(Header, magic) == 0);
    assert!(std::mem::offset_of!(Header, version) == 8);
    assert!(std::mem::offset_of!(Header, drive_letter) == 12);
    assert!(std::mem::offset_of!(Header, arena_compression) == 13);
    assert!(std::mem::offset_of!(Header, _pad) == 14);
    assert!(std::mem::offset_of!(Header, journal_id) == 16);
    assert!(std::mem::offset_of!(Header, last_usn) == 24);
    assert!(std::mem::offset_of!(Header, record_count) == 32);
    assert!(std::mem::offset_of!(Header, arena_size) == 40);
    assert!(std::mem::offset_of!(Header, hardlink_entry_count) == 48);
    assert!(std::mem::offset_of!(Header, reparse_count) == 56);
    assert!(std::mem::offset_of!(Header, flags) == 64);
    assert!(std::mem::offset_of!(Header, compressed_arena_size) == 72);

    assert!(FILE_RECORD_SIZE == 32);
    assert!(std::mem::offset_of!(FileRecord, parent_ref) == 0);
    assert!(std::mem::offset_of!(FileRecord, size) == 8);
    assert!(std::mem::offset_of!(FileRecord, allocated_size) == 16);
    assert!(std::mem::offset_of!(FileRecord, name_offset) == 24);
    assert!(std::mem::offset_of!(FileRecord, name_len) == 28);
    assert!(std::mem::offset_of!(FileRecord, is_dir) == 30);
    assert!(std::mem::offset_of!(FileRecord, _pad) == 31);
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

    let is_dir = match bytes[30] {
        value @ (0 | 1) => value,
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
        allocated_size: read_u64_le(bytes, 16, "record allocated_size")?,
        name_offset: read_u32_le(bytes, 24, "record name_offset")?,
        name_len: read_u16_le(bytes, 28, "record name_len")?,
        is_dir,
        _pad: bytes[31],
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

fn mmap_arena_enabled(arena_size: usize, compression: u8) -> bool {
    // Compressed arenas must be decompressed into memory; they cannot be
    // memory-mapped directly.
    if compression != ARENA_COMPRESSION_NONE {
        return false;
    }
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

/// Whether the records section should be memory-mapped (file-backed, evictable)
/// instead of read into private heap. Enabled by default for large volumes;
/// set `MTT_SEARCH_MMAP_RECORDS=0` to force owned records.
fn mmap_records_enabled(records_bytes: usize) -> bool {
    if records_bytes < MMAP_RECORDS_MIN_BYTES {
        return false;
    }
    match std::env::var("MTT_SEARCH_MMAP_RECORDS") {
        Ok(value) => !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

/// Decompress an arena given its compression type and expected uncompressed size.
fn decompress_arena(
    compressed: &[u8],
    compression: u8,
    expected_size: usize,
) -> Result<Vec<u8>, String> {
    match compression {
        ARENA_COMPRESSION_NONE => {
            if compressed.len() != expected_size {
                return Err(format!(
                    "Uncompressed arena size mismatch: expected {} got {}",
                    expected_size,
                    compressed.len()
                ));
            }
            Ok(compressed.to_vec())
        }
        ARENA_COMPRESSION_ZSTD => {
            let mut decoded = Vec::with_capacity(expected_size);
            zstd::stream::copy_decode(compressed, &mut decoded)
                .map_err(|e| format!("zstd decompress failed: {}", e))?;
            if decoded.len() != expected_size {
                return Err(format!(
                    "Decompressed arena size mismatch: expected {} got {}",
                    expected_size,
                    decoded.len()
                ));
            }
            Ok(decoded)
        }
        other => Err(format!("Unknown arena compression type: {}", other)),
    }
}

fn compact_arena_size(index: &VolumeIndex) -> Result<usize, String> {
    let mut arena_size = 0usize;
    for (&frn, record) in index.records.iter_sorted() {
        let name = index.names.try_get(record.name_ref()).ok_or_else(|| {
            format!(
                "Invalid NameRef for FRN {}: offset {} length {} in {}-byte arena",
                frn,
                record.name_offset,
                record.name_len,
                index.names.len()
            )
        })?;
        arena_size = arena_size
            .checked_add(name.len())
            .ok_or_else(|| "Compacted name arena size overflow".to_string())?;
        if arena_size > MAX_ARENA_BYTES {
            return Err(format!(
                "Compacted name arena too large: {} bytes",
                arena_size
            ));
        }
    }
    Ok(arena_size)
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
    crate::memory_trim::log_process_memory(&format!(
        "{}:\\ before binary snapshot",
        index.drive_letter
    ));

    // Validate every live NameRef and calculate the compact arena size without
    // materializing a second arena-sized buffer.
    let arena_size = compact_arena_size(index)?;

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
        arena_compression: ARENA_COMPRESSION_NONE,
        _pad: [0; 2],
        journal_id: index.journal_id,
        last_usn: index.last_usn,
        record_count: index.records.len() as u64,
        arena_size: arena_size as u64,
        hardlink_entry_count: hardlink_entry_count as u64,
        reparse_count: index.reparse_points.len() as u64,
        flags,
        compressed_arena_size: arena_size as u64,
    };

    // Write header.
    let header_bytes: &[u8] =
        unsafe { std::slice::from_raw_parts(&header as *const Header as *const u8, HEADER_SIZE) };
    write_authenticated_chunk(&mut writer, &mut hmac, header_bytes)?;

    // The immutable base is already sorted. Only overlay references are sorted
    // and merged, keeping save memory proportional to live deltas instead of N.
    // Each temporary record points into the compact arena that is streamed below.
    let mut compact_name_offset = 0usize;
    for (&frn, rec) in index.records.iter_sorted() {
        let name = index.names.try_get(rec.name_ref()).ok_or_else(|| {
            format!(
                "Invalid NameRef for FRN {} while writing records: offset {} length {}",
                frn, rec.name_offset, rec.name_len
            )
        })?;
        let mut compact_rec = *rec;
        compact_rec.name_offset = u32::try_from(compact_name_offset)
            .map_err(|_| "Compacted name offset exceeds u32".to_string())?;
        compact_name_offset = compact_name_offset
            .checked_add(name.len())
            .ok_or_else(|| "Compacted name offset overflow".to_string())?;

        write_authenticated_chunk(&mut writer, &mut hmac, &frn.to_le_bytes())?;
        let rec_bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(
                &compact_rec as *const FileRecord as *const u8,
                FILE_RECORD_SIZE,
            )
        };
        write_authenticated_chunk(&mut writer, &mut hmac, rec_bytes)?;
    }
    debug_assert_eq!(compact_name_offset, arena_size);

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

    // Stream live names in exactly the same sorted-record order used above.
    // NameArena stays last so records remain 8-byte aligned for zero-copy mmap.
    let mut written_arena_size = 0usize;
    for (&frn, rec) in index.records.iter_sorted() {
        let name = index.names.try_get(rec.name_ref()).ok_or_else(|| {
            format!(
                "Invalid NameRef for FRN {} while writing arena: offset {} length {}",
                frn, rec.name_offset, rec.name_len
            )
        })?;
        write_authenticated_chunk(&mut writer, &mut hmac, name.as_bytes())?;
        written_arena_size += name.len();
    }
    debug_assert_eq!(written_arena_size, arena_size);

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
        "[BINARY-IDX] {}:\\ Saved {} records + {} compact arena bytes (reclaimed {} dead bytes) in {:.3}s",
        index.drive_letter,
        index.records.len(),
        arena_size,
        index.names.len().saturating_sub(arena_size),
        elapsed.as_secs_f64()
    );
    crate::memory_trim::log_process_memory(&format!(
        "{}:\\ after binary snapshot",
        index.drive_letter
    ));
    Ok(())
}

/// Peek the 8-byte magic of a binary index file.
fn read_file_magic(path: &Path) -> Result<[u8; 8], String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("Open binary index: {}", e))?;
    let mut magic = [0u8; 8];
    file.read_exact(&mut magic)
        .map_err(|e| format!("Read magic: {}", e))?;
    Ok(magic)
}

fn open_index_read_stable(path: &Path) -> Result<std::fs::File, String> {
    std::fs::OpenOptions::new()
        .read(true)
        // Excluding FILE_SHARE_WRITE keeps authenticated bytes immutable for
        // the lifetime of this handle/mapping. Rename/delete remain possible.
        .share_mode((FILE_SHARE_READ | FILE_SHARE_DELETE).0)
        .open(path)
        .map_err(|e| format!("Open stable binary index: {}", e))
}

fn verify_file_hmac(file: &std::fs::File, file_len: usize) -> Result<(), String> {
    use std::io::{Seek, SeekFrom};

    if file_len < TRAILER_SIZE {
        return Err("binary index is shorter than its HMAC trailer".to_string());
    }
    let key = integrity::machine_key().map_err(|e| format!("HMAC key unavailable: {e}"))?;
    let mut hmac = integrity::HmacSha256::new(&key).map_err(|e| format!("HMAC init: {e}"))?;
    let mut handle = file
        .try_clone()
        .map_err(|e| format!("clone index for HMAC verification: {e}"))?;
    handle
        .seek(SeekFrom::Start(0))
        .map_err(|e| format!("seek index for HMAC verification: {e}"))?;
    let mut reader = BufReader::new(handle);
    read_authenticated_bytes(
        &mut reader,
        &mut hmac,
        file_len - TRAILER_SIZE,
        "snapshot payload",
    )?;
    let mut stored_tag = [0u8; TRAILER_SIZE];
    reader
        .read_exact(&mut stored_tag)
        .map_err(|e| format!("read snapshot HMAC trailer: {e}"))?;
    let computed_tag = hmac.finalize().map_err(|e| format!("HMAC compute: {e}"))?;
    if !integrity::ct_eq(&stored_tag, &computed_tag) {
        return Err("new binary index failed HMAC verification".to_string());
    }
    Ok(())
}

/// Load a VolumeIndex from the binary file. Returns None if the file is absent
/// or a superseded legacy format that must be rebuilt. Returns Err on
/// corruption/tampering (HMAC mismatch, truncation, bad magic).
pub fn load(drive_letter: char) -> Result<Option<(VolumeIndex, PersistedBinaryState)>, String> {
    let path = index_path(drive_letter);

    // Clean up any orphaned .tmp file left by a prior abrupt shutdown (e.g. the
    // installer killed the service mid-write between File::create(.tmp) and the
    // atomic rename(.tmp → .bin)).
    let tmp_path = path.with_extension("bin.tmp");
    if tmp_path.exists() {
        eprintln!(
            "[BINARY-IDX] {}:\\ Orphaned .tmp found at startup; removing (likely caused by \
             abrupt shutdown during a previous save).",
            drive_letter
        );
        let _ = std::fs::remove_file(&tmp_path);
    }

    if !path.exists() {
        return Ok(None);
    }

    // Older formats do not carry allocated sizes. Rebuild them instead of
    // treating an unknown physical allocation as zero.
    let magic = read_file_magic(&path)?;
    if [
        &LEGACY_V2_MAGIC,
        &LEGACY_V3_MAGIC,
        &LEGACY_V4_MAGIC,
        &LEGACY_V5_MAGIC,
    ]
    .iter()
    .any(|legacy| &magic == **legacy)
    {
        eprintln!(
            "[BINARY-IDX] {}:\\ Legacy index with incompatible allocation metrics detected; rebuilding.",
            drive_letter
        );
        let _ = std::fs::remove_file(&path);
        return Ok(None);
    }
    if &magic != MAGIC {
        let _ = std::fs::remove_file(&path);
        return Err("Bad magic".into());
    }

    load_v6(drive_letter, &path)
}

/// How the records section was materialized during a load.
enum RecordsStorage {
    /// Streamed through HMAC and to be memory-mapped after verification.
    Mmap,
    /// Read fully into an owned record store.
    Owned(RecordStore),
}

fn load_v6(
    drive_letter: char,
    path: &Path,
) -> Result<Option<(VolumeIndex, PersistedBinaryState)>, String> {
    let start = std::time::Instant::now();

    let file = open_index_read_stable(path)?;
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

    if header_bytes[..8] != *MAGIC {
        let _ = std::fs::remove_file(path);
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
    let h_arena_compression = header.arena_compression;
    let h_journal_id = header.journal_id;
    let h_last_usn = header.last_usn;
    let h_record_count = header.record_count;
    let h_arena_size = header.arena_size;
    let h_compressed_arena_size = header.compressed_arena_size;
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

    if h_arena_compression != ARENA_COMPRESSION_NONE
        && h_arena_compression != ARENA_COMPRESSION_ZSTD
    {
        return Err(format!(
            "Unsupported arena compression type: {}",
            h_arena_compression
        ));
    }

    let record_count = h_record_count as usize;
    let arena_size = h_arena_size as usize;
    let compressed_arena_size = h_compressed_arena_size as usize;
    let hardlink_count = h_hardlink_count as usize;
    let reparse_count = h_reparse_count as usize;

    // SEC: Sanity caps to prevent OOM via huge HashMap pre-allocation and to
    // make the size-equality check below meaningful even on attacker-crafted
    // headers. NTFS supports up to ~2^48 file records per volume but no
    // realistic deployment exceeds 100M files; arenas above 2 GB would
    // already be rejected by the u32 NameRef offset domain.
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
    // this stage means the header is hostile or corrupt. Use the on-disk
    // compressed arena size for the file layout check.
    let expected = HEADER_SIZE
        .checked_add(compressed_arena_size)
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

    // Records come first in MTTIDX06. Large record sections are streamed through
    // HMAC and then memory-mapped (file-backed, evictable); small ones are read
    // into an owned, sorted RecordStore. The writer stores records sorted by
    // FRN, so a mapped base can binary-search without a validating scan.
    let records_bytes = record_count * RECORD_SIZE;
    let map_records = record_count > 0 && mmap_records_enabled(records_bytes);
    let records_storage = if map_records {
        read_authenticated_bytes(&mut reader, &mut hmac, records_bytes, "records")?;
        RecordsStorage::Mmap
    } else {
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
        RecordsStorage::Owned(RecordStore::from_sorted_parts(record_frns, record_values)?)
    };

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

    // NameArena is written last in MTTIDX06. Uncompressed large arenas are
    // streamed through HMAC and then mapped read-only; compressed arenas are
    // read into memory and decompressed after the whole-file HMAC is verified.
    enum ArenaStorage {
        Mmap,
        Vec(Vec<u8>),
        Compressed(Vec<u8>),
    }

    let arena_storage = match h_arena_compression {
        ARENA_COMPRESSION_NONE => {
            if mmap_arena_enabled(arena_size, ARENA_COMPRESSION_NONE) {
                read_authenticated_bytes(&mut reader, &mut hmac, arena_size, "name arena")?;
                ArenaStorage::Mmap
            } else {
                let mut arena_bytes = vec![0u8; arena_size];
                read_authenticated_chunk(&mut reader, &mut hmac, &mut arena_bytes, "name arena")?;
                ArenaStorage::Vec(arena_bytes)
            }
        }
        ARENA_COMPRESSION_ZSTD => {
            let mut compressed = vec![0u8; compressed_arena_size];
            read_authenticated_chunk(&mut reader, &mut hmac, &mut compressed, "name arena")?;
            ArenaStorage::Compressed(compressed)
        }
        _ => unreachable!("arena_compression validated above"),
    };

    let mut stored_tag = [0u8; TRAILER_SIZE];
    reader
        .read_exact(&mut stored_tag)
        .map_err(|e| format!("Read HMAC trailer: {}", e))?;

    let computed_tag = hmac
        .finalize()
        .map_err(|e| format!("HMAC compute: {}", e))?;
    if !integrity::ct_eq(&stored_tag, &computed_tag) {
        let _ = std::fs::remove_file(path);
        return Err("HMAC mismatch (tampering or corruption)".into());
    }

    // Build VolumeIndex. Both the records and the arena may be memory-mapped;
    // the whole file was authenticated above so the mappings are trusted.
    let mut index = VolumeIndex::empty(drive_letter);
    index.records = match records_storage {
        RecordsStorage::Owned(store) => store,
        RecordsStorage::Mmap => {
            let mmap = unsafe {
                MmapOptions::new()
                    .offset(HEADER_SIZE as u64)
                    .len(records_bytes)
                    .map(&file)
            }
            .map_err(|e| format!("Map records: {}", e))?;
            match RecordStore::from_mmap(mmap, record_count) {
                Ok(store) => store,
                Err(e) => {
                    // Should be unreachable (offset 80 is 8-aligned), but fall
                    // back to an owned read so a load never fails on this alone.
                    eprintln!(
                        "[BINARY-IDX] {}:\\ records mmap unusable ({}); reading owned",
                        drive_letter, e
                    );
                    read_records_region_owned(&file, record_count)?
                }
            }
        }
    };
    let arena_offset = HEADER_SIZE
        + records_bytes
        + hardlink_count * HARDLINK_ENTRY_SIZE
        + reparse_count * REPARSE_ENTRY_SIZE;
    index.names = match arena_storage {
        ArenaStorage::Vec(bytes) => crate::name_arena::NameArena::from_vec(bytes),
        ArenaStorage::Mmap => {
            let mmap = unsafe {
                MmapOptions::new()
                    .offset(arena_offset as u64)
                    .len(arena_size)
                    .map(&file)
            }
            .map_err(|e| format!("Map name arena: {}", e))?;
            crate::name_arena::NameArena::from_mmap(mmap)
        }
        ArenaStorage::Compressed(compressed) => {
            let decompressed = decompress_arena(&compressed, h_arena_compression, arena_size)
                .inspect_err(|_| {
                    let _ = std::fs::remove_file(path);
                })?;
            crate::name_arena::NameArena::from_vec(decompressed)
        }
    };
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
    let compression_label = if h_arena_compression == ARENA_COMPRESSION_ZSTD {
        "compressed"
    } else {
        "raw"
    };
    eprintln!(
        "[BINARY-IDX] {}:\\ Loaded {} records ({}) + {} {} arena bytes in {:.3}s",
        drive_letter,
        record_count,
        if map_records { "mmap" } else { "owned" },
        arena_size,
        compression_label,
        elapsed.as_secs_f64()
    );

    Ok(Some((index, state)))
}

/// Read the records section into an owned RecordStore. Used as a fallback when
/// the records region cannot be memory-mapped. The whole file was already
/// HMAC-verified by the caller, so this trusts the on-disk bytes.
fn read_records_region_owned(
    file: &std::fs::File,
    record_count: usize,
) -> Result<RecordStore, String> {
    use std::io::{Seek, SeekFrom};
    let mut handle = file
        .try_clone()
        .map_err(|e| format!("clone index handle: {}", e))?;
    handle
        .seek(SeekFrom::Start(HEADER_SIZE as u64))
        .map_err(|e| format!("seek records: {}", e))?;
    let mut reader = BufReader::new(handle);
    let mut record_frns = Vec::with_capacity(record_count);
    let mut record_values = Vec::with_capacity(record_count);
    let mut record_buf = [0u8; RECORD_SIZE];
    for _ in 0..record_count {
        reader
            .read_exact(&mut record_buf)
            .map_err(|e| format!("read record: {}", e))?;
        let frn = read_u64_le(&record_buf, 0, "record frn")?;
        let rec = decode_file_record(&record_buf[FRN_SIZE..])?;
        record_frns.push(frn);
        record_values.push(rec);
    }
    RecordStore::from_sorted_parts(record_frns, record_values)
}

/// Build corresponding record and name storage from one handle to the snapshot
/// this process just wrote. Non-empty regions prefer mmap and fall back to owned
/// reads; empty regions stay owned because zero-length mappings are invalid.
fn build_saved_storage_pair(
    drive_letter: char,
) -> Result<(RecordStore, crate::name_arena::NameArena), String> {
    let path = index_path(drive_letter);
    let file = open_index_read_stable(&path)?;
    let file_len = file
        .metadata()
        .map_err(|e| format!("index metadata: {}", e))?
        .len();
    let file_len = usize::try_from(file_len).map_err(|_| "index file too large".to_string())?;
    if file_len < HEADER_SIZE + TRAILER_SIZE {
        return Err("new binary index is too small".to_string());
    }
    verify_file_hmac(&file, file_len)?;

    let mut header_bytes = [0u8; HEADER_SIZE];
    {
        use std::io::{Seek, SeekFrom};
        let mut file_cursor = &file;
        file_cursor
            .seek(SeekFrom::Start(0))
            .map_err(|e| format!("seek verified index header: {e}"))?;
        let mut reader = BufReader::new(&file);
        reader
            .read_exact(&mut header_bytes)
            .map_err(|e| format!("read header: {}", e))?;
    }
    if header_bytes[..8] != *MAGIC {
        return Err("new binary index has bad magic".to_string());
    }
    let header: Header =
        unsafe { std::ptr::read_unaligned(header_bytes.as_ptr() as *const Header) };
    let version = header.version;
    let file_drive_letter = header.drive_letter;
    let arena_compression = header.arena_compression;
    let record_count = usize::try_from(header.record_count)
        .map_err(|_| "new binary index record count overflow".to_string())?;
    let arena_size = usize::try_from(header.arena_size)
        .map_err(|_| "new binary index arena size overflow".to_string())?;
    let compressed_arena_size = usize::try_from(header.compressed_arena_size)
        .map_err(|_| "new binary index compressed arena size overflow".to_string())?;
    let hardlink_count = usize::try_from(header.hardlink_entry_count)
        .map_err(|_| "new binary index hardlink count overflow".to_string())?;
    let reparse_count = usize::try_from(header.reparse_count)
        .map_err(|_| "new binary index reparse count overflow".to_string())?;

    if record_count > MAX_RECORDS
        || arena_size > MAX_ARENA_BYTES
        || hardlink_count > MAX_HARDLINK_PAIRS
        || reparse_count > MAX_REPARSE
    {
        return Err("new binary index exceeds validated size limits".to_string());
    }

    if version != VERSION
        || file_drive_letter as char != drive_letter
        || arena_compression != ARENA_COMPRESSION_NONE
        || compressed_arena_size != arena_size
    {
        return Err("new binary index header does not describe a raw v6 snapshot".to_string());
    }
    let records_bytes = record_count
        .checked_mul(RECORD_SIZE)
        .ok_or_else(|| "new binary index records size overflow".to_string())?;
    let arena_offset = HEADER_SIZE
        .checked_add(records_bytes)
        .and_then(|size| size.checked_add(hardlink_count.checked_mul(HARDLINK_ENTRY_SIZE)?))
        .and_then(|size| size.checked_add(reparse_count.checked_mul(REPARSE_ENTRY_SIZE)?))
        .ok_or_else(|| "new binary index arena offset overflow".to_string())?;
    let expected_size = arena_offset
        .checked_add(arena_size)
        .and_then(|size| size.checked_add(TRAILER_SIZE))
        .ok_or_else(|| "new binary index size overflow".to_string())?;
    if expected_size != file_len {
        return Err(format!(
            "new binary index size mismatch: expected {} got {}",
            expected_size, file_len
        ));
    }

    let records = if record_count == 0 {
        RecordStore::new()
    } else {
        match unsafe {
            MmapOptions::new()
                .offset(HEADER_SIZE as u64)
                .len(records_bytes)
                .map(&file)
        } {
            Ok(mmap) => match RecordStore::from_mmap(mmap, record_count) {
                Ok(store) => store,
                Err(error) => {
                    eprintln!(
                        "[BINARY-IDX] {}:\\ records remap unusable ({}); reading owned",
                        drive_letter, error
                    );
                    read_records_region_owned(&file, record_count)?
                }
            },
            Err(error) => {
                eprintln!(
                    "[BINARY-IDX] {}:\\ records remap failed ({}); reading owned",
                    drive_letter, error
                );
                read_records_region_owned(&file, record_count)?
            }
        }
    };

    let names = if arena_size == 0 {
        crate::name_arena::NameArena::with_capacity(0)
    } else {
        match unsafe {
            MmapOptions::new()
                .offset(arena_offset as u64)
                .len(arena_size)
                .map(&file)
        } {
            Ok(mmap) => crate::name_arena::NameArena::from_mmap(mmap),
            Err(error) => {
                eprintln!(
                    "[BINARY-IDX] {}:\\ arena remap failed ({}); reading owned",
                    drive_letter, error
                );
                crate::name_arena::NameArena::from_vec(read_arena_region_owned(
                    &file,
                    arena_offset,
                    arena_size,
                )?)
            }
        }
    };

    Ok((records, names))
}

fn read_arena_region_owned(
    file: &std::fs::File,
    arena_offset: usize,
    arena_size: usize,
) -> Result<Vec<u8>, String> {
    use std::io::{Seek, SeekFrom};

    let mut handle = file
        .try_clone()
        .map_err(|e| format!("clone index handle: {}", e))?;
    handle
        .seek(SeekFrom::Start(arena_offset as u64))
        .map_err(|e| format!("seek name arena: {}", e))?;
    let mut bytes = vec![0u8; arena_size];
    handle
        .read_exact(&mut bytes)
        .map_err(|e| format!("read name arena: {}", e))?;
    Ok(bytes)
}

/// Save the index as a compact raw MTTIDX06 snapshot, then replace both the
/// record store and NameArena with corresponding storage from that same file.
///
/// Both replacement values are fully constructed before either current value is
/// changed. Mappings are preferred even for small non-empty regions to minimize
/// private heap; owned reads are retained as a portability/error fallback.
pub fn save_and_remap(index: &mut VolumeIndex) -> Result<(), String> {
    save(index)?;
    let drive_letter = index.drive_letter;
    match build_saved_storage_pair(drive_letter) {
        Ok((records, names)) => {
            index.records = records;
            index.names = names;
        }
        Err(e) => {
            eprintln!(
                "[BINARY-IDX] {}:\\ snapshot remap skipped: {}",
                drive_letter,
                crate::redact_paths(&e)
            );
        }
    }
    Ok(())
}

/// Metadata from a loaded binary index (mirrors PersistedVolumeState).
pub struct PersistedBinaryState {
    pub journal_id: u64,
    pub last_usn: i64,
    pub files_indexed: u64,
    pub has_hardlink_parent_data: bool,
    pub has_reparse_point_data: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_index::VolumeIndex;

    fn clean_index_files(drive: char) {
        let path = index_path(drive);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("bin.tmp"));
    }

    fn read_header(path: &Path) -> Header {
        let bytes = std::fs::read(path).expect("read snapshot");
        assert!(bytes.len() >= HEADER_SIZE);
        unsafe { std::ptr::read_unaligned(bytes.as_ptr() as *const Header) }
    }

    fn rewrite_arena_as_legacy_zstd(path: &Path) {
        let bytes = std::fs::read(path).expect("read raw v6 snapshot");
        let mut header: Header =
            unsafe { std::ptr::read_unaligned(bytes.as_ptr() as *const Header) };
        let record_count = header.record_count as usize;
        let hardlink_count = header.hardlink_entry_count as usize;
        let reparse_count = header.reparse_count as usize;
        let arena_size = header.arena_size as usize;
        let arena_offset = HEADER_SIZE
            + record_count * RECORD_SIZE
            + hardlink_count * HARDLINK_ENTRY_SIZE
            + reparse_count * REPARSE_ENTRY_SIZE;
        let compressed = zstd::encode_all(&bytes[arena_offset..arena_offset + arena_size], 1)
            .expect("compress legacy arena");

        header.arena_compression = ARENA_COMPRESSION_ZSTD;
        header.compressed_arena_size = compressed.len() as u64;
        let header_bytes = unsafe {
            std::slice::from_raw_parts(&header as *const Header as *const u8, HEADER_SIZE)
        };
        let mut payload = Vec::with_capacity(arena_offset + compressed.len());
        payload.extend_from_slice(header_bytes);
        payload.extend_from_slice(&bytes[HEADER_SIZE..arena_offset]);
        payload.extend_from_slice(&compressed);

        let key = integrity::machine_key().expect("test HMAC key");
        let mut hmac = integrity::HmacSha256::new(&key).expect("HMAC init");
        hmac.update(&payload).expect("HMAC update");
        payload.extend_from_slice(&hmac.finalize().expect("HMAC finalize"));
        std::fs::write(path, payload).expect("write legacy zstd v6 snapshot");
    }

    fn sample_index(drive: char) -> VolumeIndex {
        let mut index = VolumeIndex::empty(drive);
        assert!(index.insert_record(10, "docs", 5, true, false));
        assert!(index.insert_record(20, "a.txt", 10, false, false));
        index.records.get_mut(&20).unwrap().size = 55;
        index.records.get_mut(&20).unwrap().allocated_size = 64;
        assert!(index.insert_record(30, "root.bin", 5, false, false));
        index.records.get_mut(&30).unwrap().size = 7;
        index.records.get_mut(&30).unwrap().allocated_size = 8;
        index.journal_id = 7;
        index.last_usn = 8;
        index.hardlink_data_complete = true;
        index.reparse_data_complete = true;
        index.sizes_loaded = true;
        index.rebuild_children();
        index
    }

    fn assert_matches_sample(index: &VolumeIndex, drive: char) {
        assert_eq!(index.records.len(), 3);
        assert_eq!(index.records.get(&20).unwrap().size, 55);
        assert_eq!(index.records.get(&20).unwrap().allocated_size, 64);
        assert_eq!(index.records.get(&30).unwrap().size, 7);
        assert_eq!(index.records.get(&30).unwrap().allocated_size, 8);
        assert_eq!(
            index.resolve_path_to_frn(&format!(r"{}:\docs", drive)),
            Some(10)
        );
        let (total, files, folders, _zero) = index.folder_tree_summary(5);
        assert_eq!((total, files, folders), (62, 2, 1));
    }

    #[test]
    fn v6_round_trips_through_save_and_load() {
        crate::index_db::init_data_dir_for_tests();
        let drive = 'W';
        let path = index_path(drive);
        clean_index_files(drive);

        let index = sample_index(drive);
        save(&index).expect("save v6");

        let (loaded, state) = load(drive).expect("load v6").expect("index present");
        assert_eq!(state.journal_id, 7);
        assert_eq!(state.last_usn, 8);
        assert!(state.has_hardlink_parent_data);
        assert!(state.has_reparse_point_data);
        assert_matches_sample(&loaded, drive);

        assert_eq!(&read_file_magic(&path).unwrap(), MAGIC);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_compacts_dead_arena_bytes_and_writes_raw_names() {
        crate::index_db::init_data_dir_for_tests();
        let drive = 'X';
        let path = index_path(drive);
        clean_index_files(drive);

        let mut index = VolumeIndex::empty(drive);
        assert!(index.insert_record(10, "dead-name", 5, false, false));
        assert!(index.insert_record(20, "live", 5, false, false));
        index.remove_record(10);
        assert!(index.names.len() > "live".len());

        save(&index).expect("save compact v6");
        let header = read_header(&path);
        let arena_compression = header.arena_compression;
        let arena_size = header.arena_size;
        let compressed_arena_size = header.compressed_arena_size;
        assert_eq!(arena_compression, ARENA_COMPRESSION_NONE);
        assert_eq!(arena_size, "live".len() as u64);
        assert_eq!(compressed_arena_size, arena_size);

        let (loaded, _) = load(drive).unwrap().unwrap();
        assert_eq!(loaded.names.len(), "live".len());
        assert_eq!(
            loaded
                .names
                .try_get(loaded.records.get(&20).unwrap().name_ref()),
            Some("live")
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn save_and_remap_keeps_records_and_names_corresponding() {
        crate::index_db::init_data_dir_for_tests();
        let drive = 'Y';
        clean_index_files(drive);

        let mut index = sample_index(drive);
        assert!(index.insert_record(1, "dead-prefix", 5, false, false));
        index.remove_record(1);
        save_and_remap(&mut index).expect("save and remap");

        assert_eq!(
            index
                .names
                .try_get(index.records.get(&10).unwrap().name_ref()),
            Some("docs")
        );
        assert_eq!(
            index
                .names
                .try_get(index.records.get(&20).unwrap().name_ref()),
            Some("a.txt")
        );
        assert!(index.insert_record(40, "after-remap", 5, false, false));
        assert_eq!(
            index
                .names
                .try_get(index.records.get(&40).unwrap().name_ref()),
            Some("after-remap")
        );
        clean_index_files(drive);
    }

    #[test]
    fn remap_rejects_tampered_snapshot_handle() {
        use std::io::{Seek, SeekFrom};

        crate::index_db::init_data_dir_for_tests();
        let drive = 'T';
        let path = index_path(drive);
        clean_index_files(drive);
        save(&sample_index(drive)).expect("save snapshot");

        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .expect("open snapshot for tamper test");
        file.seek(SeekFrom::Start(HEADER_SIZE as u64))
            .expect("seek record byte");
        file.write_all(&[0xff]).expect("tamper record byte");
        file.sync_all().expect("flush tampered byte");
        drop(file);

        let error = match build_saved_storage_pair(drive) {
            Ok(_) => panic!("tampered snapshot must not map"),
            Err(error) => error,
        };
        assert!(error.contains("HMAC"), "{error}");
        clean_index_files(drive);
    }

    #[test]
    fn unicode_and_empty_names_round_trip() {
        crate::index_db::init_data_dir_for_tests();
        let drive = 'U';
        clean_index_files(drive);

        let mut index = VolumeIndex::empty(drive);
        assert!(index.insert_record(10, "caf\u{e9}_\u{65e5}\u{672c}", 5, false, false));
        assert!(index.insert_record(20, "", 5, false, false));
        save(&index).expect("save unicode and empty names");

        let (loaded, _) = load(drive).unwrap().unwrap();
        assert_eq!(
            loaded
                .names
                .try_get(loaded.records.get(&10).unwrap().name_ref()),
            Some("caf\u{e9}_\u{65e5}\u{672c}")
        );
        assert_eq!(
            loaded
                .names
                .try_get(loaded.records.get(&20).unwrap().name_ref()),
            Some("")
        );
        clean_index_files(drive);
    }

    #[test]
    fn serializer_rejects_invalid_name_references_and_utf8() {
        crate::index_db::init_data_dir_for_tests();

        let bounds_drive = 'I';
        clean_index_files(bounds_drive);
        let mut invalid_bounds = VolumeIndex::empty(bounds_drive);
        assert!(invalid_bounds.insert_record(10, "valid", 5, false, false));
        invalid_bounds.records.get_mut(&10).unwrap().name_offset = 99;
        let error = save(&invalid_bounds).expect_err("out-of-bounds NameRef must fail");
        assert!(error.contains("Invalid NameRef"), "{error}");

        let utf8_drive = 'J';
        clean_index_files(utf8_drive);
        let mut invalid_utf8 = VolumeIndex::empty(utf8_drive);
        assert!(invalid_utf8.insert_record(10, "x", 5, false, false));
        invalid_utf8.names = crate::name_arena::NameArena::from_vec(vec![0xff]);
        let error = save(&invalid_utf8).expect_err("invalid UTF-8 NameRef must fail");
        assert!(error.contains("Invalid NameRef"), "{error}");

        clean_index_files(bounds_drive);
        clean_index_files(utf8_drive);
    }

    #[test]
    fn empty_snapshot_saves_remaps_and_loads() {
        crate::index_db::init_data_dir_for_tests();
        let drive = 'Q';
        clean_index_files(drive);

        let mut index = VolumeIndex::empty(drive);
        save_and_remap(&mut index).expect("save and remap empty index");
        assert!(index.records.is_empty());
        assert_eq!(index.names.len(), 0);

        let (loaded, _) = load(drive).unwrap().unwrap();
        assert!(loaded.records.is_empty());
        assert_eq!(loaded.names.len(), 0);
        clean_index_files(drive);
    }

    #[test]
    fn nonempty_records_with_empty_arena_remap() {
        crate::index_db::init_data_dir_for_tests();
        let drive = 'E';
        clean_index_files(drive);

        let mut index = VolumeIndex::empty(drive);
        assert!(index.insert_record(10, "", 5, false, false));
        assert!(index.insert_record(20, "", 5, false, false));
        save_and_remap(&mut index).expect("remap records with empty arena");

        assert_eq!(index.names.len(), 0);
        for frn in [10, 20] {
            assert_eq!(
                index
                    .names
                    .try_get(index.records.get(&frn).unwrap().name_ref()),
                Some("")
            );
        }
        clean_index_files(drive);
    }

    #[test]
    fn legacy_zstd_v6_snapshot_still_loads() {
        crate::index_db::init_data_dir_for_tests();
        let drive = 'Z';
        let path = index_path(drive);
        clean_index_files(drive);

        let index = sample_index(drive);
        save(&index).expect("save raw v6");
        rewrite_arena_as_legacy_zstd(&path);

        let header = read_header(&path);
        let arena_compression = header.arena_compression;
        assert_eq!(arena_compression, ARENA_COMPRESSION_ZSTD);
        let (loaded, _) = load(drive).expect("load legacy zstd v6").unwrap();
        assert_matches_sample(&loaded, drive);
        clean_index_files(drive);
    }
}
