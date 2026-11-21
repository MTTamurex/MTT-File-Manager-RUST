//! Install-policy tests use synthetic service handles and temporary artifacts;
//! they never register a service or open the machine's active index storage.
use super::{prepare_install, ERROR_SERVICE_DOES_NOT_EXIST};

#[test]
fn repeated_install_keeps_registration_and_every_cache_artifact() {
    let directory = tempfile::tempdir().unwrap();
    let artifacts = [
        ("search_index.db", b"database".as_slice()),
        ("search_index.db-wal", b"pending transaction".as_slice()),
        ("search_index.db-shm", b"shared memory".as_slice()),
        ("index_R.bin", b"snapshot".as_slice()),
        ("hmac_key.bin", b"sealed key".as_slice()),
    ];
    for (name, bytes) in artifacts {
        std::fs::write(directory.path().join(name), bytes).unwrap();
    }

    for _ in 0..2 {
        let service = prepare_install(Ok(42), || {
            panic!("an upgrade must not reset storage");
        })
        .unwrap();
        assert_eq!(service, Some(42));
        for (name, bytes) in artifacts {
            assert_eq!(std::fs::read(directory.path().join(name)).unwrap(), bytes);
        }
    }
}

#[test]
fn fresh_install_prepares_storage_exactly_once() {
    let mut resets = 0;
    let result = prepare_install::<()>(
        Err(windows_service::Error::Winapi(
            std::io::Error::from_raw_os_error(ERROR_SERVICE_DOES_NOT_EXIST.0 as i32),
        )),
        || {
            resets += 1;
            Ok(())
        },
    );
    assert_eq!(result.unwrap(), None);
    assert_eq!(resets, 1);
}

#[test]
fn lookup_errors_leave_cache_untouched() {
    // Access denied, marked for deletion, and unexpected argument errors.
    for error in [
        windows_service::Error::Winapi(std::io::Error::from_raw_os_error(5)),
        windows_service::Error::Winapi(std::io::Error::from_raw_os_error(1072)),
        windows_service::Error::ArgumentHasNulByte("service name"),
    ] {
        let result = prepare_install::<()>(Err(error), || {
            panic!("a failed lookup must never reset storage");
        });
        assert!(result.is_err());
    }
}

#[test]
fn failed_storage_preparation_blocks_fresh_install() {
    let result = prepare_install::<()>(
        Err(windows_service::Error::Winapi(
            std::io::Error::from_raw_os_error(ERROR_SERVICE_DOES_NOT_EXIST.0 as i32),
        )),
        || Err("storage validation failed".to_string()),
    );
    assert_eq!(result.unwrap_err(), "storage validation failed");
}
