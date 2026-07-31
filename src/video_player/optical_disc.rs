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
    pub is_mounted_iso: bool,
}

impl OpticalDisc {
    /// Mounted CDFS DVDs need directory-tree access, while physical DVDs need
    /// raw-device access so libdvdcss can authenticate protected media.
    pub fn mpv_device_path(&self) -> String {
        match self.kind {
            OpticalDiscKind::Dvd if self.is_mounted_iso => {
                self.root.join("VIDEO_TS").to_string_lossy().to_string()
            }
            OpticalDiscKind::Dvd => self
                .root
                .to_string_lossy()
                .trim_end_matches(['\\', '/'])
                .to_string(),
            OpticalDiscKind::BluRay => self.root.to_string_lossy().to_string(),
        }
    }
}

pub(super) fn event_error_is_fatal(file_loaded: bool, error: &mpv::Error) -> bool {
    !file_loaded
        || matches!(
            error,
            mpv::Error::Raw(code) if *code == mpv::mpv_error::LoadingFailed
        )
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
    let kind = detect_disc_layout(&root)?;
    let is_mounted_iso = kind == OpticalDiscKind::Dvd
        && crate::infrastructure::windows::iso_mount::backing_iso_for_drive(
            root.to_string_lossy().as_ref(),
        )
        .is_some();

    Ok(OpticalDisc {
        root,
        kind,
        is_mounted_iso,
    })
}

fn detect_disc_layout(root: &Path) -> Result<OpticalDiscKind, String> {
    if root.join("BDMV").join("index.bdmv").is_file() {
        return Ok(OpticalDiscKind::BluRay);
    }
    if root.join("VIDEO_TS").join("VIDEO_TS.IFO").is_file() {
        return Ok(OpticalDiscKind::Dvd);
    }

    Err(rust_i18n::t!("video.optical_no_layout").to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        detect_disc_layout, event_error_is_fatal, normalize_optical_drive_root, OpticalDisc,
        OpticalDiscKind,
    };
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
        std::fs::write(dvd.path().join("VIDEO_TS").join("VIDEO_TS.IFO"), []).unwrap();
        assert_eq!(detect_disc_layout(dvd.path()), Ok(OpticalDiscKind::Dvd));

        let bluray = tempfile::tempdir().unwrap();
        std::fs::create_dir(bluray.path().join("BDMV")).unwrap();
        std::fs::write(bluray.path().join("BDMV").join("index.bdmv"), []).unwrap();
        assert_eq!(
            detect_disc_layout(bluray.path()),
            Ok(OpticalDiscKind::BluRay)
        );
    }

    #[test]
    fn prioritizes_bluray_and_rejects_data_discs() {
        let mixed = tempfile::tempdir().unwrap();
        std::fs::create_dir(mixed.path().join("VIDEO_TS")).unwrap();
        std::fs::write(mixed.path().join("VIDEO_TS").join("VIDEO_TS.IFO"), []).unwrap();
        std::fs::create_dir(mixed.path().join("BDMV")).unwrap();
        std::fs::write(mixed.path().join("BDMV").join("index.bdmv"), []).unwrap();
        assert_eq!(
            detect_disc_layout(mixed.path()),
            Ok(OpticalDiscKind::BluRay)
        );

        let data = tempfile::tempdir().unwrap();
        assert!(detect_disc_layout(data.path()).is_err());

        std::fs::create_dir(data.path().join("VIDEO_TS")).unwrap();
        assert!(detect_disc_layout(data.path()).is_err());
    }

    #[test]
    fn formats_physical_and_mounted_disc_devices_for_mpv() {
        let mounted_dvd = OpticalDisc {
            root: PathBuf::from("I:\\"),
            kind: OpticalDiscKind::Dvd,
            is_mounted_iso: true,
        };
        let physical_dvd = OpticalDisc {
            root: PathBuf::from("J:\\"),
            kind: OpticalDiscKind::Dvd,
            is_mounted_iso: false,
        };
        let bluray = OpticalDisc {
            root: PathBuf::from("K:\\"),
            kind: OpticalDiscKind::BluRay,
            is_mounted_iso: true,
        };

        assert_eq!(mounted_dvd.mpv_device_path(), "I:\\VIDEO_TS");
        assert_eq!(physical_dvd.mpv_device_path(), "J:");
        assert_eq!(bluray.mpv_device_path(), "K:\\");
    }

    #[test]
    fn treats_all_preload_errors_and_postload_loading_failures_as_fatal() {
        let unknown_format = mpv::Error::Raw(mpv::mpv_error::UnknownFormat);
        let property_error = mpv::Error::Raw(mpv::mpv_error::PropertyError);
        let loading_failed = mpv::Error::Raw(mpv::mpv_error::LoadingFailed);

        assert!(event_error_is_fatal(false, &unknown_format));
        assert!(event_error_is_fatal(false, &property_error));
        assert!(event_error_is_fatal(true, &loading_failed));
        assert!(!event_error_is_fatal(true, &property_error));
    }
}
