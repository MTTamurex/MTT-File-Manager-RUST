use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TranscodeBackend {
    QSV,
    NVENC,
    AMF,
    CPU,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct HardwareCapabilities {
    pub has_qsv: bool,
    pub has_nvenc: bool,
    pub has_amf: bool,
    // We don't strictly need detailed flags if we validate the backend as a whole
}

impl HardwareCapabilities {
    pub fn new() -> Self {
        Self::default()
    }
}

static CAPABILITIES: OnceLock<HardwareCapabilities> = OnceLock::new();

/// Detect system hardware capabilities for video transcoding.
/// This runs once per application lifecycle and caches the result.
pub fn get_capabilities() -> &'static HardwareCapabilities {
    CAPABILITIES.get_or_init(|| {
        println!("[HardwareDetection] Probing and VALIDATING capabilities...");
        detect_capabilities()
    })
}

fn detect_capabilities() -> HardwareCapabilities {
    let mut caps = HardwareCapabilities::default();

    // Step 1: Check what encoders are compiled into this ffmpeg binary
    let encoders_output = get_ffmpeg_encoders();

    // Step 2: Independent Validation Loop

    // --- QSV ---
    if encoders_output.contains("h264_qsv") {
        println!("[HardwareDetection] Testing QSV...");
        if smoke_test_backend(TranscodeBackend::QSV) {
            println!("[HardwareDetection] QSV smoke test passed.");
            caps.has_qsv = true;
        } else {
            println!("[HardwareDetection] QSV smoke test failed.");
        }
    } else {
        println!("[HardwareDetection] QSV encoder not found in ffmpeg.");
    }

    // --- NVENC ---
    if encoders_output.contains("h264_nvenc") || encoders_output.contains("hevc_nvenc") {
        println!("[HardwareDetection] Testing NVENC...");
        if smoke_test_backend(TranscodeBackend::NVENC) {
            println!("[HardwareDetection] NVENC smoke test passed.");
            caps.has_nvenc = true;
        } else {
            println!("[HardwareDetection] NVENC smoke test failed.");
        }
    } else {
        println!("[HardwareDetection] NVENC encoder not found in ffmpeg.");
    }

    // --- AMF ---
    if encoders_output.contains("h264_amf") || encoders_output.contains("hevc_amf") {
        println!("[HardwareDetection] Testing AMF...");
        if smoke_test_backend(TranscodeBackend::AMF) {
            println!("[HardwareDetection] AMF smoke test passed.");
            caps.has_amf = true;
        } else {
            println!("[HardwareDetection] AMF smoke test failed.");
        }
    }

    println!("[HardwareDetection] Final Capabilities: {:?}", caps);
    caps
}

fn get_ffmpeg_encoders() -> String {
    let Some(ffmpeg) = resolve_ffmpeg_executable() else {
        log::warn!("[HardwareDetection] ffmpeg executable not found in trusted locations");
        return String::new();
    };
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-encoders"]);

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    match cmd.output() {
        Ok(output) => String::from_utf8_lossy(&output.stdout).to_string(),
        Err(e) => {
            log::warn!("[HardwareDetection] Failed to run -encoders: {}", e);
            String::new()
        }
    }
}

