use std::io;
use std::path::Path;

/// Only an owned temporary file is cleaned up. Never remove a final destination
/// on an error: another process may have created it after preflight.
pub(crate) struct StagingFile(Option<tempfile::NamedTempFile>);

impl StagingFile {
    pub fn new(parent: &Path) -> io::Result<Self> {
        Ok(Self(Some(tempfile::NamedTempFile::new_in(parent)?)))
    }

    pub fn path(&self) -> &Path {
        self.0.as_ref().expect("Unpublished staging file").path()
    }

    pub fn publish(mut self, destination: &Path) -> io::Result<()> {
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows::core::PCWSTR;
            use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVE_FILE_FLAGS};
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
            // tempfile::persist_noclobber clears Windows attributes. Rename our
            // sibling directly, without REPLACE_EXISTING or cross-volume copy.
            unsafe {
                MoveFileExW(
                    PCWSTR(source.as_ptr()),
                    PCWSTR(destination.as_ptr()),
                    MOVE_FILE_FLAGS(0),
                )
            }
            .map_err(|error| io::Error::from_raw_os_error(error.code().0 & 0xFFFF))?;
            let mut file = self.0.take().expect("Unpublished staging file");
            file.disable_cleanup(true);
            Ok(())
        }
        #[cfg(not(windows))]
        match self
            .0
            .take()
            .expect("Unpublished staging file")
            .persist_noclobber(destination)
        {
            Ok(_) => Ok(()),
            Err(error) => {
                self.0 = Some(error.file);
                Err(error.error)
            }
        }
    }
}

impl Drop for StagingFile {
    fn drop(&mut self) {
        // CopyFileEx may have copied the source's read-only attribute. Windows
        // otherwise refuses to remove our staging file on publication failure.
        #[cfg(windows)]
        if let Some(file) = &self.0 {
            if let Ok(metadata) = file.as_file().metadata() {
                let mut permissions = metadata.permissions();
                if permissions.readonly() {
                    // Windows-only: unlike Unix this clears an attribute, not
                    // access-control permissions, on our owned staging file.
                    #[allow(clippy::permissions_set_readonly_false)]
                    permissions.set_readonly(false);
                    let _ = file.as_file().set_permissions(permissions);
                }
            }
        }
    }
}
