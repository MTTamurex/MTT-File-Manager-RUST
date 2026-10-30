use super::*;
use crate::app::disk_analysis_duplicates::reader::{read_candidate, ReadJob, ReadOutcome};
use std::io::Write;

fn job(model: &DiskAnalysisModel) -> ReadJob {
    ReadJob {
        idx: model
            .nodes
            .iter()
            .position(|n| n.name == "file-0.bin")
            .unwrap() as u32,
        size: PARTIAL_BYTES * 2,
        previous: None,
    }
}

#[test]
fn replacement_between_prefix_and_full_pass_is_rejected_before_reading() {
    let (dir, model, resolver) = fixture(&[vec![7; PARTIAL_BYTES as usize * 2]]);
    let mut job = job(&model);
    let cancel = AtomicBool::new(false);
    let ReadOutcome::Hashed(prefix) =
        read_candidate(&resolver, job, Some(PARTIAL_BYTES), &cancel, |_| {})
    else {
        panic!("prefix")
    };
    std::fs::rename(
        dir.path().join("file-0.bin"),
        dir.path().join("previous.bin"),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("file-0.bin"),
        vec![7; PARTIAL_BYTES as usize * 2],
    )
    .unwrap();
    job.previous = Some(prefix.stamp);
    let mut bytes = 0;
    assert!(matches!(
        read_candidate(&resolver, job, None, &cancel, |n| bytes += n),
        ReadOutcome::Changed
    ));
    assert_eq!(bytes, 0);
}

#[test]
fn modification_between_passes_and_during_read_is_rejected() {
    let (dir, model, resolver) = fixture(&[vec![7; PARTIAL_BYTES as usize * 2]]);
    let mut job = job(&model);
    let cancel = AtomicBool::new(false);
    let ReadOutcome::Hashed(prefix) =
        read_candidate(&resolver, job, Some(PARTIAL_BYTES), &cancel, |_| {})
    else {
        panic!("prefix")
    };
    job.previous = Some(prefix.stamp);
    // Force an observable timestamp change even on filesystems with coarse clocks.
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(dir.path().join("file-0.bin"))
        .unwrap();
    file.set_times(
        std::fs::FileTimes::new()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000)),
    )
    .unwrap();
    drop(file);
    assert!(matches!(
        read_candidate(&resolver, job, None, &cancel, |_| {}),
        ReadOutcome::Changed
    ));
    job.previous = None;
    let outcome = read_candidate(&resolver, job, Some(PARTIAL_BYTES), &cancel, |_| {
        let mut writer = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.path().join("file-0.bin"))
            .unwrap();
        writer.write_all(&[1]).unwrap();
    });
    assert!(matches!(outcome, ReadOutcome::Changed));
}

#[test]
fn cancelling_inside_read_never_returns_a_digest() {
    let (_dir, model, resolver) = fixture(&[vec![7; PARTIAL_BYTES as usize * 2]]);
    let cancel = AtomicBool::new(false);
    let outcome = read_candidate(&resolver, job(&model), None, &cancel, |_| {
        cancel.store(true, Ordering::Release)
    });
    assert!(matches!(outcome, ReadOutcome::Cancelled));
}

#[cfg(windows)]
#[test]
fn offline_file_is_skipped_without_reading_in_either_pass() {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::PCWSTR,
        Win32::Storage::FileSystem::{
            SetFileAttributesW, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_OFFLINE,
        },
    };
    let (dir, model, resolver) = fixture(&[vec![7; PARTIAL_BYTES as usize * 2]]);
    let path: Vec<_> = dir
        .path()
        .join("file-0.bin")
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        SetFileAttributesW(PCWSTR(path.as_ptr()), FILE_ATTRIBUTE_OFFLINE).unwrap();
    }
    let cancel = AtomicBool::new(false);
    for limit in [Some(PARTIAL_BYTES), None] {
        let outcome = read_candidate(&resolver, job(&model), limit, &cancel, |_| {
            panic!("offline content read")
        });
        assert!(matches!(outcome, ReadOutcome::Unavailable));
    }
    unsafe {
        SetFileAttributesW(PCWSTR(path.as_ptr()), FILE_ATTRIBUTE_NORMAL).unwrap();
    }
}
