use crate::file_index::VolumeIndex;
use crate::index_db::IndexDb;

pub(super) struct TestDb {
    pub db: IndexDb,
    pub sql: rusqlite::Connection,
    _directory: tempfile::TempDir,
}

impl TestDb {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("index.db");
        let db = IndexDb::open(&path).unwrap();
        let sql = rusqlite::Connection::open(path).unwrap();
        Self {
            db,
            sql,
            _directory: directory,
        }
    }
}

pub(super) fn index() -> VolumeIndex {
    let mut index = VolumeIndex::empty('R');
    index.volume_id = "volume-id".into();
    index.volume_serial = 123;
    index.journal_id = 7;
    index.last_usn = 42;
    index.hardlink_data_complete = true;
    index.reparse_data_complete = true;
    index
}

pub(super) fn usn_record(frn: u64, parent: u64, reason: u32, name: &[u16]) -> Vec<u8> {
    let mut record = vec![0u8; 60 + name.len() * 2];
    let len = record.len() as u32;
    record[0..4].copy_from_slice(&len.to_le_bytes());
    record[4..6].copy_from_slice(&2u16.to_le_bytes());
    record[8..16].copy_from_slice(&frn.to_le_bytes());
    record[16..24].copy_from_slice(&parent.to_le_bytes());
    record[40..44].copy_from_slice(&reason.to_le_bytes());
    record[56..58].copy_from_slice(&((name.len() * 2) as u16).to_le_bytes());
    record[58..60].copy_from_slice(&60u16.to_le_bytes());
    for (chunk, unit) in record[60..].chunks_exact_mut(2).zip(name) {
        chunk.copy_from_slice(&unit.to_le_bytes());
    }
    record
}
