use super::*;
use std::path::{Path, PathBuf};

fn rule_folder(db: &AppStateDb, id: i64) -> PathBuf {
    db.get_auto_tag_rules()
        .into_iter()
        .find(|rule| rule.id == id)
        .map(|rule| rule.folder_path)
        .unwrap_or_default()
}

fn rule_subfolders(db: &AppStateDb, id: i64) -> Option<bool> {
    db.get_auto_tag_rules()
        .into_iter()
        .find(|rule| rule.id == id)
        .map(|rule| rule.include_subfolders)
}

#[test]
fn auto_tag_rule_crud_and_tag_cascade() {
    let db = AppStateDb::new_in_memory().unwrap();
    let photos = db
        .create_tag("Fotos", crate::domain::file_tag::TagColor::Blue, None)
        .unwrap();
    let musicas = db
        .create_tag("Músicas", crate::domain::file_tag::TagColor::Red, None)
        .unwrap();

    let folder = Path::new(r"C:\Library\Fotos");
    let id = db.create_auto_tag_rule(folder, photos, true).unwrap();
    assert!(db.set_auto_tag_rule_enabled(id, false));
    assert!(!db.set_auto_tag_rule_enabled(id + 1000, true));

    let rules = db.get_auto_tag_rules();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].id, id);
    assert_eq!(rules[0].tag_id, photos);
    assert!(!rules[0].enabled);
    assert!(rules[0].include_subfolders);

    assert!(db.update_auto_tag_rule(id, Path::new(r"C:\Outra"), musicas, false));
    let rules = db.get_auto_tag_rules();
    assert_eq!(rules[0].tag_id, musicas);
    assert!(!rules[0].include_subfolders);
    assert_eq!(rule_folder(&db, id), PathBuf::from(r"C:\Outra"));

    // A second rule on the same folder with another tag is independent.
    let other = db
        .create_auto_tag_rule(Path::new(r"C:\Outra"), photos, true)
        .unwrap();
    assert_eq!(db.get_auto_tag_rules().len(), 2);

    // Toggling the subfolder option only affects the target rule.
    assert!(db.set_auto_tag_rule_include_subfolders(other, false));
    assert_eq!(rule_subfolders(&db, other), Some(false));
    assert_eq!(rule_subfolders(&db, id), Some(false));

    // Deleting the tag cascades to its rules (handled in delete_tag_subtree;
    // foreign keys are not enforced by pragma). The other rule survives.
    assert!(db.delete_tag_subtree(&[musicas]) > 0);
    let remaining = db.get_auto_tag_rules();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, other);

    assert!(db.delete_auto_tag_rule(other));
    assert!(db.get_auto_tag_rules().is_empty());
}

#[test]
fn rename_auto_tag_rule_folder_updates_the_path() {
    let db = AppStateDb::new_in_memory().unwrap();
    let tag = db
        .create_tag("Fotos", crate::domain::file_tag::TagColor::Blue, None)
        .unwrap();
    let id = db
        .create_auto_tag_rule(Path::new(r"C:\Antiga"), tag, true)
        .unwrap();

    assert!(db.rename_auto_tag_rule_folder(id, Path::new(r"D:\Nova")));
    assert_eq!(rule_folder(&db, id), PathBuf::from(r"D:\Nova"));
}

#[test]
fn auto_tag_rules_reject_unknown_or_deleted_tags() {
    let db = AppStateDb::new_in_memory().unwrap();
    let folder = Path::new(r"C:\Library");
    assert!(db.create_auto_tag_rule(folder, i64::MAX, true).is_none());
    let tag = db
        .create_tag("Test tag", crate::domain::file_tag::TagColor::Blue, None)
        .unwrap();
    let id = db.create_auto_tag_rule(folder, tag, true).unwrap();
    assert!(!db.update_auto_tag_rule(id, folder, i64::MAX, true));
    assert_eq!(db.get_auto_tag_rules()[0].tag_id, tag);
    assert_eq!(db.delete_tag_subtree(&[tag]), 1);
    assert!(db.create_auto_tag_rule(folder, tag, true).is_none());
    // A queued worker write must not resurrect orphan assignments after deletion.
    db.assign_tags_batch(&[folder.join("file.txt")], &[tag]);
    assert!(db.get_all_tag_assignments().is_empty());
}
