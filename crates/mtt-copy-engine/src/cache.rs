//! Buffered production copying plus a test-only unbuffered comparison mode.
#[derive(Clone, Copy)]
pub(crate) enum CacheMode {
    Buffered,
    #[cfg(all(test, windows))]
    Unbuffered,
}

impl CacheMode {
    #[cfg(windows)]
    pub(crate) fn flags(self) -> windows::Win32::Storage::FileSystem::COPYFILE_FLAGS {
        use windows::Win32::Storage::FileSystem::COPYFILE_FLAGS;
        match self {
            Self::Buffered => COPYFILE_FLAGS(0),
            #[cfg(test)]
            Self::Unbuffered => windows::Win32::Storage::FileSystem::COPY_FILE_NO_BUFFERING,
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use windows::Win32::Storage::FileSystem::{COPYFILE_FLAGS, COPY_FILE_NO_BUFFERING};

    #[test]
    fn production_mode_uses_windows_cache_without_a_file_size_threshold() {
        // Selection no longer takes a size, so large files cannot silently
        // switch modes. Buffered leaves Windows in charge of cache/buffer sizes.
        assert_eq!(CacheMode::Buffered.flags(), COPYFILE_FLAGS(0));
    }

    #[test]
    fn test_only_unbuffered_mode_remains_available_for_comparisons() {
        assert_eq!(CacheMode::Unbuffered.flags(), COPY_FILE_NO_BUFFERING);
    }
}
