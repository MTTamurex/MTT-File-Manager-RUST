//! Binary index format for fast save/load of the in-memory VolumeIndex.
//!
//! Layout (`MTTIDX04`):
//!   [Header]                     — 80 bytes
//!   [Records]                    — record_count × 32 bytes (8-byte FRN + 24-byte FileRecord)
//!   [Hardlinks]                  — hardlink_entry_count × 16 bytes (8-byte child FRN + 8-byte parent FRN)
//!   [Reparse Points]             — reparse_count × 8 bytes (8-byte FRN)
//!   [NameArena]                  — arena_size bytes (optionally zstd-compressed)
//!   [HMAC-SHA256]                — 32 bytes (covers everything above; key is per-machine, DPAPI-sealed)
//!
//! The records section is placed immediately after the fixed 80-byte header so
//! it starts at an 8-byte-aligned file offset and can be reinterpreted as
//! `&[RecordEntry]` via a read-only memory map (see [`crate::record_store`]).
//! The variable-size NameArena is written last so it does not shift the records
//! offset. `MTTIDX03` files (arena-first) are transparently reordered to
//! `MTTIDX04` on load without a full re-scan; `MTTIDX02` is discarded.
//!
//! SEC: The trailer is HMAC-SHA256 with a per-machine key sealed by DPAPI. HMAC
//! requires the per-machine key (see [`super::integrity`]).

use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use memmap2::{Mmap, MmapOptions};
use windows::Win32::Storage::FileSystem::{FILE_SHARE_DELETE, FILE_SHARE_READ};

use super::integrity::{self, HMAC_OUTPUT_SIZE};
use crate::file_index::{FileRecord, VolumeIndex};
use crate::record_store::RecordStore;

const MAGIC: &[u8; 8] = b"MTTIDX04";
const LEGACY_V3_MAGIC: &[u8; 8] = b"MTTIDX03";
const LEGACY_V2_MAGIC: &[u8; 8] = b"MTTIDX02";
const VERSION: u32 = 4;
const LEGACY_V3_VERSION: u32 = 3;
const TRAILER_SIZE: usize = HMAC_OUTPUT_SIZE;
const MMAP_ARENA_MIN_BYTES: usize = 64 * 1024 * 1024;
/// Minimum records-section size to memory-map instead of reading into private
/// heap. Below this the RAM saved is negligible and owning avoids mmap setup
/// overhead. ~512k records at 32 bytes each.
const MMAP_RECORDS_MIN_BYTES: usize = 16 * 1024 * 1024;
const HMAC_STREAM_BUF_SIZE: usize = 64 * 1024;
const MAX_RECORDS: usize = 100_000_000;
const MAX_ARENA_BYTES: usize = u32::MAX as usize;
const MAX_HARDLINK_PAIRS: usize = 200_000_000;
const MAX_REPARSE: usize = 10_000_000;

const ARENA_COMPRESSION_NONE: u8 = 0;
const ARENA_COMPRESSION_ZSTD: u8 = 1;
const ARENA_MIN_RECORDS_TO_COMPRESS: u64 = 50_000;
/// Only compress if the compressed size is at most this ratio of the original.
const ARENA_MIN_COMPRESSION_RATIO: f64 = 0.95;
const ZSTD_COMPRESSION_LEVEL: i32 = 1;

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

