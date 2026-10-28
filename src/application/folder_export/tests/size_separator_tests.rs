use super::*;

fn sized_options(root: &std::path::Path) -> Options {
    let mut options = options(root);
    options.columns.push(ExportColumn {
        field: Field::Column(ColumnId::Size),
        label: "Size".into(),
        enabled: true,
    });
    options
}

#[test]
fn cached_folder_sizes_are_exported_in_every_format_without_calculating() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("folder");
    fs::create_dir(&folder).unwrap();
    let mut options = sized_options(dir.path());
    options.folder_sizes.insert(
        crate::domain::file_tag::normalize_tag_path_key(&folder),
        12345,
    );
    for format in [Format::Csv, Format::Tsv, Format::Text, Format::Html] {
        options.format = format;
        let (listing, text) = output(&options);
        assert_eq!(listing.rows[0].size, Some(12345));
        assert!(text.contains("12345"));
    }
}

#[test]
fn missing_sizes_stay_empty_until_calculation_is_enabled() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("folder");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("file.txt"), b"1234567").unwrap();
    let mut options = sized_options(dir.path());
    assert_eq!(output(&options).0.rows[0].size, None);
    options.calculate_sizes = true;
    let (listing, text) = output(&options);
    assert_eq!(listing.rows[0].size, Some(7));
    assert!(text.contains("\"7\""));
}

#[test]
fn empty_calculated_folders_export_zero_not_unknown() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("folder")).unwrap();
    let mut options = sized_options(dir.path());
    options.calculate_sizes = true;
    for recursive in [false, true] {
        options.recursive = recursive;
        assert_eq!(output(&options).0.rows[0].size, Some(0));
    }
}

#[test]
fn recursive_sizes_sum_descendants_once_and_resort_complete_subtrees() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("folder-small").join("nested")).unwrap();
    fs::create_dir(dir.path().join("folder-large")).unwrap();
    fs::write(dir.path().join("folder-small").join("file.txt"), b"123").unwrap();
    fs::write(
        dir.path()
            .join("folder-small")
            .join("nested")
            .join("file.txt"),
        b"1234",
    )
    .unwrap();
    fs::write(
        dir.path().join("folder-large").join("file.txt"),
        b"123456789",
    )
    .unwrap();
    let mut options = sized_options(dir.path());
    options.recursive = true;
    options.calculate_sizes = true;
    options.sort = SortMode::Size;
    for descending in [false, true] {
        options.descending = descending;
        let listing = output(&options).0;
        let small = listing
            .rows
            .iter()
            .find(|row| row.name == "folder-small")
            .unwrap();
        let large = listing
            .rows
            .iter()
            .find(|row| row.name == "folder-large")
            .unwrap();
        assert_eq!(small.size, Some(7));
        assert_eq!(large.size, Some(9));
        assert_eq!(listing.rows[0].size, Some(if descending { 9 } else { 7 }));
        assert_eq!(listing.rows[1].depth, 1);
        let nested = listing
            .rows
            .iter()
            .find(|row| row.name == "nested")
            .unwrap();
        assert_eq!(nested.size, Some(4));
    }
}

#[test]
fn cached_sizes_are_preserved_and_used_for_sorting() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("folder-a");
    let b = dir.path().join("folder-b");
    fs::create_dir(&a).unwrap();
    fs::create_dir(&b).unwrap();
    let mut options = sized_options(dir.path());
    options.sort = SortMode::Size;
    options
        .folder_sizes
        .insert(crate::domain::file_tag::normalize_tag_path_key(&a), 200);
    options
        .folder_sizes
        .insert(crate::domain::file_tag::normalize_tag_path_key(&b), 100);
    assert_eq!(output(&options).0.rows[0].name, "folder-b");
    options.recursive = true;
    options.calculate_sizes = true;
    assert_eq!(output(&options).0.rows[0].size, Some(100));
}

#[test]
fn partial_recursive_sizes_do_not_look_like_complete_totals() {
    let mut options = sized_options(std::path::Path::new("."));
    options.recursive = true;
    options.calculate_sizes = true;
    let row = |depth, complete| Row {
        depth,
        directory: true,
        included: true,
        name: "folder".into(),
        cells: vec!["folder".into(), "folder".into(), "".into()],
        size: None,
        traversable: true,
        complete,
    };
    let mut rows = vec![row(0, true), row(1, false)];
    sizes::finish(&mut rows, &options, &Progress::default()).unwrap();
    assert_eq!(rows[0].size, None);
    assert_eq!(rows[1].size, None);
}

#[test]
fn selectable_csv_separators_apply_to_headers_and_records_but_not_tsv() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("file.txt"), b"").unwrap();
    let mut options = options(dir.path());
    for separator in [CsvSeparator::Comma, CsvSeparator::Semicolon] {
        options.csv_separator = separator;
        let text = output(&options).1;
        assert!(text.contains(&format!("\"Name\"{}\"Relative path\"", separator.text())));
        assert!(text.contains(&format!("\"file.txt\"{}\"file.txt\"", separator.text())));
    }
    options.format = Format::Tsv;
    assert!(output(&options).1.contains("\"Name\"\t\"Relative path\""));
}

#[test]
fn cancelled_size_scans_abort_instead_of_exporting_partial_totals() {
    let dir = tempfile::tempdir().unwrap();
    let mut options = sized_options(dir.path());
    options.calculate_sizes = true;
    let progress = Progress::default();
    progress.cancelled.store(true, Ordering::Relaxed);
    let mut skipped = 0;
    assert!(matches!(
        sizes::resolve(dir.path(), true, &options, &progress, &mut skipped),
        Err(ExportError::Cancelled)
    ));
}

#[test]
fn cached_link_sizes_do_not_inflate_parent_totals() {
    let mut options = sized_options(std::path::Path::new("."));
    options.recursive = true;
    options.calculate_sizes = true;
    let mut rows = vec![
        Row {
            depth: 0,
            directory: true,
            included: true,
            name: "folder".into(),
            cells: vec!["folder".into(), "folder".into(), "".into()],
            size: None,
            traversable: true,
            complete: true,
        },
        Row {
            depth: 1,
            directory: true,
            included: true,
            name: "link".into(),
            cells: vec!["link".into(), "link".into(), "123".into()],
            size: Some(123),
            traversable: false,
            complete: true,
        },
    ];
    sizes::finish(&mut rows, &options, &Progress::default()).unwrap();
    assert_eq!(rows[0].size, Some(0));
    assert_eq!(rows[1].size, Some(123));
}
