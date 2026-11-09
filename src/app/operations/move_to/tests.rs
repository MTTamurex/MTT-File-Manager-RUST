use super::*;

#[test]
fn move_to_accepts_single_and_mixed_filesystem_origins() {
    let paths = [
        PathBuf::from(r"C:\Folder\file.txt"),
        PathBuf::from(r"D:\Other\folder"),
        PathBuf::from(r"\\server\share\folder\file.txt"),
    ];
    assert!(supports_sources(&paths));
    for path in &paths {
        assert!(supports_sources(std::slice::from_ref(path)));
    }
}

#[test]
fn move_to_excludes_roots_virtual_views_and_relative_paths() {
    assert!(!supports_sources(&[]));
    for path in [
        r"C:\",
        r"\\server",
        r"\\server\share",
        "shell:RecycleBinFolder",
        "::computer::",
        "::recycle_bin::",
        "::tag::1",
        "file.txt",
    ] {
        let path = PathBuf::from(path);
        assert!(!supports_sources(std::slice::from_ref(&path)));
        assert!(!supports_sources(&[
            PathBuf::from(r"C:\Folder\file.txt"),
            path
        ]));
    }
}

#[test]
fn move_to_blocks_archive_entries_but_accepts_the_archive_itself() {
    let directory = tempfile::tempdir().unwrap();
    let archive = directory.path().join("archive.zip");
    std::fs::write(&archive, b"archive").unwrap();
    assert!(supports_sources(std::slice::from_ref(&archive)));
    assert!(!supports_sources(&[archive.join("file.txt")]));
    let folder = directory.path().join("folder.zip");
    std::fs::create_dir(&folder).unwrap();
    assert!(supports_sources(&[folder.join("file.txt")]));
}

#[test]
fn move_to_validates_destination_against_every_origin() {
    let sources = [
        PathBuf::from(r"C:\Folder\file.txt"),
        PathBuf::from(r"D:\Other"),
    ];
    assert!(paste_target_is_valid_for_sources(
        &sources,
        Path::new(r"C:\Destination")
    ));
    assert!(!paste_target_is_valid_for_sources(
        &sources,
        Path::new(r"D:\Other")
    ));
    assert!(!paste_target_is_valid_for_sources(
        &sources,
        Path::new(r"d:\other\Child")
    ));
    assert!(!paste_target_is_valid_for_sources(
        &sources,
        Path::new(r"\\?\D:\Other\Child")
    ));
    assert!(paste_target_is_valid_for_sources(
        &sources,
        Path::new(r"D:\OtherFolder")
    ));
}

#[test]
fn move_to_ignores_same_folder_sources_without_losing_other_origins() {
    let paths = [
        PathBuf::from(r"C:\Folder\first.txt"),
        PathBuf::from(r"D:\Other\second.txt"),
        PathBuf::from(r"\\?\C:\Folder\third.txt"),
    ];
    assert_eq!(
        sources_outside_destination(&paths, Path::new(r"c:\folder\")),
        [paths[1].clone()]
    );
    assert!(sources_outside_destination(&paths[..1], Path::new(r"C:\Folder")).is_empty());
}

#[test]
fn move_to_compares_unc_and_unicode_destinations_consistently() {
    let paths = [
        PathBuf::from(r"\\?\UNC\server\share\Folder\file.txt"),
        PathBuf::from(r"C:\Pasta Ação\file.txt"),
    ];
    assert_eq!(
        sources_outside_destination(&paths, Path::new(r"\\server\share\folder")),
        [paths[1].clone()]
    );
    assert_eq!(
        sources_outside_destination(&paths, Path::new(r"c:\pasta ação")),
        [paths[0].clone()]
    );
}
