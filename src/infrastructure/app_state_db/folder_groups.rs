use super::{AppStateDb, AppStateWriteError};
use crate::domain::folder_group::{FolderGroup, SavedGroupTabs};
use crate::tabs::{MAX_PANEL_TABS, MAX_TABS};
use rusqlite::{params, TransactionBehavior};

const MAX_GROUP_TABS_JSON_BYTES: usize = 1024 * 1024;

fn parse_saved_tabs(value: Option<String>) -> Option<SavedGroupTabs> {
    let mut saved: SavedGroupTabs = serde_json::from_str(
        value
            .filter(|json| json.len() <= MAX_GROUP_TABS_JSON_BYTES)?
            .as_str(),
    )
    .ok()?;
    saved.sanitize();
    if saved.tabs.len() > MAX_TABS
        || saved.tabs.iter().any(|tab| {
            tab.dual_panel
                .as_ref()
                .is_some_and(|dual| dual.left.tabs.len() + dual.right.tabs.len() > MAX_PANEL_TABS)
        })
    {
        return None;
    }
    (!saved.tabs.is_empty()).then_some(saved)
}

impl AppStateDb {
    /// Load saved tab groups and their paths in display and tab order. [READER]
    pub fn get_all_folder_groups(&self) -> Vec<FolderGroup> {
        let Ok(db) = self.reader.lock() else {
            log::error!("[GROUPS] Failed to acquire reader lock");
            return Vec::new();
        };
        let groups = match db.prepare(
            "SELECT id, name, position, tabs_json
             FROM folder_groups ORDER BY position ASC, id ASC",
        ) {
            Ok(mut statement) => match statement.query_map([], |row| {
                Ok(FolderGroup {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    position: row.get(2)?,
                    paths: Vec::new(),
                    saved_tabs: parse_saved_tabs(row.get(3)?),
                })
            }) {
                Ok(rows) => rows.collect::<rusqlite::Result<Vec<_>>>(),
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        };
        let Ok(mut groups) = groups else {
            log::error!("[GROUPS] Failed to load saved groups");
            return Vec::new();
        };

        for group in &mut groups {
            let paths = db
                .prepare(
                    "SELECT path FROM folder_group_paths
                     WHERE group_id = ? ORDER BY position ASC",
                )
                .and_then(|mut statement| {
                    statement
                        .query_map([group.id], |row| row.get::<_, String>(0))?
                        .collect::<rusqlite::Result<Vec<_>>>()
                });
            match paths {
                Ok(paths) => group.paths = paths,
                Err(_) => {
                    log::error!("[GROUPS] Failed to load saved group paths");
                    return Vec::new();
                }
            }
        }
        groups
    }

    /// Save a named group, its ordered paths and optional workspace state in one
    /// transaction. [WRITER]
    pub fn save_folder_group(
        &self,
        name: &str,
        paths: &[String],
        saved_tabs: Option<&SavedGroupTabs>,
    ) -> Result<(), AppStateWriteError> {
        // Never report success for workspace state the reader would discard.
        let tabs_json = saved_tabs
            .map(|saved| {
                let json = serde_json::to_string(saved)
                    .map_err(|_| AppStateWriteError::InvalidFolderGroupState)?;
                if parse_saved_tabs(Some(json.clone())).is_none() {
                    return Err(AppStateWriteError::InvalidFolderGroupState);
                }
                Ok(json)
            })
            .transpose()?;
        let mut db = self
            .writer
            .lock()
            .map_err(|_| AppStateWriteError::WriterLockPoisoned)?;
        let transaction = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let position: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(position), -1) + 1
             FROM (
                 SELECT position FROM pinned_folders
                 UNION ALL
                 SELECT position FROM folder_groups
             )",
            [],
            |row| row.get(0),
        )?;
        transaction.execute(
            "INSERT INTO folder_groups (name, position, tabs_json) VALUES (?, ?, ?)",
            params![name, position, tabs_json],
        )?;
        let group_id = transaction.last_insert_rowid();
        for (position, path) in paths.iter().enumerate() {
            transaction.execute(
                "INSERT INTO folder_group_paths (group_id, path, position)
                 VALUES (?, ?, ?)",
                params![group_id, path, position as i64],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Remove a saved group and all its stored paths in one transaction. [WRITER]
    pub fn delete_folder_group(&self, group_id: i64) -> Result<(), AppStateWriteError> {
        let mut db = self
            .writer
            .lock()
            .map_err(|_| AppStateWriteError::WriterLockPoisoned)?;
        let transaction = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "DELETE FROM folder_group_paths WHERE group_id = ?",
            [group_id],
        )?;
        transaction.execute("DELETE FROM folder_groups WHERE id = ?", [group_id])?;
        transaction.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::quick_access_item::QuickAccessItem;

    #[test]
    fn saved_groups_round_trip_in_order_and_delete_with_their_paths() {
        let db = AppStateDb::new_in_memory().unwrap();
        let paths = vec![
            r"C:\First".to_owned(),
            r"D:\Second".to_owned(),
            r"E:\Third".to_owned(),
        ];

        db.save_folder_group("Work", &paths, None).unwrap();
        db.save_folder_group("Media", &paths[..2], None).unwrap();

        let groups = db.get_all_folder_groups();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].name, "Work");
        assert_eq!(groups[0].paths, paths);
        assert_eq!(groups[0].position, 0);
        assert!(groups[0].saved_tabs.is_none());
        assert_eq!(groups[1].name, "Media");
        assert_eq!(groups[1].paths, paths[..2]);

        db.delete_folder_group(groups[0].id).unwrap();
        let remaining = db.get_all_folder_groups();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].name, "Media");
    }

    #[test]
    fn group_names_are_unique_without_case_sensitivity() {
        let db = AppStateDb::new_in_memory().unwrap();
        db.save_folder_group("Work", &[r"C:\Folder".to_owned()], None)
            .unwrap();

        assert!(db
            .save_folder_group("work", &[r"D:\Folder".to_owned()], None)
            .is_err());
        let groups = db.get_all_folder_groups();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].paths, [r"C:\Folder"]);
    }

    #[test]
    fn combined_quick_access_order_round_trips_and_new_groups_append() {
        let db = AppStateDb::new_in_memory().unwrap();
        db.save_pinned_folder("folder-a", "A", 0);
        db.save_pinned_folder("folder-b", "B", 1);
        db.save_folder_group("Group A", &["target-a".to_owned()], None)
            .unwrap();
        let group_a = db.get_all_folder_groups()[0].id;

        let order = [
            QuickAccessItem::PinnedFolder("folder-a".to_owned()),
            QuickAccessItem::FolderGroup(group_a),
            QuickAccessItem::PinnedFolder("folder-b".to_owned()),
        ];
        db.update_quick_access_positions(&order).unwrap();

        let folders = db.get_all_pinned_folders();
        let groups = db.get_all_folder_groups();
        assert_eq!(
            folders
                .iter()
                .map(|folder| (folder.path.as_str(), folder.position))
                .collect::<Vec<_>>(),
            [("folder-a", 0), ("folder-b", 2)]
        );
        assert_eq!(groups[0].position, 1);

        db.save_folder_group("Group B", &["target-b".to_owned()], None)
            .unwrap();
        let groups = db.get_all_folder_groups();
        assert_eq!(groups[1].name, "Group B");
        assert_eq!(groups[1].position, 3);
    }

    #[test]
    fn saved_group_workspace_state_round_trips_and_invalid_state_is_ignored() {
        use crate::domain::folder_group::{
            SavedGroupDualPanel, SavedGroupPanel, SavedGroupPanelTab, SavedGroupPanelTabs,
            SavedGroupTab,
        };

        let db = AppStateDb::new_in_memory().unwrap();
        let saved = SavedGroupTabs {
            active_tab: 0,
            tabs: vec![SavedGroupTab {
                path: r"D:\Videos\Folder".to_owned(),
                view_mode: Some("miller".to_owned()),
                dual_panel: Some(SavedGroupDualPanel {
                    active: SavedGroupPanel::Right,
                    split_ratio: 0.65,
                    left: SavedGroupPanelTabs {
                        active: 1,
                        tabs: vec![
                            SavedGroupPanelTab {
                                path: r"C:\Users\user\Documents\Project\FolderA".to_owned(),
                                view_mode: Some("list".to_owned()),
                                pinned: true,
                            },
                            SavedGroupPanelTab {
                                path: r"C:\Users\user\Documents\Project\FolderB".to_owned(),
                                view_mode: Some("column_list".to_owned()),
                                pinned: false,
                            },
                        ],
                    },
                    right: SavedGroupPanelTabs {
                        active: 0,
                        tabs: vec![SavedGroupPanelTab {
                            path: r"D:\Videos\Folder".to_owned(),
                            view_mode: Some("grid".to_owned()),
                            pinned: false,
                        }],
                    },
                }),
            }],
        };

        db.save_folder_group("Workspace", &[r"D:\Videos\Folder".to_owned()], Some(&saved))
            .unwrap();

        let groups = db.get_all_folder_groups();
        assert_eq!(groups[0].saved_tabs.as_ref(), Some(&saved));

        let db = AppStateDb::new_in_memory().unwrap();
        db.save_folder_group(
            "Broken",
            &[r"C:\Users\user\Documents\Project".to_owned()],
            Some(&SavedGroupTabs {
                active_tab: 0,
                tabs: Vec::new(),
            }),
        )
        .unwrap_err();
        assert!(db.get_all_folder_groups().is_empty());
        assert!(parse_saved_tabs(Some("{invalid".to_owned())).is_none());
    }

    #[test]
    fn oversized_workspace_is_rejected_without_creating_a_mono_group() {
        use crate::domain::folder_group::SavedGroupTab;

        let db = AppStateDb::new_in_memory().unwrap();
        let saved = SavedGroupTabs {
            active_tab: 0,
            tabs: vec![SavedGroupTab {
                path: "folder".repeat(MAX_GROUP_TABS_JSON_BYTES / 6 + 1),
                view_mode: Some("grid".to_owned()),
                dual_panel: None,
            }],
        };
        assert!(matches!(
            db.save_folder_group("Group", &["folder".to_owned()], Some(&saved)),
            Err(AppStateWriteError::InvalidFolderGroupState)
        ));
        assert!(db.get_all_folder_groups().is_empty());
    }
}
