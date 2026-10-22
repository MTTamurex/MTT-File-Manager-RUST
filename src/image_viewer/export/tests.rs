use super::*;

fn frame() -> DecodedFrame {
    DecodedFrame {
        rgba: vec![255, 0, 0, 255],
        width: 1,
        height: 1,
        original_width: 1,
        original_height: 1,
    }
}

#[test]
fn conversion_creates_and_replaces_png_and_pdf_atomically() {
    for (format, name) in [
        (ExportImageFormat::Png, "image.png"),
        (ExportImageFormat::Pdf, "image.pdf"),
    ] {
        for existing in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(name);
            if existing {
                std::fs::write(&path, b"original").unwrap();
            }
            let destination = ExportDestination::confirm(path.clone(), format).unwrap();
            assert_eq!(destination.write(frame()).unwrap(), path);
            let bytes = std::fs::read(&path).unwrap();
            if format == ExportImageFormat::Pdf {
                assert!(bytes.starts_with(b"%PDF-"));
            } else {
                assert_eq!(
                    image::open(&path).unwrap().to_rgba8().as_raw(),
                    &[255, 0, 0, 255]
                );
            }
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }
}

#[test]
fn encoding_failure_keeps_the_original_and_removes_only_its_own_temporary() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("image.pdf");
    let unrelated = dir.path().join("unrelated.tmp");
    std::fs::write(&path, b"original").unwrap();
    std::fs::write(&unrelated, b"keep").unwrap();
    let destination = ExportDestination::confirm(path.clone(), ExportImageFormat::Pdf).unwrap();
    let mut invalid = frame();
    invalid.rgba.clear();
    assert!(destination.write(invalid).is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"original");
    assert_eq!(std::fs::read(unrelated).unwrap(), b"keep");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn destination_changed_or_created_after_confirmation_is_not_overwritten() {
    for existing in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image.png");
        if existing {
            std::fs::write(&path, b"original").unwrap();
        }
        let destination = ExportDestination::confirm(path.clone(), ExportImageFormat::Png).unwrap();
        std::fs::write(&path, b"newer contents").unwrap();
        assert!(destination.write(frame()).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"newer contents");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}

#[test]
fn extension_normalization_never_implicitly_confirms_an_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("image.pdf");
    std::fs::write(&path, b"original").unwrap();
    assert!(
        ExportDestination::confirm(dir.path().join("image.png"), ExportImageFormat::Pdf).is_err()
    );
    assert_eq!(std::fs::read(path).unwrap(), b"original");
}

#[cfg(target_os = "windows")]
#[test]
fn publication_failure_preserves_destination_and_cleans_temporary() {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::FILE_SHARE_READ;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("image.png");
    std::fs::write(&path, b"original").unwrap();
    let destination = ExportDestination::confirm(path.clone(), ExportImageFormat::Png).unwrap();
    // Deny deletion/replacement while still allowing snapshot verification.
    let _guard = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0)
        .open(&path)
        .unwrap();
    assert!(destination.write(frame()).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
