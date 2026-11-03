//! Explicit-use diagnostic for Windows Sandbox, not installed with the app.
//! No requests are sent unless --sandbox is supplied. No response data is logged.

#[path = "ipc_access_probe/transport.rs"]
mod transport;

use std::process::ExitCode;
use std::sync::mpsc;
use std::time::Duration;

use mtt_search_protocol::{SearchRequest, SearchResponse};

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Pass,
    Exposed,
    Inconclusive,
}

fn drive_from_args(args: &[String]) -> Result<char, &'static str> {
    match args {
        [flag] if flag == "--sandbox" => Ok('C'),
        [flag, option, drive] if flag == "--sandbox" && option == "--drive" => {
            let bytes = drive.as_bytes();
            if bytes.len() == 1 && bytes[0].is_ascii_alphabetic() {
                Ok((bytes[0] as char).to_ascii_uppercase())
            } else {
                Err("Drive must be one ASCII letter, for example C.")
            }
        }
        _ => Err("Usage (inside Windows Sandbox only): ipc_access_probe.exe --sandbox [--drive C]"),
    }
}

fn protected_outcome(response: &SearchResponse) -> Outcome {
    match response {
        SearchResponse::Error(message) if message == "Authorization failed" => Outcome::Pass,
        SearchResponse::Status(_) | SearchResponse::DiskAnalysis(_) => Outcome::Exposed,
        _ => Outcome::Inconclusive,
    }
}

fn run(drive: char) -> Outcome {
    let response = match transport::request(&SearchRequest::Ping) {
        Ok(response) => response,
        Err(reason) => {
            println!("INCONCLUSIVE Ping: {reason}");
            return Outcome::Inconclusive;
        }
    };
    if !matches!(response, SearchResponse::Pong) {
        println!("INCONCLUSIVE Ping: expected public Pong response.");
        return Outcome::Inconclusive;
    }
    println!("PASS Ping: registered service responded; Ping is intentionally public.");

    let mut overall = Outcome::Pass;
    for (label, request) in [
        ("GetStatus", SearchRequest::GetStatus),
        (
            "DiskAnalysis",
            SearchRequest::DiskAnalysis {
                drive_letter: drive,
            },
        ),
    ] {
        let outcome = match transport::request(&request) {
            Ok(response) => protected_outcome(&response),
            Err(reason) => {
                println!("INCONCLUSIVE {label}: {reason}");
                if overall != Outcome::Exposed {
                    overall = Outcome::Inconclusive;
                }
                continue;
            }
        };
        match outcome {
            Outcome::Pass => println!("PASS {label}: unauthorized client explicitly rejected."),
            Outcome::Exposed => {
                println!("FAIL {label}: protected metadata returned; contents not displayed.");
                overall = Outcome::Exposed;
            }
            Outcome::Inconclusive => {
                println!(
                    "INCONCLUSIVE {label}: response was not an explicit authorization denial."
                );
                if overall != Outcome::Exposed {
                    overall = Outcome::Inconclusive;
                }
            }
        }
    }
    overall
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let drive = match drive_from_args(&args) {
        Ok(drive) => drive,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    // This flag is explicit consent, not automatic Sandbox detection.
    println!("Sandbox-only diagnostic: Ping, GetStatus and DiskAnalysis; no changes requested.");
    println!("Do not rename this executable to the file manager's name.");
    let renamed = std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .is_none_or(|name| name.eq_ignore_ascii_case("mtt-file-manager.exe"));
    if renamed {
        eprintln!("Refusing to probe without a distinct diagnostic executable identity.");
        return ExitCode::from(2);
    }

    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(run(drive));
    });
    // Exiting main ends this diagnostic process and closes its pipe handles,
    // including a worker blocked on I/O. No service process is terminated.
    match receiver.recv_timeout(Duration::from_secs(15)) {
        Ok(Outcome::Pass) => ExitCode::SUCCESS,
        Ok(Outcome::Exposed) => ExitCode::from(1),
        Ok(Outcome::Inconclusive) => ExitCode::from(2),
        Err(_) => {
            eprintln!("INCONCLUSIVE: diagnostic timed out or worker failed; no success claimed.");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mtt_search_protocol::{DiskAnalysisSnapshot, IndexStatusInfo};

    #[test]
    fn explicit_sandbox_argument_is_required() {
        assert!(drive_from_args(&[]).is_err());
        assert!(drive_from_args(&["--drive".into(), "C".into()]).is_err());
        assert_eq!(drive_from_args(&["--sandbox".into()]), Ok('C'));
        assert_eq!(
            drive_from_args(&["--sandbox".into(), "--drive".into(), "d".into()]),
            Ok('D')
        );
        for drive in ["", "C:", "1", "CC", "é"] {
            assert!(
                drive_from_args(&["--sandbox".into(), "--drive".into(), drive.into()]).is_err()
            );
        }
    }

    #[test]
    fn only_explicit_authorization_denial_counts_as_pass() {
        assert_eq!(
            protected_outcome(&SearchResponse::Error("Authorization failed".into())),
            Outcome::Pass
        );
        for response in [
            SearchResponse::Pong,
            SearchResponse::Error("Volume not ready".into()),
            SearchResponse::Error("Server busy, try again later".into()),
        ] {
            assert_eq!(protected_outcome(&response), Outcome::Inconclusive);
        }
    }

    #[test]
    fn protected_metadata_is_not_misreported_as_denial() {
        for response in [
            SearchResponse::Status(IndexStatusInfo {
                volumes: Vec::new(),
                total_files_indexed: 0,
                service_executable_path: "service.exe".into(),
            }),
            SearchResponse::DiskAnalysis(Box::new(DiskAnalysisSnapshot {
                drive_letter: 'C',
                records: Vec::new(),
            })),
        ] {
            assert_eq!(protected_outcome(&response), Outcome::Exposed);
        }
    }
}