/// Compress the raw arena bytes using zstd level 1. Returns `None` if
/// compression fails or does not meet the minimum savings threshold.
fn compress_arena(raw: &[u8], record_count: u64) -> Option<Vec<u8>> {
    if record_count < ARENA_MIN_RECORDS_TO_COMPRESS {
        return None;
    }

    let compressed = zstd::encode_all(raw, ZSTD_COMPRESSION_LEVEL).ok()?;
    let ratio = compressed.len() as f64 / raw.len().max(1) as f64;
    if ratio > ARENA_MIN_COMPRESSION_RATIO {
        return None;
    }

    Some(compressed)
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

    // Collect the NameArena into a contiguous buffer so we can optionally
    // compress it. For large indexes this saves significant disk space; the
    // temporary duplicate is freed as soon as the save finishes.
    let arena_size = index.names.len();
    let mut raw_arena = Vec::with_capacity(arena_size);
    index
        .names
        .for_each_slice(|slice| raw_arena.extend_from_slice(slice));

    let (arena_compression, arena_bytes): (u8, Vec<u8>) =
        match compress_arena(&raw_arena, index.records.len() as u64) {
            Some(compressed) => {
                eprintln!(
                    "[BINARY-IDX] {}:\\ Compressing arena {} -> {} bytes ({:.1}%)",
                    index.drive_letter,
                    arena_size,
                    compressed.len(),
                    compressed.len() as f64 / arena_size.max(1) as f64 * 100.0
                );
                (ARENA_COMPRESSION_ZSTD, compressed)
            }
            None => (ARENA_COMPRESSION_NONE, raw_arena),
        };

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
        arena_compression,
        _pad: [0; 2],
        journal_id: index.journal_id,
        last_usn: index.last_usn,
        record_count: index.records.len() as u64,
        arena_size: arena_size as u64,
        hardlink_entry_count: hardlink_entry_count as u64,
        reparse_count: index.reparse_points.len() as u64,
        flags,
        compressed_arena_size: arena_bytes.len() as u64,
    };

    // Write header.
    let header_bytes: &[u8] =
        unsafe { std::slice::from_raw_parts(&header as *const Header as *const u8, HEADER_SIZE) };
    write_authenticated_chunk(&mut writer, &mut hmac, header_bytes)?;

    // The immutable base is already sorted. Only overlay references are sorted
    // and merged, keeping save memory proportional to live deltas instead of N.
    for (&frn, rec) in index.records.iter_sorted() {
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

    // Write NameArena (compressed or raw) LAST so the fixed-size records section
    // stays at an 8-byte-aligned offset (HEADER_SIZE) for zero-copy mmap on load.
    write_authenticated_chunk(&mut writer, &mut hmac, &arena_bytes)?;

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

    // Select format by magic; migrate legacy layouts before reading.
    let magic = read_file_magic(&path)?;
    if &magic == LEGACY_V2_MAGIC {
        eprintln!(
            "[BINARY-IDX] {}:\\ Legacy MTTIDX02 format detected; discarding and rebuilding.",
            drive_letter
        );
        let _ = std::fs::remove_file(&path);
        return Ok(None);
    }
    if &magic == LEGACY_V3_MAGIC {
        // Reorder MTTIDX03 (arena-first) to MTTIDX04 (records-first) in place so
        // records become memory-mappable, without a full MFT re-scan.
        match migrate_v3_to_v4(&path, drive_letter) {
            Ok(true) => {} // file is now MTTIDX04; fall through to load it
            Ok(false) => {
                eprintln!(
                    "[BINARY-IDX] {}:\\ MTTIDX03 file corrupt/mismatched; rebuilding.",
                    drive_letter
                );
                let _ = std::fs::remove_file(&path);
                return Ok(None);
            }
            Err(e) => {
                eprintln!(
                    "[BINARY-IDX] {}:\\ MTTIDX03 -> MTTIDX04 migration failed ({}); preserving the old snapshot.",
                    drive_letter, e
                );
                return Err(e);
            }
        }
    } else if &magic != MAGIC {
        let _ = std::fs::remove_file(&path);
        return Err("Bad magic".into());
    }

    load_v4(drive_letter, &path)
}

/// How the records section was materialized during a load.
enum RecordsStorage {
    /// Streamed through HMAC and to be memory-mapped after verification.
    Mmap,
    /// Read fully into an owned record store.
    Owned(RecordStore),
}

