use super::*;
use std::fs;

mod review_tests;
mod size_separator_tests;

fn options(root: &std::path::Path) -> Options {
    Options {
        root: root.to_owned(),
        selection: None,
        recursive: false,
        contents: Contents::Both,
        format: Format::Csv,
        csv_separator: CsvSeparator::Comma,
        calculate_sizes: false,
        folder_sizes: Default::default(),
        tree: true,
        columns: vec![
            ExportColumn {
                field: Field::Column(ColumnId::Name),
                label: "Name".into(),
                enabled: true,
            },
            ExportColumn {
                field: Field::RelativePath,
                label: "Relative path".into(),
                enabled: true,
            },
        ],
        sort: SortMode::Name,
        descending: false,
        folders: FoldersPosition::First,
        date_format: DateFormat::YearMonthDay,
        tags: Arc::new(FxHashMap::default()),
        tag_names: FxHashMap::default(),
    }
}

fn output(options: &Options) -> (Listing, String) {
    let progress = Progress::default();
    let listing = collect::collect(options, &progress).unwrap();
    let mut bytes = Vec::new();
    render::render(&mut bytes, options, &listing, &progress).unwrap();
    (listing, String::from_utf8(bytes).unwrap())
}

#[test]
fn all_reads_disk_including_hidden_and_unfiltered_entries() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("file10.txt"), "").unwrap();
    fs::write(dir.path().join("file2.txt"), "").unwrap();
    fs::write(dir.path().join(".hidden.txt"), "").unwrap();
    let (listing, _) = output(&options(dir.path()));
    assert_eq!(listing.rows.len(), 3);
    assert_eq!(listing.rows[1].name, "file2.txt");
    assert_eq!(listing.rows[2].name, "file10.txt");
}

#[test]
fn selected_folders_recurse_only_when_requested_and_deduplicate_descendants() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("folder");
    fs::create_dir(&folder).unwrap();
    let file = folder.join("file.txt");
    fs::write(&file, "data").unwrap();
    fs::write(dir.path().join("other.txt"), "").unwrap();
    let mut options = options(dir.path());
    options.selection = Some(vec![folder.clone()]);
    assert_eq!(output(&options).0.rows.len(), 1);
    options.selection = Some(vec![folder, file]);
    options.recursive = true;
    let (listing, text) = output(&options);
    assert_eq!(listing.rows.len(), 2);
    assert_eq!(listing.rows[1].depth, 1);
    assert!(text.contains("folder\\file.txt"));
}

#[test]
fn file_filter_still_traverses_folders_and_folder_filter_excludes_files() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("folder")).unwrap();
    fs::write(dir.path().join("folder").join("file.txt"), "").unwrap();
    let mut options = options(dir.path());
    options.recursive = true;
    options.contents = Contents::Files;
    let (listing, csv) = output(&options);
    assert_eq!(listing.rows.iter().filter(|row| row.included).count(), 1);
    assert!(csv.contains("file.txt"));
    options.contents = Contents::Folders;
    let (_, csv) = output(&options);
    assert!(!csv.contains("file.txt"));
}

#[test]
fn formats_escape_unicode_quotes_and_spreadsheet_formulas() {
    let options = options(std::path::Path::new("."));
    let listing = Listing {
        skipped: 0,
        rows: vec![Row {
            size: None,
            traversable: false,
            complete: true,
            depth: 0,
            directory: false,
            included: true,
            name: "file.txt".into(),
            cells: vec!["=1+1".into(), "ação,\"value\"\nnext".into()],
        }],
    };
    for format in [Format::Csv, Format::Tsv] {
        let mut options = options.clone();
        options.format = format;
        let mut out = Vec::new();
        render::render(&mut out, &options, &listing, &Progress::default()).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with('\u{feff}'));
        assert!(text.contains("\"'=1+1\""));
        assert!(text.contains("ação,\"\"value\"\"\nnext"));
    }
}

