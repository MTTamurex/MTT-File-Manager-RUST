//! Native cover fixtures contain only generated media and generic attachment names.
use super::*;
use std::{
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

fn directory() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("cover-test-")
        .tempdir_in(std::env::var_os("MTT_MPV_TEST_DIR").expect("Set MTT_MPV_TEST_DIR"))
        .unwrap()
}

fn fixture(directory: &Path, name: Option<&str>, color: &str, corrupt: bool) -> PathBuf {
    let attachment = directory.join("image.jpg");
    if name.is_some() {
        if corrupt {
            std::fs::write(&attachment, b"invalid jpeg").unwrap();
        } else {
            let status = Command::new(
                std::env::var_os("MTT_MPV_TEST_FFMPEG").unwrap_or_else(|| "ffmpeg".into()),
            )
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-n",
                "-f",
                "lavfi",
                "-i",
                &format!("color=c={color}:size=200x300"),
                "-frames:v",
                "1",
            ])
            .arg(&attachment)
            .status()
            .unwrap();
            assert!(status.success());
        }
    }
    let path = directory.join("video.mkv");
    let mut command =
        Command::new(std::env::var_os("MTT_MPV_TEST_FFMPEG").unwrap_or_else(|| "ffmpeg".into()));
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-n",
        "-f",
        "lavfi",
        "-i",
        "color=c=blue:size=320x180:rate=24:duration=2",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
    ]);
    if let Some(name) = name {
        command.arg("-attach").arg(&attachment).args([
            "-metadata:s:t:0",
            "mimetype=image/jpeg",
            "-metadata:s:t:0",
            &format!("filename={name}"),
        ]);
    }
    let status = command.arg(&path).status().unwrap();
    assert!(status.success());
    path
}

#[test]
#[ignore = "Requires the existing DLL, FFmpeg fixture generator, and a test directory"]
fn embedded_cover_precedes_video_and_preserves_explicit_and_legacy() {
    let directory = directory();
    let path = fixture(directory.path(), Some("cover.jpg"), "red", false);
    let api = Api::load(&PathBuf::from(
        std::env::var_os("MTT_MPV_TEST_DLL").expect("Set MTT_MPV_TEST_DLL"),
    ))
    .unwrap();
    let mut session = Session::new(&api).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    session.load_tracks(&path, deadline).unwrap();
    assert!(
        session.property("video-codec").is_err(),
        "Discovery must not initialize mpv's main video decoder"
    );
    assert!(session
        .property("track-list")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .all(|track| track["selected"] != true));
    session.stop(deadline).unwrap();
    let result = session
        .capture(&path, None, 256, Duration::from_secs(10))
        .unwrap();
    assert_eq!(result.details["source"], "embedded-cover");
    assert_eq!(result.image.dimensions(), (171, 256));
    let pixel = result.image.get_pixel(80, 128);
    assert!(pixel[0] > 200 && pixel[2] < 30);
    let explicit = session
        .capture(&path, Some(0.5), 256, Duration::from_secs(10))
        .unwrap();
    assert!(explicit.image.get_pixel(100, 100)[2] > 200);
    let legacy = session
        .capture_legacy(&path, 256, Duration::from_secs(10))
        .unwrap();
    assert!(legacy.image.get_pixel(100, 100)[2] > 200);
    let _exclusive = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
}

#[test]
#[ignore = "Requires the existing DLL, FFmpeg fixture generator, and a test directory"]
fn jpeg_name_and_blank_covers_are_preserved_across_session_reuse() {
    let api = Api::load(&PathBuf::from(
        std::env::var_os("MTT_MPV_TEST_DLL").expect("Set MTT_MPV_TEST_DLL"),
    ))
    .unwrap();
    let mut session = Session::new(&api).unwrap();
    for (name, color) in [
        ("cover.jpeg", "red"),
        ("cover.jpg", "white"),
        ("cover.jpeg", "black"),
    ] {
        let directory = directory();
        let path = fixture(directory.path(), Some(name), color, false);
        let result = session
            .capture(&path, None, 256, Duration::from_secs(10))
            .unwrap();
        assert_eq!(result.details["source"], "embedded-cover");
        assert_eq!(result.image.dimensions(), (171, 256));
        if color == "red" {
            assert!(result.image.get_pixel(80, 128)[0] > 200);
        } else {
            assert!(
                quality::assess(&result.image).unsuitable,
                "Blank covers must not be replaced by video frames"
            );
        }
        let _exclusive = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
    }
}

#[test]
#[ignore = "Requires the existing DLL, FFmpeg fixture generator, and a test directory"]
fn missing_wrong_name_and_corrupt_covers_fall_back_to_video_and_release_source() {
    let api = Api::load(&PathBuf::from(
        std::env::var_os("MTT_MPV_TEST_DLL").expect("Set MTT_MPV_TEST_DLL"),
    ))
    .unwrap();
    let mut session = Session::new(&api).unwrap();
    for (name, corrupt) in [
        (None, false),
        (Some("other.jpg"), false),
        (Some("cover.png"), false),
        (Some("cover.jpg"), true),
        (Some("cover.jpeg"), false),
    ] {
        let directory = directory();
        let path = fixture(directory.path(), name, "red", corrupt);
        let result = session
            .capture(&path, None, 256, Duration::from_secs(6))
            .unwrap();
        let valid = name == Some("cover.jpeg");
        assert_eq!(
            result.details["source"],
            if valid {
                "embedded-cover"
            } else {
                "video-frame"
            },
            "Synthetic attachment: {name:?}, corrupt: {corrupt}"
        );
        let pixel = result.image.get_pixel(80, 128);
        assert!(if valid {
            pixel[0] > 200 && pixel[2] < 30
        } else {
            pixel[2] > 200 && pixel[0] < 30
        });
        let _exclusive = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
    }
}

#[test]
#[ignore = "Requires the existing DLL, FFmpeg fixture generator, and a test directory"]
fn expired_cover_probe_can_be_stopped_before_reusing_the_session() {
    let directory = directory();
    let path = fixture(directory.path(), Some("cover.jpg"), "red", false);
    let api = Api::load(&PathBuf::from(
        std::env::var_os("MTT_MPV_TEST_DLL").expect("Set MTT_MPV_TEST_DLL"),
    ))
    .unwrap();
    let mut session = Session::new(&api).unwrap();
    for _ in 0..3 {
        // Queue a load but expire before FILE_LOADED; await END_FILE only if started.
        assert!(session
            .embedded_cover(&path, 256, std::time::Instant::now())
            .is_err());
        let result = session
            .capture(&path, None, 256, Duration::from_secs(10))
            .unwrap();
        assert_eq!(result.details["source"], "embedded-cover");
        assert!(result.image.get_pixel(80, 128)[0] > 200);
        let _exclusive = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
    }
}
