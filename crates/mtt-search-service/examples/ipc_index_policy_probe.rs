//! Explicit-consent diagnostic of the three incrementally restricted index APIs.
//! Never install with the app or rename to the app's executable name.

#[path = "ipc_index_policy_probe/checks.rs"]
mod checks;
#[path = "ipc_index_policy_probe/fixture.rs"]
mod fixture;
// Reuse the existing SCM/PID verification and bounded response decoder.
#[path = "ipc_access_probe/transport.rs"]
mod transport;

use std::process::ExitCode;
use std::sync::mpsc;
use std::time::Duration;

use checks::Outcome;
use fixture::Fixture;

#[derive(Debug, PartialEq, Eq)]
enum Target {
    Sandbox,
    InstalledService,
}

fn target_from_args(args: &[String]) -> Result<Target, &'static str> {
    match args {
        [flag] if flag == "--sandbox" => Ok(Target::Sandbox),
        [flag] if flag == "--installed-service" => Ok(Target::InstalledService),
        _ => Err("Usage: ipc_index_policy_probe.exe --sandbox | --installed-service (explicit consent required)"),
    }
}

fn diagnostic_name_allowed(name: &str) -> bool {
    name.eq_ignore_ascii_case("ipc_index_policy_probe.exe")
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let target = match target_from_args(&args) {
        Ok(target) => target,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    let name_allowed = std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .is_some_and(|name| diagnostic_name_allowed(&name));
    if !name_allowed {
        eprintln!("Refusing renamed diagnostic; keep the original executable name.");
        return ExitCode::from(2);
    }

    // Explicit operator consent, not automatic environment detection.
    match target {
        Target::Sandbox => {
            println!("Sandbox diagnostic: Ping, Query, FolderSize and CheckPathsModified.")
        }
        Target::InstalledService => {
            println!("Authorized installed-service diagnostic: Ping, Query, FolderSize and CheckPathsModified.");
            println!(
                "Tests the installed binary, not proof of the repository's current source version."
            );
        }
    }
    println!("Uses temporary synthetic data only; response contents are never displayed.");
    let fixture = match Fixture::new() {
        Ok(fixture) => fixture,
        Err(reason) => {
            eprintln!("INCONCLUSIVE fixture: {reason}");
            return ExitCode::from(2);
        }
    };
    let checks = match fixture.checks() {
        Ok(checks) => checks,
        Err(reason) => {
            eprintln!("INCONCLUSIVE fixture: {reason}");
            return ExitCode::from(2);
        }
    };
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(checks::run(checks, transport::request));
    });
    // Keep the fixture alive for the complete request budget. A timed-out worker
    // only owns request strings and pipe handles, not the temporary directory.
    let outcome = receiver.recv_timeout(Duration::from_secs(15));
    let exit = match outcome {
        Ok(Outcome::Pass) => ExitCode::SUCCESS,
        Ok(Outcome::Accepted) => ExitCode::from(1),
        Ok(Outcome::Inconclusive) => ExitCode::from(2),
        Err(_) => {
            eprintln!(
                "INCONCLUSIVE: request deadline exceeded or worker failed; no success claimed."
            );
            ExitCode::from(2)
        }
    };
    // Cleanup concerns this fixture only. Returning terminates a blocked worker
    // with this diagnostic process; it never terminates the service.
    drop(fixture);
    exit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exactly_one_explicit_target_is_required() {
        assert_eq!(target_from_args(&["--sandbox".into()]), Ok(Target::Sandbox));
        assert_eq!(
            target_from_args(&["--installed-service".into()]),
            Ok(Target::InstalledService)
        );
        for args in [
            vec![],
            vec!["--drive".into(), "C".into()],
            vec!["--sandbox".into(), "--drive".into(), "C".into()],
            vec!["--sandbox".into(), "--sandbox".into()],
            vec!["--sandbox".into(), "--installed-service".into()],
            vec!["--installed-service".into(), "--installed-service".into()],
            vec!["--installed-service".into(), "--drive".into(), "C".into()],
        ] {
            assert!(target_from_args(&args).is_err());
        }
    }

    #[test]
    fn only_original_diagnostic_name_is_allowed() {
        assert!(diagnostic_name_allowed("ipc_index_policy_probe.exe"));
        assert!(diagnostic_name_allowed("IPC_INDEX_POLICY_PROBE.EXE"));
        for name in [
            "mtt-file-manager.exe",
            "other.exe",
            "",
            "ipc_access_probe.exe",
        ] {
            assert!(!diagnostic_name_allowed(name));
        }
    }
}
