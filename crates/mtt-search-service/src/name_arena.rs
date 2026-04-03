/// Contiguous byte arena for storing file name strings.
///
/// All file names are appended into a single `Vec<u8>` buffer and referenced
/// via compact `NameRef` handles (offset + length).  This eliminates per-string
/// heap allocation overhead (~16 bytes malloc header on Windows) and the three
/// machine-word `String` representation (ptr + len + cap = 24 bytes) per name.
///
/// Memory layout for 1.5M files with ~20-byte average name:
///   Arena buffer: ~30 MB  (contiguous, cache-friendly)
///   vs old layout: ~63 MB  (1.5M×(24 stack + 20 heap + 16 malloc header))
///
/// The arena is append-only.  Deletions leave "dead" bytes that are reclaimed
/// when the index is persisted and reloaded (the arena is rebuilt from the
/// surviving records).
/// Compact reference to a name stored in the arena.
/// Total size: 6 bytes (packed into 8 with alignment in CompactFileRecord).
#[derive(Clone, Copy, Debug)]
pub struct NameRef {
    /// Byte offset into the arena buffer.
    pub offset: u32,
    /// Byte length of the UTF-8 name.
    pub len: u16,
}

/// Contiguous arena holding all file name strings as raw UTF-8 bytes.
pub struct NameArena {
    buf: Vec<u8>,
}

impl NameArena {
    /// Create a new arena pre-allocated for `estimated_bytes` of name data.
    pub fn with_capacity(estimated_bytes: usize) -> Self {
        Self {
            buf: Vec::with_capacity(estimated_bytes),
        }
    }

    /// Append a name to the arena and return a compact reference.
    ///
    /// Returns `None` if the arena would exceed 4 GB (u32 offset overflow) or
    /// the individual name exceeds 65 535 bytes (u16 len overflow).  Both are
    /// unreachable in practice for NTFS file names (max 255 UTF-16 code units
    /// ≈ 1 020 bytes UTF-8) but returning `None` avoids crashing a system
    /// service if it ever happens.
    pub fn insert(&mut self, name: &str) -> Option<NameRef> {
        let offset = self.buf.len();
        if offset > u32::MAX as usize || name.len() > u16::MAX as usize {
            return None;
        }
        self.buf.extend_from_slice(name.as_bytes());
        Some(NameRef {
            offset: offset as u32,
            len: name.len() as u16,
        })
    }

    /// Retrieve a name by reference.
    #[inline]
    pub fn get(&self, r: NameRef) -> &str {
        let start = r.offset as usize;
        let end = start + r.len as usize;
        let bytes = &self.buf[start..end];
        // All data is inserted via `insert` which only accepts &str (valid UTF-8).
        // Use safe validation as defense-in-depth for a SYSTEM-level service.
        std::str::from_utf8(bytes).unwrap_or("")
    }

    /// Clear all names (for re-scan).
    pub fn clear(&mut self) {
        self.buf.clear();
    }

    /// Release excess capacity after the initial scan is complete.
    pub fn shrink_to_fit(&mut self) {
        self.buf.shrink_to_fit();
    }

    /// Total bytes used by name data.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// Total bytes allocated (may be larger than len() due to Vec growth).
    pub fn capacity(&self) -> usize {
        self.buf.capacity()
    }

    /// Get a raw pointer + len for warming (touching pages into RAM).
    /// Returns the underlying byte slice so callers can iterate over it.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }
}
