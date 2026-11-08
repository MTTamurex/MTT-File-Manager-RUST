//! Opt-in tests of the REAL same-executable helper, without launching the GUI.
use mtt_file_manager::{
    domain::thumbnail::is_valid_rgba_buffer,
    infrastructure::mpv_thumbnails::{Error, Extractor, Failure},
};
use std::{io::Write, os::windows::fs::OpenOptionsExt, path::PathBuf};

fn fixture() -> PathBuf {
    PathBuf::from(
        std::env::var_os("MTT_MPV_TEST_VIDEO")
            .expect("Set MTT_MPV_TEST_VIDEO to a local generic test fixture"),
    )
    .canonicalize()
    .expect("Resolve the test fixture")
}

fn extractor() -> Extractor {
    Extractor::new(PathBuf::from(env!("CARGO_BIN_EXE_mtt-file-manager")))
}

#[test]
#[ignore = "Requires MTT_MPV_TEST_VIDEO and the existing libmpv DLL adjacent to the application executable"]
fn real_helper_reuses_session_and_releases_source_after_every_frame() {
    let path = fixture();
    let mut extractor = extractor();
    for _ in 0..10 {
        let frame = extractor.extract(&path, 256, &|| false).unwrap();
        assert!(is_valid_rgba_buffer(
            frame.width,
            frame.height,
            256,
            frame.rgba.len()
        ));
        let _exclusive = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .expect("Source must be released while the helper remains alive");
    }
}

#[test]
#[ignore = "Requires MTT_MPV_TEST_VIDEO and the existing libmpv runtime"]
fn real_helper_recovers_after_corrupted_media() {
    let path = fixture();
    let mut corrupt = tempfile::Builder::new().suffix(".mp4").tempfile().unwrap();
    corrupt.write_all(&[0; 64]).unwrap();
    corrupt.flush().unwrap();
    let corrupt_path = corrupt.path().canonicalize().unwrap();
    let _path_guard = corrupt.into_temp_path(); // close writer; retain fixture cleanup ownership
    let mut extractor = extractor();
    let error = extractor
        .extract(&corrupt_path, 256, &|| false)
        .unwrap_err();
    assert_eq!(error, Error::Helper(Failure::Decode));
    assert!(extractor.extract(&path, 256, &|| false).is_ok());
}

#[test]
#[ignore = "Requires MTT_MPV_TEST_VIDEO and the existing libmpv runtime"]
fn independent_helpers_can_decode_the_same_file_concurrently() {
    let path = fixture();
    std::thread::scope(|scope| {
        let tasks: Vec<_> = (0..2)
            .map(|_| scope.spawn(|| extractor().extract(&path, 256, &|| false).unwrap()))
            .collect();
        for task in tasks {
            assert!(!task.join().unwrap().rgba.is_empty());
        }
    });
}
