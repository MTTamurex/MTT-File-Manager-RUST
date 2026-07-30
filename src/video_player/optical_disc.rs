use std::path::{Path, PathBuf};

use crate::infrastructure::windows::{detect_drive_type, normalize_drive_root_path, DriveType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OpticalDiscKind {
    Dvd,
    BluRay,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OpticalDisc {
    pub root: PathBuf,
    pub kind: OpticalDiscKind,
}

impl OpticalDisc {
    /// A mounted CDFS DVD must be opened as a directory tree. Opening `I:` as
    /// a raw device makes libdvdread look only for UDF metadata, which is not
    /// present in some valid DVD ISO images.
    pub fn mpv_device_path(&self) -> String {
        match self.kind {
            OpticalDiscKind::Dvd => self.root.join("VIDEO_TS").to_string_lossy().to_string(),
            OpticalDiscKind::BluRay => self.root.to_string_lossy().to_string(),
        }
    }
}

fn normalize_optical_drive_root(path: &Path) -> Result<PathBuf, String> {
    normalize_drive_root_path(path)
        .map(PathBuf::from)
        .ok_or_else(|| rust_i18n::t!("video.optical_invalid_root").to_string())
}

/// Validate only cheap drive metadata. Disc layout probing is deferred to the
/// player subprocess because waking an optical drive can block for seconds.
pub(super) fn validate_optical_drive(path: &Path) -> Result<PathBuf, String> {
    let root = normalize_optical_drive_root(path)?;
    let root_text = root.to_string_lossy();
    if detect_drive_type(root_text.as_ref()) != DriveType::Cdrom {
        return Err(rust_i18n::t!("video.optical_not_drive").to_string());
    }

    Ok(root)
}

pub(super) fn detect_optical_disc(path: &Path) -> Result<OpticalDisc, String> {
    let root = validate_optical_drive(path)?;
    detect_disc_layout(&root).map(|kind| OpticalDisc { root, kind })
}

fn detect_disc_layout(root: &Path) -> Result<OpticalDiscKind, String> {
    if root.join("BDMV").is_dir() {
        return Ok(OpticalDiscKind::BluRay);
    }
    if root.join("VIDEO_TS").is_dir() {
        return Ok(OpticalDiscKind::Dvd);
    }

    Err(rust_i18n::t!("video.optical_no_layout").to_string())
}

#[cfg(test)]
mod tests {
    use super::{detect_disc_layout, normalize_optical_drive_root, OpticalDisc, OpticalDiscKind};
    use std::path::{Path, PathBuf};

    #[test]
    fn normalizes_only_windows_drive_roots() {
        assert_eq!(
            normalize_optical_drive_root(Path::new("d:")).unwrap(),
            PathBuf::from("D:\\")
        );
        assert_eq!(
            normalize_optical_drive_root(Path::new("e:/")).unwrap(),
            PathBuf::from("E:\\")
        );
        assert!(normalize_optical_drive_root(Path::new("D:\\VIDEO_TS")).is_err());
        assert!(normalize_optical_drive_root(Path::new("\\\\server\\disc")).is_err());
        assert!(normalize_optical_drive_root(Path::new("dvd://")).is_err());
    }

    #[test]
    fn detects_dvd_and_bluray_layouts() {
        let dvd = tempfile::tempdir().unwrap();
        std::fs::create_dir(dvd.path().join("VIDEO_TS")).unwrap();
        assert_eq!(detect_disc_layout(dvd.path()), Ok(OpticalDiscKind::Dvd));

        let bluray = tempfile::tempdir().unwrap();
        std::fs::create_dir(bluray.path().join("BDMV")).unwrap();
        assert_eq!(
            detect_disc_layout(bluray.path()),
            Ok(OpticalDiscKind::BluRay)
        );
    }

    #[test]
    fn prioritizes_bluray_and_rejects_data_discs() {
        let mixed = tempfile::tempdir().unwrap();
        std::fs::create_dir(mixed.path().join("VIDEO_TS")).unwrap();
        std::fs::create_dir(mixed.path().join("BDMV")).unwrap();
        assert_eq!(
            detect_disc_layout(mixed.path()),
            Ok(OpticalDiscKind::BluRay)
        );

        let data = tempfile::tempdir().unwrap();
        assert!(detect_disc_layout(data.path()).is_err());
    }

    #[test]
    fn formats_mounted_disc_directories_for_mpv() {
        let disc = OpticalDisc {
            root: PathBuf::from("I:\\"),
            kind: OpticalDiscKind::Dvd,
        };
        let bluray = OpticalDisc {
            root: PathBuf::from("J:\\"),
            kind: OpticalDiscKind::BluRay,
        };

        assert_eq!(disc.mpv_device_path(), "I:\\VIDEO_TS");
        assert_eq!(bluray.mpv_device_path(), "J:\\");
    }
}
