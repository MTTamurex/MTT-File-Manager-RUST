mod metrics;

use anyhow::{ensure, Context, Result};
use clap::Parser;
use metrics::Watchdog;
use mtt_mpv_thumbnail::{Api, Session};
use serde_json::json;
use std::{
    io::Write,
    os::windows::fs::OpenOptionsExt,
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Parser)]
#[command(about = "Headless in-memory video thumbnail experiment using an existing libmpv DLL")]
struct Args {
    /// Exact DLL to test; no PATH/CWD discovery or automatic downloads.
    #[arg(long)]
    dll: PathBuf,
    /// Print runtime versions and compiled decoder inventory.
    #[arg(long)]
    inspect: bool,
    /// Exact timestamp; default prefers MKV covers, then dispersed nonblank frames.
    #[arg(long, value_parser = timestamp)]
    at: Option<f64>,
    /// A/B baseline: original exact 10%-up-to-30s selection, without native scaling.
    #[arg(long, conflicts_with = "at")]
    legacy: bool,
    #[arg(long, default_value_t = 256, value_parser = clap::value_parser!(u32).range(1..=4096))]
    size: u32,
    #[arg(long, default_value_t = 15, value_parser = clap::value_parser!(u32).range(1..=600))]
    timeout: u32,
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=10000))]
    repeat: u32,
    /// Reuse one dedicated mpv instance across the entire batch.
    #[arg(long)]
    reuse: bool,
    /// Test exclusive file access after each capture (use only with isolated fixtures).
    #[arg(long)]
    check_release: bool,
    /// Save one PNG for inspection. Requires exactly one input and one repetition.
    #[arg(long)]
    output: Option<PathBuf>,
    /// Local video files. Report identifiers are numeric, not filenames.
    inputs: Vec<PathBuf>,
}

fn timestamp(value: &str) -> std::result::Result<f64, String> {
    let value: f64 = value.parse().map_err(|_| "Timestamp must be a number")?;
    if value.is_finite() && value >= 0.0 {
        Ok(value)
    } else {
        Err("Timestamp must be finite and nonnegative".into())
    }
}

fn run(args: Args) -> Result<bool> {
    ensure!(
        args.inspect || !args.inputs.is_empty(),
        "Provide --inspect or at least one local video"
    );
    ensure!(
        args.output.is_none() || (args.inputs.len() == 1 && args.repeat == 1),
        "--output requires one input and --repeat 1"
    );
    if let Some(output) = &args.output {
        ensure!(!output.exists(), "Refusing to overwrite the output file");
    }
    // Validate local paths before passing them to mpv; URLs are not accepted.
    let inputs = args
        .inputs
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let path = path
                .canonicalize()
                .with_context(|| format!("Cannot resolve input {}", index + 1))?;
            ensure!(path.is_file(), "Input {} is not a regular file", index + 1);
            ensure!(
                path.to_str().is_some(),
                "Input {} is not Unicode",
                index + 1
            );
            Ok(path)
        })
        .collect::<Result<Vec<_>>>()?;
    let timeout = Duration::from_secs(args.timeout as u64);
    let startup_guard = Watchdog::start(timeout);
    let api = Api::load(&args.dll)?;
    let mut shared = None;
    if args.inspect || args.reuse {
        let session = Session::new(&api)?;
        if args.inspect {
            println!("{}", json!({"runtime": session.inspect()}));
        }
        if args.reuse {
            shared = Some(session);
        }
    }
    drop(startup_guard);
    let mut elapsed = Vec::new();
    let mut failures = 0;
    println!(
        "{}",
        json!({"baseline": metrics::resources(), "reuse": args.reuse})
    );
    for repetition in 1..=args.repeat {
        for (index, path) in inputs.iter().enumerate() {
            // Created before session so its Drop also covers native destruction.
            let guard = Watchdog::start(timeout);
            let before = metrics::resources();
            let start = Instant::now();
            let mut local = None;
            let result = (|| {
                if shared.is_none() {
                    local = Some(Session::new(&api)?);
                }
                let init_ms = start.elapsed().as_secs_f64() * 1000.0;
                let session = shared
                    .as_mut()
                    .or(local.as_mut())
                    .context("Missing mpv session")?;
                let mut capture = if args.legacy {
                    session.capture_legacy(path, args.size, timeout)?
                } else {
                    session.capture(path, args.at, args.size, timeout)?
                };
                if args.check_release {
                    let _file = std::fs::OpenOptions::new()
                        .read(true)
                        .share_mode(0)
                        .open(path)
                        .context("Input is still locked after capture")?;
                    capture.details["file_released"] = true.into();
                }
                if let Some(output) = &args.output {
                    // create_new prevents both accidental overwrites and check/use races.
                    let file = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(output)
                        .context("Cannot create the requested PNG output")?;
                    let mut writer = std::io::BufWriter::new(file);
                    image::DynamicImage::ImageRgba8(capture.image)
                        .write_to(&mut writer, image::ImageFormat::Png)?;
                    writer.flush().context("Cannot flush the PNG output")?;
                }
                Ok::<_, anyhow::Error>((capture.details, init_ms))
            })();
            drop(local);
            let total_ms = start.elapsed().as_secs_f64() * 1000.0;
            match result {
                Ok((details, init_ms)) => {
                    elapsed.push(total_ms);
                    println!(
                        "{}",
                        json!({"input": index + 1, "repetition": repetition,
                        "ok": true, "init_ms": init_ms, "total_with_teardown_ms": total_ms,
                        "capture": details, "resources_before": before, "resources_after": metrics::resources()})
                    );
                }
                Err(error) => {
                    failures += 1;
                    println!(
                        "{}",
                        json!({"input": index + 1, "repetition": repetition,
                        "ok": false, "error": format!("{error:#}"), "total_with_teardown_ms": total_ms,
                        "resources_after": metrics::resources()})
                    );
                    // Do not reuse uncertain native state following a failed capture.
                    drop(shared.take());
                    if args.reuse {
                        shared = Some(Session::new(&api)?);
                    }
                }
            }
            drop(guard);
        }
    }
    let teardown_guard = Watchdog::start(timeout);
    drop(shared);
    elapsed.sort_by(f64::total_cmp);
    println!(
        "{}",
        json!({"summary": {"successful": elapsed.len(), "failed": failures,
        "median_ms": metrics::percentile(&elapsed, 50), "p95_ms": metrics::percentile(&elapsed, 95),
        "resources_final": metrics::resources()}})
    );
    drop(teardown_guard);
    Ok(failures == 0)
}

fn main() {
    match run(Args::parse()) {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(error) => {
            eprintln!("POC failed: {error:#}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_timestamps() {
        for invalid in ["NaN", "inf", "-1", "abc"] {
            assert!(timestamp(invalid).is_err());
        }
        assert_eq!(timestamp("0").unwrap(), 0.0);
        assert_eq!(timestamp("1.25").unwrap(), 1.25);
    }

    #[test]
    fn validates_cli_bounds() {
        assert!(Args::try_parse_from(["poc", "--dll", "runtime.dll", "--size", "0"]).is_err());
        assert!(Args::try_parse_from(["poc", "--dll", "runtime.dll", "--timeout", "0"]).is_err());
        assert!(Args::try_parse_from(["poc", "--dll", "runtime.dll", "--repeat", "0"]).is_err());
        assert!(
            Args::try_parse_from(["poc", "--dll", "runtime.dll", "--legacy", "--at", "1"]).is_err()
        );
    }
}
