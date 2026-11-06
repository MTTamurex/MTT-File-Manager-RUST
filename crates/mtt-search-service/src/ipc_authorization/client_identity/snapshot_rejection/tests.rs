use super::*;

fn encoded(name: &str) -> Vec<u16> {
    name.encode_utf16().chain([0, 0]).collect()
}

fn entry(pid: u32, name: &str) -> PROCESSENTRY32W {
    let mut entry = PROCESSENTRY32W {
        th32ProcessID: pid,
        ..Default::default()
    };
    let name = encoded(name);
    entry.szExeFile[..name.len()].copy_from_slice(&name);
    entry
}

#[test]
fn complete_names_supply_negative_evidence_only() {
    assert_eq!(
        classify_basename(&encoded("other.exe")),
        Ok(BasenameEvidence::Foreign)
    );
    assert_eq!(
        classify_basename(&encoded("MTT-FILE-MANAGER.EXE")),
        Ok(BasenameEvidence::MatchesExpected)
    );
    assert!(matches!(
        after_image_failure("image unavailable".into(), Ok(BasenameEvidence::Foreign)),
        ClientIdentityError::Rejected(_)
    ));
    for evidence in [
        Ok(BasenameEvidence::MatchesExpected),
        Err("snapshot unavailable"),
    ] {
        let error = after_image_failure("both query reasons".into(), evidence);
        assert!(
            matches!(error, ClientIdentityError::Unavailable(ref reason) if reason.contains("both query reasons"))
        );
    }
}

#[test]
fn uncertain_names_never_supply_negative_evidence() {
    for name in [
        "",
        ".",
        "..",
        "other.exe.",
        "other.exe ",
        "OTHER~1.EXE",
        "folder/file.exe",
        r"folder\file.exe",
        "C:other.exe",
        "bad\n.exe",
        "bad?.exe",
        "bad*.exe",
        "bad|.exe",
        "bad<.exe",
        "bad>.exe",
        "bad\".exe",
    ] {
        assert!(classify_basename(&encoded(name)).is_err());
    }
    for name in [vec![], vec![65; 260], vec![0xd800, 0, 0], vec![65, 0]] {
        assert!(classify_basename(&name).is_err());
    }
    let mut truncated = [65; 260];
    truncated[259] = 0;
    assert!(classify_basename(&truncated).is_err());
}

#[test]
fn only_the_target_pid_can_supply_evidence() {
    let mut entries = [entry(10, "other.exe"), entry(20, EXPECTED_CLIENT_BASENAME)].into_iter();
    assert_eq!(
        scan_entries(20, 2, || Ok(entries.next().unwrap()), || true),
        Ok(BasenameEvidence::MatchesExpected)
    );
    let mut entries = [entry(10, EXPECTED_CLIENT_BASENAME), entry(20, "other.exe")].into_iter();
    assert_eq!(
        scan_entries(20, 2, || Ok(entries.next().unwrap()), || true),
        Ok(BasenameEvidence::Foreign)
    );
}

#[test]
fn missing_pid_and_native_errors_remain_inconclusive() {
    assert!(scan_entries(20, 3, || Err("end or native error"), || true).is_err());
    assert!(inspect_pid(0).is_err());
}

#[test]
fn entry_cap_stops_enumeration() {
    let mut reads = 0;
    assert!(scan_entries(
        20,
        2,
        || {
            reads += 1;
            Ok(entry(10, "other.exe"))
        },
        || true
    )
    .is_err());
    assert_eq!(reads, 2);
    assert!(scan_entries(20, 0, || panic!("zero cap must not read"), || true).is_err());
}

#[test]
fn expired_budget_prevents_native_read() {
    assert!(scan_entries(20, 2, || panic!("expired budget must not read"), || false).is_err());
}

#[test]
fn slow_read_cannot_supply_negative_evidence_after_budget() {
    let mut checks = 0;
    assert!(scan_entries(
        20,
        2,
        || Ok(entry(20, "other.exe")),
        || {
            checks += 1;
            checks == 1
        }
    )
    .is_err());
    assert_eq!(checks, 2);
}

#[test]
fn budget_is_checked_between_entries() {
    let mut checks = 0;
    let mut reads = 0;
    assert!(scan_entries(
        20,
        3,
        || {
            reads += 1;
            Ok(entry(10, "other.exe"))
        },
        || {
            checks += 1;
            checks < 3
        }
    )
    .is_err());
    assert_eq!(reads, 1);
}

#[test]
fn rejection_reason_contains_no_observed_name() {
    let error = after_image_failure(
        "private query details".into(),
        classify_basename(&encoded("other.exe")),
    )
    .to_string();
    assert_eq!(
        error,
        "Client process snapshot identifies a different executable basename"
    );
}