#[test]
fn html_is_self_contained_escaped_and_balances_nested_details() {
    let mut options = options(std::path::Path::new("."));
    options.format = Format::Html;
    options.recursive = true;
    let row = |depth, directory, value: &str| Row {
        size: None,
        traversable: false,
        complete: true,
        depth,
        directory,
        included: true,
        name: value.into(),
        cells: vec![value.into(), "".into()],
    };
    let listing = Listing {
        skipped: 0,
        rows: vec![
            row(0, true, "folder"),
            row(1, true, "subfolder"),
            row(2, false, "<script>&\""),
            row(0, false, "file.txt"),
        ],
    };
    let mut out = Vec::new();
    render::render(&mut out, &options, &listing, &Progress::default()).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert_eq!(text.matches("<details open>").count(), 2);
    assert_eq!(text.matches("</details>").count(), 2);
    assert!(!text.contains("<script>"));
    assert!(text.contains("&lt;script&gt;&amp;&quot;"));
    assert!(text.ends_with("</body></html>\n"));
}

#[test]
fn cancellation_does_not_generate_a_report() {
    let dir = tempfile::tempdir().unwrap();
    let progress = Progress::default();
    progress.cancelled.store(true, Ordering::Relaxed);
    assert!(matches!(
        collect::collect(&options(dir.path()), &progress),
        Err(ExportError::Cancelled)
    ));
}

#[test]
fn empty_selection_is_not_interpreted_as_all() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("file.txt"), "").unwrap();
    let mut options = options(dir.path());
    options.selection = Some(Vec::new());
    assert!(output(&options).0.rows.is_empty());
}

#[test]
fn missing_selected_entry_is_counted_and_other_entries_survive() {
    let dir = tempfile::tempdir().unwrap();
    let existing = dir.path().join("file.txt");
    fs::write(&existing, "").unwrap();
    let mut options = options(dir.path());
    options.selection = Some(vec![existing, dir.path().join("missing.txt")]);
    let (listing, _) = output(&options);
    assert_eq!(listing.skipped, 1);
    assert_eq!(listing.rows.len(), 1);
}

#[test]
fn descending_size_sort_is_numeric_and_keeps_folder_position() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("folder")).unwrap();
    fs::write(dir.path().join("small.txt"), b"a").unwrap();
    fs::write(dir.path().join("large.txt"), b"abcdefghij").unwrap();
    let mut options = options(dir.path());
    options.sort = SortMode::Size;
    options.descending = true;
    let listing = output(&options).0;
    assert_eq!(
        listing
            .rows
            .iter()
            .map(|row| row.name.as_str())
            .collect::<Vec<_>>(),
        ["folder", "large.txt", "small.txt"]
    );
}

#[test]
fn text_tree_indents_names_and_plain_text_table_does_not() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("folder")).unwrap();
    fs::write(dir.path().join("folder").join("file.txt"), "").unwrap();
    let mut options = options(dir.path());
    options.format = Format::Text;
    options.recursive = true;
    assert!(output(&options)
        .1
        .lines()
        .any(|line| line.starts_with("  file.txt")));
    options.tree = false;
    assert!(output(&options)
        .1
        .lines()
        .any(|line| line.starts_with("file.txt")));
}

#[test]
fn selected_ancestor_does_not_hide_unrelated_siblings() {
    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().join("folder");
    fs::create_dir(&parent).unwrap();
    let child = parent.join("file.txt");
    fs::write(&child, "").unwrap();
    let sibling = dir.path().join("folder-other.txt");
    fs::write(&sibling, "").unwrap();
    let mut options = options(dir.path());
    options.selection = Some(vec![sibling, child, parent]);
    options.recursive = true;
    assert_eq!(output(&options).0.rows.len(), 3);
}

