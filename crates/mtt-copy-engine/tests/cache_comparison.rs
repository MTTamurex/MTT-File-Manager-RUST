//! Explicit opt-in smoke comparison, not a cold-cache benchmark or a speed claim.
#[cfg(windows)]
#[test]
#[ignore = "Writes a 32 MiB fixture and three copies; run explicitly on a disposable test volume"]
fn compare_buffered_unbuffered_and_engine() {
    use std::io::Write;
    use std::os::windows::ffi::OsStrExt;
    use std::time::Instant;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        CopyFileExW, COPYFILE_FLAGS, COPY_FILE_NO_BUFFERING,
    };

    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.bin");
    let mut fixture = std::fs::File::create(&source).unwrap();
    let block = vec![37u8; 1024 * 1024];
    for _ in 0..32 {
        fixture.write_all(&block).unwrap();
    }
    fixture.sync_all().unwrap();
    drop(fixture);
    let source_wide: Vec<_> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    for (label, flags) in [
        ("buffered", COPYFILE_FLAGS(0)),
        ("unbuffered", COPY_FILE_NO_BUFFERING),
    ] {
        let destination = root.path().join(format!("{label}.bin"));
        let wide: Vec<_> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let start = Instant::now();
        unsafe {
            CopyFileExW(
                PCWSTR(source_wide.as_ptr()),
                PCWSTR(wide.as_ptr()),
                None,
                None,
                None,
                flags,
            )
        }
        .unwrap();
        println!("mode={label} elapsed_ms={}", start.elapsed().as_millis());
        let bytes = std::fs::read(destination).unwrap();
        assert_eq!(bytes.len(), 32 * 1024 * 1024);
        assert!(bytes.iter().all(|&byte| byte == 37));
    }
    let destination = root.path().join("engine");
    std::fs::create_dir(&destination).unwrap();
    let progress = mtt_copy_engine::Progress::default();
    let start = Instant::now();
    let summary = mtt_copy_engine::copy(&[source], &destination, &progress).unwrap();
    println!(
        "mode=engine elapsed_ms={} policy={:?}",
        start.elapsed().as_millis(),
        progress.telemetry.policy()
    );
    assert_eq!(summary.failures, 0);
    assert_eq!(summary.files_copied, 1);
    let bytes = std::fs::read(destination.join("source.bin")).unwrap();
    assert_eq!(bytes.len(), 32 * 1024 * 1024);
    assert!(bytes.iter().all(|&byte| byte == 37));
}