/// Runs a real dummy encode to see if the hardware path actually works.
fn smoke_test_backend(backend: TranscodeBackend) -> bool {
    let Some(ffmpeg) = resolve_ffmpeg_executable() else {
        return false;
    };
    let mut cmd = Command::new(ffmpeg);

    // Common quiet flags
    cmd.args(["-v", "error", "-hide_banner"]);

    match backend {
        TranscodeBackend::QSV => {
            // QSV initialization: 1280x720 is a safe standard
            cmd.args([
                "-init_hw_device",
                "qsv=hw",
                "-filter_hw_device",
                "hw",
                "-f",
                "lavfi",
                "-i",
                "color=black:s=1280x720",
                "-c:v",
                "h264_qsv",
                "-f",
                "null",
                "-",
            ]);
        }
        TranscodeBackend::NVENC => {
            // Strict NVENC: Use 1920x1080 to avoid "invalid param" on some GPUs
            cmd.args([
                "-init_hw_device",
                "cuda=cuda",
                "-filter_hw_device",
                "cuda",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=1920x1080:rate=30",
                "-frames:v",
                "1",
                "-c:v",
                "h264_nvenc",
                "-f",
                "null",
                "-",
            ]);
        }
        TranscodeBackend::AMF => {
            // AMF: Use 1280x720
            cmd.args([
                "-f",
                "lavfi",
                "-i",
                "color=black:s=1280x720",
                "-c:v",
                "h264_amf",
                "-f",
                "null",
                "-",
            ]);
        }
        TranscodeBackend::CPU => return true,
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    // Run and capture output to log failure reasons
    match cmd.output() {
        Ok(output) => {
            if output.status.success() {
                true
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                // Log the confusing failure for debugging
                // We print to stdout so it shows in the app console logs
                println!(
                    "[HardwareDetection] Backend {:?} failed: {}",
                    backend, stderr
                );
                false
            }
        }
        Err(e) => {
            println!(
                "[HardwareDetection] Backend {:?} failed to spawn: {}",
                backend, e
            );
            false
        }
    }
}

/// SEC: Resolve ffmpeg.exe to an absolute path from trusted locations only,
/// to defeat PATH/CWD hijacking attacks. `Command::new("ffmpeg")` would
/// otherwise let any `ffmpeg.exe` in the current working directory or PATH
/// be executed in the user context, which is trivially weaponised: a malicious
/// download placed alongside legitimate media triggers RCE the moment hardware
/// detection runs.
///
/// Trusted lookup order:
///   1. The directory of the currently running executable (bundled ffmpeg).
///   2. The `MTT_FFMPEG_PATH` environment variable (must be an absolute path
///      to an existing file).
/// PATH and CWD are NEVER consulted. Returns `None` if ffmpeg is not found,
/// in which case hardware acceleration falls back to CPU transparently.
fn resolve_ffmpeg_executable() -> Option<PathBuf> {
    static CACHED: OnceLock<Option<PathBuf>> = OnceLock::new();
    CACHED
        .get_or_init(|| {
            // 1) Same directory as the running app/service binary.
            if let Ok(exe) = std::env::current_exe() {
                if let Some(dir) = exe.parent() {
                    let candidate = dir.join("ffmpeg.exe");
                    if candidate.is_file() {
                        return Some(candidate);
                    }
                }
            }

            // 2) Dedicated env var (NOT the generic PATH). Must be absolute.
            if let Ok(value) = std::env::var("MTT_FFMPEG_PATH") {
                let candidate = PathBuf::from(value);
                if candidate.is_absolute() && candidate.is_file() {
                    return Some(candidate);
                }
            }

            None
        })
        .clone()
}

#[derive(Debug, Clone)]
pub struct TranscodeProfile {
    pub backend: TranscodeBackend,
    pub video_codec: &'static str,
    pub name: &'static str,
}

/// Returns a prioritized list of verified profiles.
/// Order: QSV > NVENC > AMF > CPU
pub fn get_prioritized_profiles() -> Vec<TranscodeProfile> {
    let caps = get_capabilities();
    let mut profiles = Vec::new();

    if caps.has_qsv {
        profiles.push(TranscodeProfile {
            backend: TranscodeBackend::QSV,
            video_codec: "h264_qsv",
            name: "Intel QuickSync",
        });
    }

    if caps.has_nvenc {
        profiles.push(TranscodeProfile {
            backend: TranscodeBackend::NVENC,
            video_codec: "h264_nvenc",
            name: "NVIDIA NVENC",
        });
    }

    if caps.has_amf {
        profiles.push(TranscodeProfile {
            backend: TranscodeBackend::AMF,
            video_codec: "h264_amf",
            name: "AMD AMF",
        });
    }

    // Always add CPU as final fallback
    profiles.push(TranscodeProfile {
        backend: TranscodeBackend::CPU,
        video_codec: "libx264",
        name: "CPU (Software)",
    });

    profiles
}

/// Centralized Command Builder
/// Ensures that complex flags (like QSV init) are applied consistently.
pub fn build_transcode_args(
    profile: &TranscodeProfile,
    file_path: &str,
    seek: Option<f64>,
) -> Vec<String> {
    let mut args = Vec::new();

    // 1. Pre-input flags (hardware init)
    match profile.backend {
        TranscodeBackend::QSV => {
            args.extend_from_slice(&[
                "-init_hw_device".into(),
                "qsv=hw".into(),
                "-filter_hw_device".into(),
                "hw".into(),
            ]);
        }
        TranscodeBackend::NVENC => {
            // NVENC Full Pipeline:
            // 1. Init CUDA
            // 2. Enable HW Decoding (-hwaccel cuda)
            // 3. Keep decode in GPU memory (-hwaccel_output_format cuda)
            args.extend_from_slice(&[
                "-init_hw_device".into(),
                "cuda=cuda".into(),
                "-filter_hw_device".into(),
                "cuda".into(),
                "-hwaccel".into(),
                "cuda".into(),
                "-hwaccel_output_format".into(),
                "cuda".into(),
            ]);
        }
        TranscodeBackend::AMF => {
            args.extend_from_slice(&["-hwaccel".into(), "d3d11va".into()]);
        }
        _ => {}
    }

    // 2. Seek (must be before input for fast seek)
    if let Some(s) = seek {
        args.push("-ss".into());
        args.push(format!("{:.2}", s));
    }

    // 3. Input
    args.push("-i".into());
    args.push(file_path.to_string());

    // 4. Video Codec & Flags
    args.push("-c:v".into());
    args.push(profile.video_codec.to_string());

    // Hardware specific encoding filters & presets
    match profile.backend {
        TranscodeBackend::QSV => {
            args.extend_from_slice(&["-preset".into(), "fast".into()]);
        }
        TranscodeBackend::NVENC => {
            // NVENC: Use p4 preset (balanced speed/quality), scale to NV12 in GPU memory
            // -cq 23 = Constant Quality mode (similar to x264 CRF, lower = better quality)
            args.extend_from_slice(&[
                "-vf".into(),
                "scale_cuda=format=nv12".into(),
                "-preset".into(),
                "p4".into(),
                "-cq".into(),
                "23".into(),
                "-rc".into(),
                "vbr".into(), // Variable bitrate for better quality
            ]);
        }
        TranscodeBackend::CPU => {
            // CPU: libx264 supports -tune zerolatency for streaming
            args.extend_from_slice(&[
                "-preset".into(),
                "ultrafast".into(),
                "-tune".into(),
                "zerolatency".into(),
            ]);
        }
        _ => {
            args.extend_from_slice(&["-preset".into(), "fast".into()]);
        }
    }

    // 5. Common settings (Audio, Container) - NO -tune here, it's encoder-specific
    args.extend_from_slice(&[
        "-c:a".into(),
        "aac".into(),
        "-ac".into(),
        "2".into(), // Force stereo for safety
        "-b:a".into(),
        "128k".into(),
        "-f".into(),
        "mp4".into(),
        "-movflags".into(),
        "frag_keyframe+empty_moov+faststart".into(),
        "pipe:1".into(),
    ]);

    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_qsv() {
        let profile = TranscodeProfile {
            backend: TranscodeBackend::QSV,
            video_codec: "h264_qsv",
            name: "Test",
        };
        let args = build_transcode_args(&profile, "input.mp4", Some(10.0));
        assert!(args.contains(&"-init_hw_device".to_string()));
        assert!(args.contains(&"qsv=hw".to_string()));
    }

    #[test]
    fn test_builder_nvenc() {
        let profile = TranscodeProfile {
            backend: TranscodeBackend::NVENC,
            video_codec: "h264_nvenc",
            name: "Test",
        };
        let args = build_transcode_args(&profile, "input.mp4", None);
        assert!(args.contains(&"cuda=cuda".to_string()));
    }
}
