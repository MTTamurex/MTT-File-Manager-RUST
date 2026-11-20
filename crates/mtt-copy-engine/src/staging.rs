use std::io;
use std::path::Path;

/// Only an owned temporary file is cleaned up. Never remove a final destination
/// on an error: another process may have created it after preflight.
pub(crate) struct StagingFile {
    #[cfg(windows)]
    path: Option<tempfile::TempPath>,
    #[cfg(not(windows))]
    file: Option<tempfile::NamedTempFile>,
}

impl StagingFile {
    pub fn new(parent: &Path) -> io::Result<Self> {
        let file = tempfile::NamedTempFile::new_in(parent)?;
        #[cfg(windows)]
        {
            // Close the writable handle so move finalization can acquire a
            // handle that denies writes while still allowing the staged rename.
            Ok(Self {
                path: Some(file.into_temp_path()),
            })
        }
        #[cfg(not(windows))]
        {
            Ok(Self { file: Some(file) })
        }
    }

    pub fn path(&self) -> &Path {
        #[cfg(windows)]
        {
            self.path
                .as_ref()
                .expect("Unpublished staging file")
                .as_ref()
        }
        #[cfg(not(windows))]
        {
            self.file.as_ref().expect("Unpublished staging file").path()
        }
    }

    pub fn publish(mut self, destination: &Path) -> io::Result<()> {
        self.publish_impl(destination, false)
    }

    pub fn publish_with_replacement(mut self, destination: &Path, replace: bool) -> io::Result<()> {
        self.publish_impl(destination, replace)
    }

    fn publish_impl(&mut self, destination: &Path, replace: bool) -> io::Result<()> {
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows::core::PCWSTR;
            use windows::Win32::Storage::FileSystem::{
                MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVE_FILE_FLAGS,
            };
            let source: Vec<u16> = self
                .path()
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect();
            let destination: Vec<u16> = destination
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect();
            // Same-volume publication only. Replacement is possible solely for
            // a confirmed, revalidated file, after staging verification.
            unsafe {
                MoveFileExW(
                    PCWSTR(source.as_ptr()),
                    PCWSTR(destination.as_ptr()),
                    if replace {
                        MOVEFILE_REPLACE_EXISTING
                    } else {
                        MOVE_FILE_FLAGS(0)
                    },
                )
            }
            .map_err(|error| io::Error::from_raw_os_error(error.code().0 & 0xFFFF))?;
            self.disarm_cleanup();
            Ok(())
        }
        #[cfg(not(windows))]
        if replace {
            std::fs::rename(self.path(), destination)?;
            let mut file = self.file.take().expect("Unpublished staging file");
            file.disable_cleanup(true);
            return Ok(());
        }
        #[cfg(not(windows))]
        match self
            .file
            .take()
            .expect("Unpublished staging file")
            .persist_noclobber(destination)
        {
            Ok(_) => Ok(()),
            Err(error) => {
                self.file = Some(error.file);
                Err(error.error)
            }
        }
    }

    #[cfg(windows)]
    fn disarm_cleanup(&mut self) {
        let mut path = self.path.take().expect("Unpublished staging file");
        // The old name no longer belongs to us after publication. Another
        // process may immediately reuse it; never clean it up on drop.
        path.disable_cleanup(true);
    }
}

impl Drop for StagingFile {
    fn drop(&mut self) {
        // CopyFileEx may have copied the source's read-only attribute. Windows
        // otherwise refuses to remove our staging file on publication failure.
        #[cfg(windows)]
        if let Some(path) = &self.path {
            if let Ok(metadata) = std::fs::metadata(path) {
                let mut permissions = metadata.permissions();
                if permissions.readonly() {
                    // Windows-only: unlike Unix this clears an attribute, not
                    // access-control permissions, on our owned staging file.
                    #[allow(clippy::permissions_set_readonly_false)]
                    permissions.set_readonly(false);
                    let _ = std::fs::set_permissions(path, permissions);
                }
            }
        }
    }
}

#[cfg(all(test, windows))]
mod tests;
