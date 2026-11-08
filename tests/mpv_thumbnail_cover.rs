//! Synthetic cover pixels through the real same-executable helper.
use mtt_file_manager::infrastructure::mpv_thumbnails::Extractor;
use std::{os::windows::fs::OpenOptionsExt, path::PathBuf, process::Command};

#[test]
#[ignore = "Requires FFmpeg, MTT_MPV_TEST_DIR, and the existing DLL beside the application executable"]
fn real_helper_prefers_embedded_jpeg_cover_and_releases_source() {
    let directory = tempfile::Builder::new()
        .prefix("helper-cover-")
        .tempdir_in(std::env::var_os("MTT_MPV_TEST_DIR").expect("Set MTT_MPV_TEST_DIR"))
        .unwrap();
    let image = directory.path().join("image.jpg");
    let ffmpeg = std::env::var_os("MTT_MPV_TEST_FFMPEG").unwrap_or_else(|| "ffmpeg".into());
    let status = Command::new(&ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-n",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:size=200x300",
            "-frames:v",
            "1",
        ])
        .arg(&image)
        .status()
        .unwrap();
    assert!(status.success());
    let mut extractor = Extractor::new(PathBuf::from(env!("CARGO_BIN_EXE_mtt-file-manager")));
    for name in ["cover.jpg", "cover.jpeg"] {
        let video = directory.path().join(format!("{name}.mkv"));
        let status = Command::new(&ffmpeg)
            .args([
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
                "-attach",
            ])
            .arg(&image)
            .args([
                "-metadata:s:t:0",
                "mimetype=image/jpeg",
                "-metadata:s:t:0",
                &format!("filename={name}"),
            ])
            .arg(&video)
            .status()
            .unwrap();
        assert!(status.success());
        let video = video.canonicalize().unwrap();
        for _ in 0..3 {
            let frame = extractor.extract(&video, 256, &|| false).unwrap();
            assert_eq!((frame.width, frame.height), (171, 256));
            assert_eq!(frame.rgba.len(), (frame.width * frame.height * 4) as usize);
            for pixel in frame.rgba.as_chunks::<4>().0 {
                assert!(pixel[0] > 200 && pixel[2] < 30 && pixel[3] == 255);
            }
            let _exclusive = std::fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(&video)
                .unwrap();
        }
    }
}
