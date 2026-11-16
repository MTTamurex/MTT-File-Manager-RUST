//! Late worker notifications may heal ancestry, but cannot resurrect a tag the
//! user has already removed (or a path already cleared by a file operation).
use super::*;

impl AppStateDb {
    pub(crate) fn heal_auto_tag_assignments(
        &self,
        paths: &[PathBuf],
        rule_tag: i64,
        ids: &[i64],
    ) -> Option<bool> {
        let paths = storage_path_texts(paths);
        let mut db = self.writer.lock().ok()?;
        let result = (|| -> rusqlite::Result<bool> {
            let tx = db.transaction()?;
            let mut all_present = true;
            {
                let mut exists = tx.prepare(
                    "SELECT EXISTS(SELECT 1 FROM file_tag_assignments WHERE file_path = ?1 AND tag_id = ?2)",
                )?;
                let mut insert = tx.prepare(
                    "INSERT OR IGNORE INTO file_tag_assignments (file_path, tag_id)
                     SELECT ?1, ?2 WHERE EXISTS(SELECT 1 FROM file_tags WHERE id = ?2)",
                )?;
                for path in paths {
                    let present: bool =
                        exists.query_row(params![path, rule_tag], |row| row.get(0))?;
                    if !present {
                        all_present = false;
                        continue;
                    }
                    for id in ids {
                        insert.execute(params![path, id])?;
                    }
                }
            }
            tx.commit()?;
            Ok(all_present)
        })();
        result.ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_events_cannot_restore_removed_assignments_but_can_heal_ancestors() {
        let db = AppStateDb::new_in_memory().unwrap();
        let parent = db.create_tag("Parent", TagColor::Blue, None).unwrap();
        let child = db
            .create_tag("Child", TagColor::Green, Some(parent))
            .unwrap();
        let path = PathBuf::from(r"C:\Data\file.txt");
        assert!(db.assign_tags_batch(std::slice::from_ref(&path), &[child]));
        assert_eq!(
            db.heal_auto_tag_assignments(std::slice::from_ref(&path), child, &[child, parent]),
            Some(true)
        );
        assert_eq!(db.get_tag_assignment_paths(parent), vec![path.clone()]);
        assert!(db.unassign_tags_batch(std::slice::from_ref(&path), &[child, parent]));
        assert_eq!(
            db.heal_auto_tag_assignments(std::slice::from_ref(&path), child, &[child, parent]),
            Some(false)
        );
        assert!(db.try_get_all_tag_assignments().unwrap().is_empty());
    }
}
