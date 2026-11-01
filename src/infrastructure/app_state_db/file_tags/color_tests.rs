use super::*;

#[test]
fn custom_tag_colors_and_assignments_survive_reopening_the_database() {
    let directory = tempfile::tempdir().unwrap();
    let path = PathBuf::from(r"C:\Folder\file.txt");
    let color = TagColor::Custom([0, 128, 255]);
    let id;
    {
        let db = AppStateDb::new(directory.path().to_path_buf()).unwrap();
        id = db.create_tag("Example", color).unwrap();
        assert!(db.assign_tag_batch(std::slice::from_ref(&path), id));
    }
    let db = AppStateDb::new(directory.path().to_path_buf()).unwrap();
    let tag = db
        .get_all_tags()
        .into_iter()
        .find(|tag| tag.id == id)
        .unwrap();
    assert_eq!(tag.color, color);
    assert_eq!(db.get_tag_assignment_paths(id), vec![path]);
}

#[test]
fn custom_tag_colors_can_be_created_and_updated_without_changing_assignments() {
    let db = AppStateDb::new_in_memory().unwrap();
    let color = TagColor::Custom([12, 34, 56]);
    let id = db.create_tag("Example", color).unwrap();
    let path = PathBuf::from(r"C:\Folder\file.txt");
    assert!(db.assign_tag_batch(std::slice::from_ref(&path), id));
    let original = db
        .get_all_tags()
        .into_iter()
        .find(|tag| tag.id == id)
        .unwrap();
    assert_eq!(original.color, color);

    for next_color in [TagColor::Custom([255, 0, 128]), TagColor::Blue, color] {
        assert!(db.update_tag_color(id, next_color));
        let loaded = db
            .get_all_tags()
            .into_iter()
            .find(|tag| tag.id == id)
            .unwrap();
        assert_eq!(loaded.color, next_color);
        assert_eq!(loaded.id, original.id);
        assert_eq!(loaded.name, original.name);
        assert_eq!(loaded.position, original.position);
        assert_eq!(db.get_tag_assignment_paths(id), vec![path.clone()]);
    }
}

#[test]
fn legacy_colors_keep_their_stored_names_and_rgb_values() {
    let db = AppStateDb::new_in_memory().unwrap();
    for color in TagColor::expanded_palette() {
        let name = color.as_db_str();
        let id = db.create_tag(&format!("Example {name}"), color).unwrap();
        let stored: String = db
            .reader
            .lock()
            .unwrap()
            .query_row("SELECT color FROM file_tags WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(stored, name);
        let loaded = db
            .get_all_tags()
            .into_iter()
            .find(|tag| tag.id == id)
            .unwrap();
        assert_eq!(loaded.color, color);
        assert_eq!(loaded.color.to_color32(), color.to_color32());
    }
}
