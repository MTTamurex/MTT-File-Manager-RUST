//! Opt-in, synthetic media only. No Windows thumbnail provider is involved.
use super::*;
use image::RgbaImage;
use std::{
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

fn runtime() -> Api {
    Api::load(&PathBuf::from(
        std::env::var_os("MTT_MPV_TEST_DLL").expect("Set MTT_MPV_TEST_DLL"),
    ))
    .unwrap()
}

fn directory() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("thumbnail-test-")
        .tempdir_in(
            std::env::var_os("MTT_MPV_TEST_DIR")
                .expect("Set MTT_MPV_TEST_DIR to an existing test directory"),
        )
        .unwrap()
}

fn generate(path: &Path, source: &str, filter: Option<&str>, gop: &str, extra: &[&str]) {
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
        source,
    ]);
    if let Some(filter) = filter {
        command.args(["-vf", filter]);
    }
    let status = command
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-g",
            gop,
            "-pix_fmt",
            "yuv420p",
        ])
        .args(extra)
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "Synthetic fixture generation failed");
}

fn capture(session: &mut Session<'_>, path: &Path) -> Capture {
    let capture = session
        .capture(path, None, 256, Duration::from_secs(10))
        .unwrap();
    let _exclusive = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
        .unwrap();
    assert!(capture.details["raw_width"].as_u64().unwrap() <= 256);
    assert!(capture.details["raw_height"].as_u64().unwrap() <= 256);
    capture
}

#[test]
#[ignore = "Requires the existing DLL, FFmpeg fixture generator, and a test directory"]
fn retries_blank_first_candidate_and_preserves_explicit_requests() {
    let directory = directory();
    let api = runtime();
    let mut session = Session::new(&api).unwrap();
    for color in ["black", "white"] {
        let path = directory.path().join(format!("{color}.mp4"));
        let target = selection::candidates(Some(6.0), selection::seed(&path))[0];
        let filter = format!(
            "drawbox=x=0:y=0:w=iw:h=ih:color={color}:t=fill:enable='between(t,{},{})'",
            target - 0.25,
            target + 0.25
        );
        generate(
            &path,
            "testsrc2=size=640x360:rate=24:duration=6",
            Some(&filter),
            "1",
            &[],
        );
        let first = session
            .capture(&path, Some(target), 256, Duration::from_secs(10))
            .unwrap();
        assert!(quality::assess(&first.image).unsuitable);
        assert_eq!(first.details["attempts"], 1);
        let selected = capture(&mut session, &path);
        assert_eq!(selected.details["attempts"], 2);
        assert_eq!(selected.details["selected_attempt"], 2);
        assert!(!quality::assess(&selected.image).unsuitable);
        assert_eq!(selected.details["quality_fallback"], false);
    }
}

