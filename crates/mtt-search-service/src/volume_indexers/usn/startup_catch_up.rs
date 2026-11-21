use crate::file_index::VolumeIndex;

/// Finish replaying the journal visible at startup before publishing the cache.
/// A journal read returns only one buffer, not necessarily the whole backlog.
pub(super) fn catch_up(
    index: &mut VolumeIndex,
    startup_tail: i64,
    mut read_changes: impl FnMut(i64, &mut VolumeIndex) -> Result<i64, String>,
    should_stop: impl Fn() -> bool,
) -> Result<i64, String> {
    if index.last_usn > startup_tail {
        return Err("Cached checkpoint is ahead of the current journal".to_string());
    }
    index.journal_refresh_in_progress = true;
    while index.last_usn < startup_tail {
        if should_stop() {
            return Err("Startup journal replay cancelled".to_string());
        }
        let previous_usn = index.last_usn;
        let next_usn = read_changes(previous_usn, index)?;
        if next_usn <= previous_usn {
            return Err(
                "Startup journal replay stopped before reaching its checkpoint".to_string(),
            );
        }
        index.last_usn = next_usn;
    }
    if should_stop() {
        return Err("Startup journal replay cancelled".to_string());
    }
    index.journal_refresh_in_progress = false;
    Ok(index.last_usn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::volume_indexers::test_support::{index, usn_record};

    #[test]
    fn cached_metrics_survive_multiple_journal_buffers_without_a_full_scan() {
        let mut index = index();
        assert!(index.insert_record(10, "existing.txt", 5, false, false));
        index.records.get_mut(&10).unwrap().size = 123;
        index.sizes_loaded = true;
        index.clear_pending();
        let mut calls = 0;
        let result = catch_up(
            &mut index,
            50,
            |cursor, index| {
                assert!(index.journal_refresh_in_progress);
                calls += 1;
                let frn = 20 + calls;
                let record = usn_record(
                    frn,
                    5,
                    0x0000_0100, // USN_REASON_FILE_CREATE
                    &"new.txt".encode_utf16().collect::<Vec<_>>(),
                );
                crate::usn_journal::parse_usn_records(&record, index, &mut 0, true)?;
                Ok(cursor + 4)
            },
            || false,
        );
        assert_eq!(result.unwrap(), 50);
        assert_eq!(calls, 2);
        assert_eq!(index.records.len(), 3);
        assert_eq!(index.records.get(&10).unwrap().size, 123);
        assert!(index.sizes_loaded);
        assert!(!index.journal_refresh_in_progress);
    }

    #[test]
    fn current_cache_needs_no_journal_reads() {
        let mut index = index();
        assert_eq!(
            catch_up(&mut index, 42, |_, _| panic!("already caught up"), || false).unwrap(),
            42
        );
    }

    #[test]
    fn failed_or_nonadvancing_replay_is_not_reported_as_ready() {
        for fail in [false, true] {
            let mut index = index();
            assert!(catch_up(
                &mut index,
                50,
                |cursor, _| if fail {
                    Err("injected read error".to_string())
                } else {
                    Ok(cursor)
                },
                || false,
            )
            .is_err());
            assert_eq!(index.last_usn, 42);
            assert!(index.journal_refresh_in_progress);
        }
    }

    #[test]
    fn cancelled_replay_does_not_open_another_buffer() {
        let mut index = index();
        assert!(catch_up(&mut index, 50, |_, _| panic!("cancelled"), || true).is_err());
        assert_eq!(index.last_usn, 42);
    }
}
