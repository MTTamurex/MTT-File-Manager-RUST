use super::*;
use std::os::windows::io::AsRawHandle;

fn geometry(records: usize) -> MftGeometry {
    MftGeometry {
        bytes_per_sector: 512,
        bytes_per_cluster: 512,
        bytes_per_file_record: 1024,
        mft_start_lcn: 0,
        mft_valid_data_length: (records * 1024) as i64,
    }
}

#[test]
fn seek_failure_cannot_be_reported_as_a_successful_size_refresh() {
    let result = read_mft_size_data(
        HANDLE(-1isize as *mut _),
        'R',
        &geometry(1),
        &[(0, 2)],
        1,
        &mut |_, _| {},
        &mut || false,
    );
    assert!(result.is_err());
}

#[test]
fn eof_and_short_reads_cannot_be_reported_as_complete_metrics() {
    for length in [0, 1024] {
        let file = tempfile::tempfile().unwrap();
        file.set_len(length).unwrap();
        let result = read_mft_size_data(
            HANDLE(file.as_raw_handle()),
            'R',
            &geometry(4),
            &[(0, 8)],
            4,
            &mut |_, _| {},
            &mut || false,
        );
        assert!(result.is_err());
    }
}

#[test]
fn exhausted_runs_cannot_mark_a_partial_scan_complete() {
    let file = tempfile::tempfile().unwrap();
    let result = read_mft_size_data(
        HANDLE(file.as_raw_handle()),
        'R',
        &geometry(1),
        &[],
        1,
        &mut |_, _| {},
        &mut || false,
    );
    assert!(result.is_err());
}

#[test]
fn runs_beyond_the_valid_mft_length_are_not_read() {
    let result = read_mft_size_data(
        HANDLE(-1isize as *mut _),
        'R',
        &geometry(0),
        &[(0, 2)],
        0,
        &mut |_, _| {},
        &mut || false,
    );
    assert!(result.unwrap().is_empty());
}