#[test]
#[ignore = "Requires the existing DLL, FFmpeg fixture generator, and a test directory"]
fn all_blank_and_short_clips_stop_after_three_candidates() {
    let directory = directory();
    let api = runtime();
    let mut session = Session::new(&api).unwrap();
    for (name, source) in [
        ("black.mp4", "color=c=black:size=320x180:rate=24:duration=3"),
        (
            "white.mp4",
            "color=c=white:size=320x180:rate=24:duration=0.2",
        ),
    ] {
        let path = directory.path().join(name);
        generate(&path, source, None, "1", &[]);
        let selected = capture(&mut session, &path);
        assert_eq!(selected.details["attempts"], 3);
        assert_eq!(selected.details["quality_fallback"], true);
        let score = selected.details["quality"]["score"].as_f64().unwrap();
        assert!(selected.details["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .all(|candidate| candidate["quality"]["score"].as_f64().unwrap() <= score));
    }
}

#[test]
#[ignore = "Requires the existing DLL, FFmpeg fixture generator, and a test directory"]
fn dispersion_is_stable_and_long_gop_cannot_return_the_opening() {
    let directory = directory();
    let api = runtime();
    let mut session = Session::new(&api).unwrap();
    let mut times = Vec::new();
    for index in 0..3 {
        let path = directory.path().join(format!("episode{index}.mp4"));
        generate(
            &path,
            "testsrc2=size=320x180:rate=24:duration=6",
            Some("drawbox=x=0:y=0:w=iw:h=ih:color=black:t=fill:enable='lt(t,1.5)'"),
            "240",
            &["-sc_threshold", "0"],
        );
        let first = capture(&mut session, &path);
        let again = capture(&mut session, &path);
        assert_eq!(
            first.details["requested_seconds"],
            again.details["requested_seconds"]
        );
        assert!((1.5..=4.5).contains(&first.details["actual_seconds"].as_f64().unwrap()));
        assert!(!quality::assess(&first.image).unsuitable);
        assert_eq!(first.details["candidates"][0]["keyframe_corrected"], true);
        times.push(first.details["requested_seconds"].as_f64().unwrap());
    }
    assert!(times.windows(2).any(|pair| pair[0] != pair[1]));
}

#[test]
#[ignore = "Requires the existing DLL, FFmpeg fixture generator, and a test directory"]
fn native_scale_preserves_anamorphic_aspect_and_legacy_can_be_restored() {
    let directory = directory();
    let path = directory.path().join("anamorphic.mp4");
    generate(
        &path,
        "testsrc2=size=640x360:rate=24:duration=2",
        Some("setsar=2"),
        "1",
        &[],
    );
    let api = runtime();
    let mut session = Session::new(&api).unwrap();
    for _ in 0..2 {
        let legacy = session
            .capture_legacy(&path, 256, Duration::from_secs(10))
            .unwrap();
        assert!(legacy.details["raw_width"].as_u64().unwrap() > 256);
        let selected = capture(&mut session, &path);
        assert_eq!((selected.image.width(), selected.image.height()), (256, 72));
        assert!(!quality::assess(&selected.image).unsuitable);
    }
}

#[test]
#[ignore = "Requires the existing DLL, FFmpeg fixture generator, and a test directory"]
fn unknown_duration_is_bounded_and_returns_an_available_frame() {
    let directory = directory();
    let path = directory.path().join("clip.h264");
    generate(
        &path,
        "color=c=black:size=160x90:rate=24:duration=0.2",
        None,
        "1",
        &["-f", "h264"],
    );
    let api = runtime();
    let mut session = Session::new(&api).unwrap();
    let selected = capture(&mut session, &path);
    assert!(selected.details["duration_seconds"].is_null());
    assert_eq!(selected.details["quality_fallback"], true);
    assert!(selected.details["attempts"].as_u64().unwrap() <= 3);
    assert_eq!(
        (selected.image.width(), selected.image.height()),
        (256, 144)
    );
}

// Keep the analysis independent from the native decoder and its output format.
#[test]
fn sampled_quality_handles_minimal_image() {
    let image = RgbaImage::from_pixel(1, 1, image::Rgba([255, 255, 255, 255]));
    assert!(quality::assess(&image).unsuitable);
}

#[test]
#[ignore = "Opt-in 4K A/B benchmark; requires the existing DLL, FFmpeg, and a test directory"]
fn interleaved_policy_benchmark() {
    let directory = directory();
    let path = directory.path().join("clip.mp4");
    generate(
        &path,
        "testsrc2=size=3840x2160:rate=12:duration=8",
        None,
        "24",
        &["-sc_threshold", "0"],
    );
    let api = runtime();
    let mut session = Session::new(&api).unwrap();
    let mut times: [Vec<f64>; 4] = std::array::from_fn(|_| Vec::new());
    let mut copies: [Vec<f64>; 4] = std::array::from_fn(|_| Vec::new());
    // Rotate order every round to reduce warm-cache/thermal ordering bias.
    for round in 0..12 {
        for offset in 0..4 {
            let mode = (round + offset) % 4;
            let start = std::time::Instant::now();
            let result = match mode {
                0 => session.capture_legacy(&path, 512, Duration::from_secs(10)),
                1 => session.capture(&path, Some(0.8), 512, Duration::from_secs(10)),
                2 => session.capture_adaptive_unscaled(&path, 512, Duration::from_secs(10)),
                _ => session.capture(&path, None, 512, Duration::from_secs(10)),
            }
            .unwrap();
            times[mode].push(start.elapsed().as_secs_f64() * 1000.0);
            copies[mode].push(result.details["capture_resize_ms"].as_f64().unwrap());
            assert_eq!(result.details["attempts"], 1);
        }
    }
    for (mode, name) in [
        "legacy",
        "scaled-exact",
        "adaptive-unscaled",
        "adaptive-scaled",
    ]
    .into_iter()
    .enumerate()
    {
        times[mode].sort_by(f64::total_cmp);
        println!(
            "{}",
            serde_json::json!({"benchmark_mode": name, "samples": times[mode].len(),
            "median_ms": times[mode][5], "p95_ms": times[mode][11],
            "capture_average_ms": copies[mode].iter().sum::<f64>() / copies[mode].len() as f64})
        );
    }
}
