use memmap2::Mmap;

/// Logical byte arena for storing file name strings.
///
/// File names are appended into compact byte storage and referenced via
/// `NameRef` handles (offset + length).  This eliminates per-string heap
/// allocation overhead (~16 bytes malloc header on Windows) and the three
/// machine-word `String` representation (ptr + len + cap = 24 bytes) per name.
///
/// Memory layout for 1.5M files with ~20-byte average name:
///   Arena buffer: ~30 MB  (contiguous, cache-friendly)
///   vs old layout: ~63 MB  (1.5M×(24 stack + 20 heap + 16 malloc header))
///
/// The arena is append-only.  Binary-cache loads can map the stable base arena
/// directly from disk and append live USN changes into a small owned overlay.
/// Deletions leave "dead" bytes that are reclaimed when the index is persisted
/// and reloaded (the arena is rebuilt from the surviving records).
/// Compact reference to a name stored in the arena.
/// Total size: 6 bytes (packed into 8 with alignment in CompactFileRecord).
#[derive(Clone, Copy, Debug)]
pub struct NameRef {
    /// Byte offset into the arena buffer.
    pub offset: u32,
    /// Byte length of the UTF-8 name.
    pub len: u16,
}

enum NameStorage {
    Owned(Vec<u8>),
    Mapped { base: Mmap, append: Vec<u8> },
}

/// Arena holding all file name strings as raw UTF-8 bytes.
pub struct NameArena {
    storage: NameStorage,
}

impl NameArena {
    /// Create a new arena pre-allocated for `estimated_bytes` of name data.
    pub fn with_capacity(estimated_bytes: usize) -> Self {
        Self {
            storage: NameStorage::Owned(Vec::with_capacity(estimated_bytes)),
        }
    }

    /// Reconstruct an arena from owned raw bytes without an extra copy.
    pub fn from_vec(bytes: Vec<u8>) -> Self {
        Self {
            storage: NameStorage::Owned(bytes),
        }
    }

    /// Reconstruct an arena from a read-only file mapping.
    pub fn from_mmap(mmap: Mmap) -> Self {
        Self {
            storage: NameStorage::Mapped {
                base: mmap,
                append: Vec::new(),
            },
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
        let offset = self.len();
        let end = offset.checked_add(name.len())?;
        if end > u32::MAX as usize || name.len() > u16::MAX as usize {
            return None;
        }
        match &mut self.storage {
            NameStorage::Owned(buf) => buf.extend_from_slice(name.as_bytes()),
            NameStorage::Mapped { append, .. } => append.extend_from_slice(name.as_bytes()),
        }
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
        if end > self.len() {
            return "";
        }
        let bytes = match &self.storage {
            NameStorage::Owned(buf) => &buf[start..end],
            NameStorage::Mapped { base, append } => {
                if end <= base.len() {
                    &base[start..end]
                } else if start >= base.len() {
                    let append_start = start - base.len();
                    let append_end = end - base.len();
                    &append[append_start..append_end]
                } else {
                    return "";
                }
            }
        };
        // All data is inserted via `insert` which only accepts &str (valid UTF-8).
        // Use safe validation as defense-in-depth for a SYSTEM-level service.
        std::str::from_utf8(bytes).unwrap_or("")
    }

    /// Clear all names (for re-scan).
    pub fn clear(&mut self) {
        self.storage = NameStorage::Owned(Vec::new());
    }

    /// Release excess capacity after the initial scan is complete.
    pub fn shrink_to_fit(&mut self) {
        match &mut self.storage {
            NameStorage::Owned(buf) => buf.shrink_to_fit(),
            NameStorage::Mapped { append, .. } => append.shrink_to_fit(),
        }
    }

    /// Total bytes used by name data.
    pub fn len(&self) -> usize {
        match &self.storage {
            NameStorage::Owned(buf) => buf.len(),
            NameStorage::Mapped { base, append } => base.len() + append.len(),
        }
    }

    /// Total bytes allocated (may be larger than len() due to Vec growth).
    pub fn capacity(&self) -> usize {
        match &self.storage {
            NameStorage::Owned(buf) => buf.capacity(),
            NameStorage::Mapped { base, append } => base.len() + append.capacity(),
        }
    }

    /// Visit the logical arena slices in order.
    pub fn try_for_each_slice<E, F>(&self, mut visitor: F) -> Result<(), E>
    where
        F: FnMut(&[u8]) -> Result<(), E>,
    {
        match &self.storage {
            NameStorage::Owned(buf) => visitor(buf),
            NameStorage::Mapped { base, append } => {
                visitor(base)?;
                if !append.is_empty() {
                    visitor(append)?;
                }
                Ok(())
            }
        }
    }

    /// Visit the logical arena slices in order.
    pub fn for_each_slice<F>(&self, mut visitor: F)
    where
        F: FnMut(&[u8]),
    {
        let _ = self.try_for_each_slice::<(), _>(|slice| {
            visitor(slice);
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::NameArena;
    use memmap2::MmapOptions;
    use std::io::Write;

    #[test]
    fn mapped_arena_reads_base_and_appended_names() {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "mtt-name-arena-test-{}-{}.bin",
            std::process::id(),
            suffix
        ));

        {
            let mut file = std::fs::File::create(&path).unwrap();
            file.write_all(b"alphabeta").unwrap();
            file.sync_all().unwrap();
        }

        let file = std::fs::File::open(&path).unwrap();
        let mmap = unsafe { MmapOptions::new().len(9).map(&file).unwrap() };
        let mut arena = NameArena::from_mmap(mmap);

        assert_eq!(arena.get(super::NameRef { offset: 0, len: 5 }), "alpha");
        assert_eq!(arena.get(super::NameRef { offset: 5, len: 4 }), "beta");

        let gamma = arena.insert("gamma").unwrap();
        assert_eq!(gamma.offset, 9);
        assert_eq!(arena.get(gamma), "gamma");

        let mut slices = Vec::new();
        arena.for_each_slice(|slice| slices.push(slice.to_vec()));
        assert_eq!(slices, vec![b"alphabeta".to_vec(), b"gamma".to_vec()]);

        drop(file);
        let _ = std::fs::remove_file(path);
    }
}