fn load_v4(
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

    // Records come first in MTTIDX04. Large record sections are streamed through
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

    // NameArena is written last in MTTIDX04. Uncompressed large arenas are
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

/// Reorder an `MTTIDX03` file (arena-first) into `MTTIDX04` (records-first) in
/// place so records can be memory-mapped, without a full MFT re-scan. The v3
/// HMAC is verified first, then recomputed over the new layout. Returns
/// `Ok(false)` if the file is corrupt or mismatched (caller rebuilds).
fn migrate_v3_to_v4(path: &Path, drive_letter: char) -> Result<bool, String> {
    let mut source = open_index_read_stable(path)?;
    let file_len = source
        .metadata()
        .map_err(|e| format!("read v3 metadata: {}", e))?
        .len() as usize;
    if file_len < HEADER_SIZE + TRAILER_SIZE {
        return Ok(false);
    }

    let mut header_bytes = [0u8; HEADER_SIZE];
    source
        .read_exact(&mut header_bytes)
        .map_err(|e| format!("read v3 header: {}", e))?;
    if header_bytes[..8] != *LEGACY_V3_MAGIC {
        return Ok(false);
    }

    let header: Header =
        unsafe { std::ptr::read_unaligned(header_bytes.as_ptr() as *const Header) };
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

    if h_version != LEGACY_V3_VERSION
        || h_drive_letter as char != drive_letter
        || (h_arena_compression != ARENA_COMPRESSION_NONE
            && h_arena_compression != ARENA_COMPRESSION_ZSTD)
    {
        return Ok(false);
    }

    let record_count = usize::try_from(h_record_count)
        .map_err(|_| "record_count does not fit usize".to_string())?;
    let arena_size =
        usize::try_from(h_arena_size).map_err(|_| "arena_size does not fit usize".to_string())?;
    let hardlink_count = usize::try_from(h_hardlink_count)
        .map_err(|_| "hardlink count does not fit usize".to_string())?;
    let reparse_count = usize::try_from(h_reparse_count)
        .map_err(|_| "reparse count does not fit usize".to_string())?;
    let compressed_arena_size = usize::try_from(h_compressed_arena_size)
        .map_err(|_| "compressed arena size does not fit usize".to_string())?;
    if record_count > MAX_RECORDS
        || arena_size > MAX_ARENA_BYTES
        || compressed_arena_size > MAX_ARENA_BYTES
        || hardlink_count > MAX_HARDLINK_PAIRS
        || reparse_count > MAX_REPARSE
    {
        return Ok(false);
    }

    let overflow = || "size overflow in v3 header".to_string();
    let records_bytes = record_count.checked_mul(RECORD_SIZE).ok_or_else(overflow)?;
    let hardlink_bytes = hardlink_count
        .checked_mul(HARDLINK_ENTRY_SIZE)
        .ok_or_else(overflow)?;
    let reparse_bytes = reparse_count
        .checked_mul(REPARSE_ENTRY_SIZE)
        .ok_or_else(overflow)?;
    let payload = HEADER_SIZE
        .checked_add(compressed_arena_size)
        .and_then(|s| s.checked_add(records_bytes))
        .and_then(|s| s.checked_add(hardlink_bytes))
        .and_then(|s| s.checked_add(reparse_bytes))
        .ok_or_else(overflow)?;
    let expected = payload.checked_add(TRAILER_SIZE).ok_or_else(overflow)?;
    if file_len != expected {
        return Ok(false);
    }

    // Verify the v3 HMAC in fixed-size chunks before copying any section.
    let key = integrity::machine_key().map_err(|e| format!("HMAC key: {}", e))?;
    let mut hmac = integrity::HmacSha256::new(&key).map_err(|e| format!("HMAC init: {}", e))?;
    hmac.update(&header_bytes)
        .map_err(|e| format!("HMAC update: {}", e))?;
    {
        let mut reader = BufReader::new(&source);
        read_authenticated_bytes(&mut reader, &mut hmac, payload - HEADER_SIZE, "v3 payload")?;
        let mut stored_tag = [0u8; TRAILER_SIZE];
        reader
            .read_exact(&mut stored_tag)
            .map_err(|e| format!("read v3 HMAC: {}", e))?;
        let tag = hmac
            .finalize()
            .map_err(|e| format!("HMAC compute: {}", e))?;
        if !integrity::ct_eq(&tag, &stored_tag) {
            return Ok(false);
        }
    }

    // v4 header: identical fields, updated magic/version.
    let new_header = Header {
        magic: *MAGIC,
        version: VERSION,
        drive_letter: h_drive_letter,
        arena_compression: h_arena_compression,
        _pad: [0; 2],
        journal_id: h_journal_id,
        last_usn: h_last_usn,
        record_count: h_record_count,
        arena_size: h_arena_size,
        hardlink_entry_count: h_hardlink_count,
        reparse_count: h_reparse_count,
        flags: h_flags,
        compressed_arena_size: h_compressed_arena_size,
    };
    let new_header_bytes: &[u8] = unsafe {
        std::slice::from_raw_parts(&new_header as *const Header as *const u8, HEADER_SIZE)
    };

    let tmp_path = path.with_extension("bin.tmp");
    let out = std::fs::File::create(&tmp_path).map_err(|e| format!("create tmp: {}", e))?;
    let mut writer = BufWriter::new(out);
    let mut out_hmac = integrity::HmacSha256::new(&key).map_err(|e| format!("HMAC init: {}", e))?;
    write_authenticated_chunk(&mut writer, &mut out_hmac, new_header_bytes)?;

    // v3 layout: [header][arena][records][hardlinks][reparse]. Copy each
    // section directly into its v4 position with one reusable 64 KiB buffer.
    let arena_offset = HEADER_SIZE as u64;
    let records_offset = arena_offset + compressed_arena_size as u64;
    let hardlinks_offset = records_offset + records_bytes as u64;
    let reparse_offset = hardlinks_offset + hardlink_bytes as u64;
    let mut copy_buffer = vec![0u8; HMAC_STREAM_BUF_SIZE];
    copy_authenticated_file_section(
        &mut source,
        records_offset,
        records_bytes,
        &mut writer,
        &mut out_hmac,
        &mut copy_buffer,
        "records",
    )?;
    copy_authenticated_file_section(
        &mut source,
        hardlinks_offset,
        hardlink_bytes,
        &mut writer,
        &mut out_hmac,
        &mut copy_buffer,
        "hardlinks",
    )?;
    copy_authenticated_file_section(
        &mut source,
        reparse_offset,
        reparse_bytes,
        &mut writer,
        &mut out_hmac,
        &mut copy_buffer,
        "reparse points",
    )?;
    copy_authenticated_file_section(
        &mut source,
        arena_offset,
        compressed_arena_size,
        &mut writer,
        &mut out_hmac,
        &mut copy_buffer,
        "name arena",
    )?;
    let out_tag = out_hmac
        .finalize()
        .map_err(|e| format!("HMAC compute: {}", e))?;
    writer
        .write_all(&out_tag)
        .map_err(|e| format!("write trailer: {}", e))?;
    writer.flush().map_err(|e| format!("flush: {}", e))?;
    let out = writer
        .into_inner()
        .map_err(|e| format!("finish write: {}", e))?;
    out.sync_all().map_err(|e| format!("sync: {}", e))?;
    drop(out);
    std::fs::rename(&tmp_path, path).map_err(|e| format!("rename: {}", e))?;

    eprintln!(
        "[BINARY-IDX] {}:\\ Migrated MTTIDX03 -> MTTIDX04 (records-first) in place",
        drive_letter
    );
    Ok(true)
}

fn copy_authenticated_file_section<W: Write>(
    source: &mut std::fs::File,
    offset: u64,
    mut len: usize,
    writer: &mut W,
    hmac: &mut integrity::HmacSha256,
    buffer: &mut [u8],
    label: &str,
) -> Result<(), String> {
    source
        .seek(SeekFrom::Start(offset))
        .map_err(|e| format!("seek v3 {}: {}", label, e))?;
    while len > 0 {
        let chunk_len = len.min(buffer.len());
        source
            .read_exact(&mut buffer[..chunk_len])
            .map_err(|e| format!("read v3 {}: {}", label, e))?;
        write_authenticated_chunk(writer, hmac, &buffer[..chunk_len])?;
        len -= chunk_len;
    }
    Ok(())
}

/// Open the current index file and memory-map its records section when it is
/// large enough to be worth mapping. Returns `Ok(None)` for small records
/// (kept owned) or a non-current file. Does not re-verify the HMAC: it is only
/// called on files this process just wrote in [`save_and_remap`].
fn map_records_region(drive_letter: char) -> Result<Option<(Mmap, usize)>, String> {
    let path = index_path(drive_letter);
    let file = open_index_read_stable(&path)?;
    let file_len = file
        .metadata()
        .map_err(|e| format!("index metadata: {}", e))?
        .len() as usize;
    if file_len < HEADER_SIZE + TRAILER_SIZE {
        return Ok(None);
    }

    let mut header_bytes = [0u8; HEADER_SIZE];
    {
        let mut reader = BufReader::new(&file);
        reader
            .read_exact(&mut header_bytes)
            .map_err(|e| format!("read header: {}", e))?;
    }
    if header_bytes[..8] != *MAGIC {
        return Ok(None);
    }
    let header: Header =
        unsafe { std::ptr::read_unaligned(header_bytes.as_ptr() as *const Header) };
    if header.version != VERSION {
        return Ok(None);
    }
    let record_count = header.record_count as usize;
    let records_bytes = match record_count.checked_mul(RECORD_SIZE) {
        Some(bytes) => bytes,
        None => return Ok(None),
    };
    if record_count == 0 || !mmap_records_enabled(records_bytes) {
        return Ok(None);
    }

    let mmap = unsafe {
        MmapOptions::new()
            .offset(HEADER_SIZE as u64)
            .len(records_bytes)
            .map(&file)
    }
    .map_err(|e| format!("map records: {}", e))?;
    Ok(Some((mmap, record_count)))
}

/// Save the index as MTTIDX04 and, for large volumes, swap its record store to
/// a memory mapping of the freshly written file.
///
/// Dropping the previous record store releases any prior mapping, so Windows
/// reclaims the superseded (unlinked) inode — keeping the on-disk footprint at
/// 1x while record bytes move from private heap into evictable, file-backed
/// page cache. Small volumes stay owned (mapping saves negligible RAM).
pub fn save_and_remap(index: &mut VolumeIndex) -> Result<(), String> {
    save(index)?;
    let drive_letter = index.drive_letter;
    match map_records_region(drive_letter) {
        Ok(Some((mmap, count))) => match RecordStore::from_mmap(mmap, count) {
            Ok(store) => index.records = store,
            Err(e) => {
                eprintln!(
                    "[BINARY-IDX] {}:\\ records remap skipped: {}",
                    drive_letter, e
                );
            }
        },
        Ok(None) => {
            // A mapped base can fall below the mapping threshold after mass
            // deletions (or mmap can be disabled at runtime). Replace it with
            // the freshly saved compact owned representation so the old map
            // and its potentially large tombstone set are released.
            if index.records.is_mapped() {
                let file = open_index_read_stable(&index_path(drive_letter))?;
                index.records = read_records_region_owned(&file, index.records.len())?;
            }
        }
        Err(e) => {
            eprintln!(
                "[BINARY-IDX] {}:\\ records remap failed: {}",
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

    #[test]
    fn arena_compression_round_trips() {
        // File names have lots of repeated extensions/patterns, so this should
        // compress well and exercise the threshold logic.
        let mut raw = Vec::new();
        for i in 0..100_000 {
            raw.extend_from_slice(format!("file_{}.txt\0", i).as_bytes());
        }

        let compressed =
            compress_arena(&raw, 100_000).expect("should compress large repetitive arena");
        assert!(compressed.len() < raw.len() / 2);

        let decompressed = decompress_arena(&compressed, ARENA_COMPRESSION_ZSTD, raw.len())
            .expect("should decompress");
        assert_eq!(decompressed, raw);
    }

    #[test]
    fn small_arena_is_not_compressed() {
        let raw = b"short unique names without repetition".to_vec();
        assert!(compress_arena(&raw, 10).is_none());
    }

    #[test]
    fn uncompressed_arena_decompresses_exactly() {
        let raw = b"hello world".to_vec();
        let decompressed = decompress_arena(&raw, ARENA_COMPRESSION_NONE, raw.len())
            .expect("should pass through raw bytes");
        assert_eq!(decompressed, raw);
    }

    #[test]
    fn decompress_detects_size_mismatch() {
        let raw = b"hello world".to_vec();
        assert!(decompress_arena(&raw, ARENA_COMPRESSION_NONE, 5).is_err());
    }

    use crate::file_index::VolumeIndex;

    fn sample_index(drive: char) -> VolumeIndex {
        let mut index = VolumeIndex::empty(drive);
        assert!(index.insert_record(10, "docs", 5, true, false));
        assert!(index.insert_record(20, "a.txt", 10, false, false));
        index.records.get_mut(&20).unwrap().size = 55;
        assert!(index.insert_record(30, "root.bin", 5, false, false));
        index.records.get_mut(&30).unwrap().size = 7;
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
        assert_eq!(index.records.get(&30).unwrap().size, 7);
        assert_eq!(
            index.resolve_path_to_frn(&format!(r"{}:\docs", drive)),
            Some(10)
        );
        let (total, files, folders, _zero) = index.folder_tree_summary(5);
        assert_eq!((total, files, folders), (62, 2, 1));
    }

    #[test]
    fn v4_round_trips_through_save_and_load() {
        crate::index_db::init_data_dir_for_tests();
        let drive = 'W';
        let path = index_path(drive);
        let _ = std::fs::remove_file(&path);

        let index = sample_index(drive);
        save(&index).expect("save v4");

        let (loaded, state) = load(drive).expect("load v4").expect("index present");
        assert_eq!(state.journal_id, 7);
        assert_eq!(state.last_usn, 8);
        assert!(state.has_hardlink_parent_data);
        assert!(state.has_reparse_point_data);
        assert_matches_sample(&loaded, drive);

        assert_eq!(&read_file_magic(&path).unwrap(), MAGIC);
        let _ = std::fs::remove_file(&path);
    }

    /// Serialize an index in the legacy MTTIDX03 (arena-first) layout so the
    /// migration path can be exercised end to end.
    fn write_v3_file(path: &Path, index: &VolumeIndex) -> Result<(), String> {
        let key = integrity::machine_key()?;
        let mut arena = Vec::new();
        index
            .names
            .for_each_slice(|slice| arena.extend_from_slice(slice));
        let arena_size = index.names.len() as u64;
        let hardlink_entry_count: u64 = index
            .hardlink_parents
            .values()
            .map(|v| v.len() as u64)
            .sum();

        let header = Header {
            magic: *LEGACY_V3_MAGIC,
            version: LEGACY_V3_VERSION,
            drive_letter: index.drive_letter as u8,
            arena_compression: ARENA_COMPRESSION_NONE,
            _pad: [0; 2],
            journal_id: index.journal_id,
            last_usn: index.last_usn,
            record_count: index.records.len() as u64,
            arena_size,
            hardlink_entry_count,
            reparse_count: index.reparse_points.len() as u64,
            flags: 1 | 2 | 4,
            compressed_arena_size: arena_size,
        };
        let header_bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(&header as *const Header as *const u8, HEADER_SIZE)
        };

        let file = std::fs::File::create(path).map_err(|e| format!("create v3: {}", e))?;
        let mut writer = BufWriter::new(file);
        let mut hmac = integrity::HmacSha256::new(&key).map_err(|e| format!("hmac: {}", e))?;

        // Legacy v3 order: header, arena, records, hardlinks, reparse.
        write_authenticated_chunk(&mut writer, &mut hmac, header_bytes)?;
        write_authenticated_chunk(&mut writer, &mut hmac, &arena)?;
        let mut frns: Vec<u64> = index.records.keys().copied().collect();
        frns.sort_unstable();
        for frn in frns {
            let rec = *index.records.get(&frn).unwrap();
            write_authenticated_chunk(&mut writer, &mut hmac, &frn.to_le_bytes())?;
            let rb: &[u8] = unsafe {
                std::slice::from_raw_parts(&rec as *const FileRecord as *const u8, FILE_RECORD_SIZE)
            };
            write_authenticated_chunk(&mut writer, &mut hmac, rb)?;
        }
        for (&child, parents) in &index.hardlink_parents {
            for &parent in parents {
                write_authenticated_chunk(&mut writer, &mut hmac, &child.to_le_bytes())?;
                write_authenticated_chunk(&mut writer, &mut hmac, &parent.to_le_bytes())?;
            }
        }
        let mut reparse: Vec<u64> = index.reparse_points.iter().copied().collect();
        reparse.sort_unstable();
        for frn in reparse {
            write_authenticated_chunk(&mut writer, &mut hmac, &frn.to_le_bytes())?;
        }
        let tag = hmac.finalize()?;
        writer
            .write_all(&tag)
            .map_err(|e| format!("trailer: {}", e))?;
        writer.flush().map_err(|e| format!("flush: {}", e))?;
        Ok(())
    }

    #[test]
    fn v3_file_migrates_to_v4_and_loads() {
        crate::index_db::init_data_dir_for_tests();
        let drive = 'V';
        let path = index_path(drive);
        let _ = std::fs::remove_file(&path);

        let index = sample_index(drive);
        write_v3_file(&path, &index).expect("write v3");
        assert_eq!(&read_file_magic(&path).unwrap(), LEGACY_V3_MAGIC);

        let (loaded, state) = load(drive).expect("load migrates").expect("index present");
        assert_eq!(state.journal_id, 7);
        assert_matches_sample(&loaded, drive);

        // The on-disk file is now reordered to MTTIDX04 and still loads.
        assert_eq!(&read_file_magic(&path).unwrap(), MAGIC);
        let (reloaded, _) = load(drive).expect("reload v4").expect("index present");
        assert_matches_sample(&reloaded, drive);

        let _ = std::fs::remove_file(&path);
    }
}
