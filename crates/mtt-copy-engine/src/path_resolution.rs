//! WinFsp local mounts may support files but not DOS-volume name resolution.
use std::io;
use std::path::{Path, PathBuf};

#[cfg(all(test, windows))]
mod tests;
#[cfg(windows)]
mod windows;

pub(crate) struct Resolved {
    pub path: PathBuf,
    pub virtual_volume: bool,
}

pub(crate) fn resolve(path: &Path) -> io::Result<Resolved> {
    resolve_with(path, |path| std::fs::canonicalize(path))
}

fn resolve_with(
    path: &Path,
    canonicalize: impl FnOnce(&Path) -> io::Result<PathBuf>,
) -> io::Result<Resolved> {
    match canonicalize(path) {
        Ok(path) => Ok(Resolved {
            path,
            virtual_volume: false,
        }),
        #[cfg(windows)]
        Err(error) if error.raw_os_error() == Some(1005) => {
            // Only ERROR_UNRECOGNIZED_VOLUME takes this path. Do not hide missing
            // files, access failures or unsupported links with lexical fallback.
            crate::plan::check_path(path)?;
            let path = windows::volume_relative_path(path)?;
            crate::plan::check_path(&path)?;
            Ok(Resolved {
                path,
                virtual_volume: true,
            })
        }
        Err(error) => Err(error),
    }
}

/// Canonical DOS names cannot expose aliases of a volume without mount-manager
/// support. Compare directory identities as well before allowing tree copies.
pub(crate) fn contains_directory(source: &Path, destination: &Path) -> io::Result<bool> {
    #[cfg(windows)]
    {
        windows::contains_directory(source, destination)
    }
    #[cfg(not(windows))]
    {
        Ok(destination.starts_with(source))
    }
}
