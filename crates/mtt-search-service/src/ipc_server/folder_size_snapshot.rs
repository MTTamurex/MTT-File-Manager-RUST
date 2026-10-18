use crate::file_index::VolumeIndex;

/// Called while the index read/write guard is held. Checking before live-path
/// I/O is insufficient: another journal buffer may be applied during that I/O.
pub(super) fn summary(index: &VolumeIndex, frn: u64) -> Result<(u64, u64, u64, u64), &'static str> {
    if index.journal_refresh_in_progress {
        return Err("Index refresh pending");
    }
    Ok(crate::mft_reader::folder_size_for_service(index, frn))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_journal_snapshot_is_rejected_until_the_reader_catches_up() {
        let mut index = VolumeIndex::empty('C');
        assert!(index.insert_record(10, "Folder", 5, true, false));
        assert!(index.insert_record(20, "file.bin", 10, false, false));
        index.records.get_mut(&20).unwrap().size = 10;
        index.journal_refresh_in_progress = true;
        assert_eq!(summary(&index, 5), Err("Index refresh pending"));
        assert_eq!(summary(&index, 10), Err("Index refresh pending"));

        index.journal_refresh_in_progress = false;
        assert_eq!(summary(&index, 5).unwrap().0, 10);
        assert_eq!(summary(&index, 10).unwrap().0, 10);
    }

    #[test]
    fn an_empty_indexed_subtree_is_not_proof_that_the_journal_is_caught_up() {
        let mut index = VolumeIndex::empty('C');
        index.journal_refresh_in_progress = true;
        assert!(summary(&index, 5).is_err());
        index.clear();
        assert!(!index.journal_refresh_in_progress);
        assert_eq!(summary(&index, 5).unwrap().0, 0);
    }
}
