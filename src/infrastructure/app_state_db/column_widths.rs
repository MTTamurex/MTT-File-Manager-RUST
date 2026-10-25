//! Sparse manual edits, stored separately from global column layout preferences.
use super::{AppStateDb, AppStateWriteError};
use crate::domain::{
    column_widths::{valid_width, WidthLocation},
    list_columns::ColumnId,
};

impl AppStateDb {
    pub(crate) fn load_column_widths(
        &self,
        location: &WidthLocation,
    ) -> rusqlite::Result<Vec<(ColumnId, f32)>> {
        let reader = self
            .reader
            .lock()
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        let mut statement = reader
            .prepare("SELECT column_id, width FROM folder_column_widths WHERE location = ?1")?;
        let rows = statement.query_map([&location.0], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        })?;
        let mut widths = Vec::new();
        for row in rows {
            let (id, width) = row?;
            if let Ok(id) = serde_json::from_str::<ColumnId>(&id) {
                let width = width as f32;
                if valid_width(id, width) {
                    widths.push((id, width));
                }
            }
        }
        Ok(widths)
    }

    pub(crate) fn save_column_widths(
        &self,
        changes: &[(WidthLocation, ColumnId, f32)],
    ) -> Result<(), AppStateWriteError> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| AppStateWriteError::WriterLockPoisoned)?;
        let transaction = writer.transaction()?;
        {
            let mut statement = transaction.prepare_cached("INSERT INTO folder_column_widths (location, column_id, width)
                VALUES (?1, ?2, ?3) ON CONFLICT(location, column_id) DO UPDATE SET width = excluded.width")?;
            for (location, id, width) in changes {
                if !valid_width(*id, *width) {
                    continue;
                }
                let id = serde_json::to_string(id).expect("column identity serializes");
                statement.execute(rusqlite::params![location.0, id, f64::from(*width)])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::list_columns::PropertyId;
    #[test]
    fn sparse_widths_round_trip_all_column_types_without_affecting_other_locations() {
        let db = AppStateDb::new_in_memory().unwrap();
        let a = WidthLocation::for_view(r"C:\Example\Folder", false, false).unwrap();
        let parent = WidthLocation::for_view(r"C:\Example", false, false).unwrap();
        let property = ColumnId::Property(PropertyId {
            fmtid: u128::MAX,
            pid: 42,
        });
        db.save_column_widths(&[
            (a.clone(), ColumnId::Name, 500.0),
            (a.clone(), property, 260.0),
        ])
        .unwrap();
        db.save_column_widths(&[(a.clone(), ColumnId::Name, 600.0)])
            .unwrap();
        let widths: std::collections::HashMap<_, _> =
            db.load_column_widths(&a).unwrap().into_iter().collect();
        assert_eq!(widths.len(), 2);
        assert_eq!(widths[&property], 260.0);
        assert_eq!(widths[&ColumnId::Name], 600.0);
        assert!(db.load_column_widths(&parent).unwrap().is_empty());
        assert!(db
            .load_column_widths(&WidthLocation::for_view("", true, false).unwrap())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn invalid_widths_are_not_persisted_and_small_profiles_use_little_database_space() {
        let db = AppStateDb::new_in_memory().unwrap();
        let location = WidthLocation("example".into());
        db.save_column_widths(&[
            (location.clone(), ColumnId::Name, f32::NAN),
            (location.clone(), ColumnId::Size, -1.0),
        ])
        .unwrap();
        assert!(db.load_column_widths(&location).unwrap().is_empty());
        let changes: Vec<_> = (0..1000)
            .flat_map(|folder| {
                (0..20).map(move |column| {
                    (
                        WidthLocation(format!("location:c:\\example\\folder_{folder}")),
                        ColumnId::Property(PropertyId {
                            fmtid: 1,
                            pid: column,
                        }),
                        160.0,
                    )
                })
            })
            .collect();
        db.save_column_widths(&changes).unwrap();
        let reader = db.reader.lock().unwrap();
        let pages: u64 = reader
            .query_row("PRAGMA page_count", [], |row| row.get(0))
            .unwrap();
        let page_size: u64 = reader
            .query_row("PRAGMA page_size", [], |row| row.get(0))
            .unwrap();
        assert!(
            pages * page_size < 8 * 1024 * 1024,
            "one thousand sparse profiles should use less than 8 MiB"
        );
    }
}