#[test]
fn directory_links_are_listed_as_folders_but_never_followed() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("folder");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("file.txt"), "").unwrap();
    let link = dir.path().join("link");
    // Windows requires Developer Mode or symlink privileges. Other tests remain unconditional.
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
    let listing = output(&options).0;
    assert_eq!(listing.rows.len(), 3);
    assert!(listing
        .rows
        .iter()
        .any(|row| row.name == "link" && row.directory));
    assert_eq!(
        listing
            .rows
            .iter()
            .filter(|row| row.name == "file.txt")
            .count(),
        1
    );
}

#[test]
fn file_output_requires_overwrite_permission_and_commits_complete_content() {
    let source = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    fs::write(source.path().join("file.txt"), "").unwrap();
    let path = destination.path().join("listing.csv");
    fs::write(&path, "original").unwrap();
    let options = options(source.path());
    let result = worker::run(
        &options,
        worker::Destination::File {
            path: path.clone(),
            overwrite: false,
        },
        &Progress::default(),
    );
    assert!(matches!(result, Err(ExportError::Exists)));
    assert_eq!(fs::read_to_string(&path).unwrap(), "original");
    worker::run(
        &options,
        worker::Destination::File {
            path: path.clone(),
            overwrite: true,
        },
        &Progress::default(),
    )
    .unwrap();
    let content = fs::read_to_string(&path).unwrap();
    assert!(content.starts_with('\u{feff}'));
    assert!(content.contains("file.txt"));
    assert_eq!(fs::read_dir(destination.path()).unwrap().count(), 1);
}

#[test]
fn cancelled_export_preserves_existing_destination() {
    let source = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let path = destination.path().join("listing.csv");
    fs::write(&path, "original").unwrap();
    let progress = Progress::default();
    progress.cancelled.store(true, Ordering::Relaxed);
    let result = worker::run(
        &options(source.path()),
        worker::Destination::File {
            path: path.clone(),
            overwrite: true,
        },
        &progress,
    );
    assert!(matches!(result, Err(ExportError::Cancelled)));
    assert_eq!(fs::read_to_string(&path).unwrap(), "original");
    assert_eq!(fs::read_dir(destination.path()).unwrap().count(), 1);
}

#[test]
fn all_formula_prefixes_are_neutralized_including_leading_whitespace() {
    let options = options(std::path::Path::new("."));
    for prefix in ["=", "+", "-", "@", "\t", "\r", "\n", "  ="] {
        let listing = Listing {
            skipped: 0,
            rows: vec![Row {
                size: None,
                traversable: false,
                complete: true,
                depth: 0,
                directory: false,
                included: true,
                name: "file.txt".into(),
                cells: vec![format!("{prefix}formula"), "".into()],
            }],
        };
        let mut bytes = Vec::new();
        render::render(&mut bytes, &options, &listing, &Progress::default()).unwrap();
        assert!(String::from_utf8(bytes)
            .unwrap()
            .contains(&format!("\"'{prefix}formula\"")));
    }
}

#[test]
fn tag_columns_export_values_and_sort_missing_values_last_in_both_directions() {
    let dir = tempfile::tempdir().unwrap();
    let tagged = dir.path().join("file2.txt");
    fs::write(&tagged, "").unwrap();
    fs::write(dir.path().join("file1.txt"), "").unwrap();
    let mut options = options(dir.path());
    options.tags = Arc::new(
        [(
            crate::domain::file_tag::normalize_tag_path_key(&tagged),
            vec![1],
        )]
        .into_iter()
        .collect(),
    );
    options.tag_names.insert(1, "Tag".into());
    options.sort = SortMode::Column(ColumnId::Tags);
    options.columns.push(ExportColumn {
        field: Field::Column(ColumnId::Tags),
        label: "Tags".into(),
        enabled: true,
    });
    for descending in [false, true] {
        options.descending = descending;
        let (listing, csv) = output(&options);
        assert_eq!(listing.rows[0].name, "file2.txt");
        assert!(csv.contains("\"Tag\""));
    }
}
