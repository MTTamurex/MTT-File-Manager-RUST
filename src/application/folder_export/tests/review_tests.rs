use super::*;

#[test]
fn cached_sizes_match_windows_paths_case_insensitively() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("folder");
    fs::create_dir(&folder).unwrap();
    let mut options = options(dir.path());
    let cached_path = dir.path().join("FOLDER");
    options.folder_sizes.insert(
        crate::domain::file_tag::normalize_tag_path_key(&cached_path),
        123,
    );
    assert_eq!(output(&options).0.rows[0].size, Some(123));
}

#[test]
fn files_only_shallow_export_does_not_calculate_excluded_directories() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("folder");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("file.txt"), b"123456789").unwrap();
    let mut options = options(dir.path());
    options.contents = Contents::Files;
    options.calculate_sizes = true;
    options.columns.push(ExportColumn {
        field: Field::Column(ColumnId::Size),
        label: "Size".into(),
        enabled: true,
    });
    let (listing, _) = output(&options);
    assert_eq!(listing.rows[0].size, None);
    assert!(!listing.rows[0].included);
}

#[test]
fn text_tree_keeps_subsequent_columns_aligned_after_indentation() {
    let mut options = options(std::path::Path::new("."));
    options.format = Format::Text;
    options.recursive = true;
    let row = |depth, directory, name: &str| Row {
        depth,
        directory,
        included: true,
        name: name.into(),
        cells: vec![name.into(), "END".into()],
        size: None,
        traversable: directory,
        complete: true,
    };
    let listing = Listing {
        rows: vec![row(0, true, "folder"), row(1, false, "long-file-name.txt")],
        skipped: 0,
    };
    let mut bytes = Vec::new();
    render::render(&mut bytes, &options, &listing, &Progress::default()).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let offsets: Vec<_> = text.lines().filter_map(|line| line.find("END")).collect();
    assert_eq!(offsets.len(), 2);
    assert_eq!(offsets[0], offsets[1]);
}

#[test]
fn clipboard_text_has_no_file_encoding_marker() {
    let dir = tempfile::tempdir().unwrap();
    let mut options = options(dir.path());
    for format in [Format::Csv, Format::Tsv] {
        options.format = format;
        let completed = worker::run(
            &options,
            worker::Destination::Clipboard,
            &Progress::default(),
        )
        .unwrap();
        assert!(completed.clipboard.unwrap().starts_with("\"Name\""));
    }
}

#[test]
fn explicitly_selected_descendants_of_directory_links_are_not_discarded() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("folder");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("file.txt"), b"data").unwrap();
    let link = dir.path().join("link");
    if let Err(error) = std::os::windows::fs::symlink_dir(&folder, &link) {
        assert_eq!(
            error.raw_os_error(),
            Some(1314),
            "unexpected symlink creation failure"
        );
        return;
    }
    let mut options = options(dir.path());
    options.recursive = true;
    options.selection = Some(vec![link.join("file.txt"), link]);
    let listing = output(&options).0;
    assert_eq!(listing.rows.len(), 2);
    assert!(listing.rows.iter().any(|row| row.name == "file.txt"));
}
