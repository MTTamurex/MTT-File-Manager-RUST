use super::*;

#[test]
fn repeated_checkpoint_save_replaces_a_snapshot_that_is_still_memory_mapped() {
    crate::index_db::init_data_dir_for_tests();
    let mut index = VolumeIndex::empty('O');
    index.volume_id = r"\\?\Volume{abcdef01-2345-6789-abcd-ef0123456789}\".into();
    index.volume_serial = 123;
    index.journal_id = 7;
    index.sizes_loaded = true;
    index.hardlink_data_complete = true;
    index.reparse_data_complete = true;
    assert!(index.insert_record(10, "file.txt", 5, false, false));

    for checkpoint in [42, 50, 60] {
        index.last_usn = checkpoint;
        save_and_remap(&mut index).unwrap();
        assert!(index.records.is_mapped());
        let (_, saved) = load('O').unwrap().unwrap();
        assert_eq!(saved.last_usn, checkpoint);
    }
}
