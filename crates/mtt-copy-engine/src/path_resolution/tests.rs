use super::*;

fn virtual_path(path: &Path) -> io::Result<Resolved> {
    resolve_with(path, |_| Err(io::Error::from_raw_os_error(1005)))
}

#[test]
fn unrecognized_volume_uses_normalized_volume_relative_names() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("folder");
    std::fs::create_dir(&folder).unwrap();
    let source = root.path().join("file-ç.txt");
    std::fs::write(&source, b"contents").unwrap();
    let input = folder.join("..").join("file-ç.txt");
    let resolved = virtual_path(&input).unwrap();
    assert!(resolved.virtual_volume);
    assert_eq!(
        std::fs::canonicalize(resolved.path).unwrap(),
        std::fs::canonicalize(source).unwrap()
    );
    let normal = resolve(root.path()).unwrap();
    assert!(!normal.virtual_volume);
    let extended = std::fs::canonicalize(root.path()).unwrap();
    let extended_resolved = virtual_path(&extended).unwrap();
    assert_eq!(
        std::fs::canonicalize(extended_resolved.path).unwrap(),
        extended
    );
}

#[test]
fn fallback_is_limited_to_1005_and_never_accepts_missing_files() {
    let root = tempfile::tempdir().unwrap();
    for code in [2, 3, 5, 32, 50] {
        let error = resolve_with(root.path(), |_| Err(io::Error::from_raw_os_error(code)))
            .err()
            .unwrap();
        assert_eq!(error.raw_os_error(), Some(code));
    }
    assert!(virtual_path(&root.path().join("missing.txt")).is_err());
}

#[test]
fn virtual_volume_preflight_and_staged_copy_preserve_contents() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("file.txt");
    let destination = root.path().join("target");
    std::fs::write(&source, b"contents").unwrap();
    std::fs::create_dir(&destination).unwrap();
    let progress = crate::Progress::default();
    let plan = crate::plan::build_with(
        std::slice::from_ref(&source),
        &destination,
        &progress,
        virtual_path,
    )
    .unwrap();
    assert_eq!(plan.files.len(), 1);
    crate::platform::copy_file(&plan.files[0], &progress).unwrap();
    assert_eq!(
        std::fs::read(destination.join("file.txt")).unwrap(),
        b"contents"
    );
    assert_eq!(std::fs::read(source).unwrap(), b"contents");
    assert_eq!(std::fs::read_dir(&destination).unwrap().count(), 1);
}

#[test]
fn directory_identity_checks_allow_separate_trees_and_mixed_resolution_modes() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let destination = root.path().join("target");
    std::fs::create_dir_all(source.join("empty")).unwrap();
    std::fs::create_dir(&destination).unwrap();
    std::fs::write(source.join("file.txt"), b"contents").unwrap();
    assert!(!contains_directory(&source, &destination).unwrap());
    for virtual_source in [false, true] {
        let plan = crate::plan::build_with(
            std::slice::from_ref(&source),
            &destination,
            &crate::Progress::default(),
            |path| {
                if (path == source) == virtual_source {
                    virtual_path(path)
                } else {
                    resolve(path)
                }
            },
        )
        .unwrap();
        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.directories.len(), 2);
    }
    assert_eq!(std::fs::read_dir(destination).unwrap().count(), 0);
}

#[test]
fn virtual_volume_still_rejects_self_recursive_and_existing_targets() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let child = source.join("child");
    std::fs::create_dir_all(&child).unwrap();
    let progress = crate::Progress::default();
    for destination in [&source, &child] {
        assert!(crate::plan::build_with(
            std::slice::from_ref(&source),
            destination,
            &progress,
            virtual_path
        )
        .is_err());
    }
    // Identity checks also detect the same directory under different casing.
    let alias = PathBuf::from(source.to_string_lossy().to_uppercase());
    assert!(contains_directory(&alias, &child).unwrap());
    let destination = root.path().join("target");
    std::fs::create_dir_all(destination.join("source")).unwrap();
    let error = crate::plan::build_with(&[source], &destination, &progress, virtual_path)
        .err()
        .unwrap();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read_dir(child).unwrap().count(), 0);
}

#[test]
fn virtual_volume_still_rejects_junction_ancestors() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target");
    let link = root.path().join("junction");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("file.txt"), b"contents").unwrap();
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(&target)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        virtual_path(&link.join("file.txt")).err().unwrap().kind(),
        io::ErrorKind::Unsupported
    );
    std::fs::remove_dir(link).unwrap();
}

#[test]
#[ignore = "Read-only probe; requires MTT_COPY_PROBE_ROOT pointing to a mounted test volume root"]
fn mounted_volume_root_can_be_resolved_without_dos_volume_registration() {
    let root =
        PathBuf::from(std::env::var_os("MTT_COPY_PROBE_ROOT").expect("Set MTT_COPY_PROBE_ROOT"));
    assert_eq!(root.parent(), None, "Probe accepts volume roots only");
    crate::plan::check_path(&root).unwrap();
    let original_error = std::fs::canonicalize(&root)
        .err()
        .and_then(|e| e.raw_os_error());
    let resolved = resolve(&root).unwrap();
    assert!(std::fs::metadata(&resolved.path).unwrap().is_dir());
    crate::plan::check_path(&resolved.path).unwrap();
    assert!(contains_directory(&resolved.path, &resolved.path).unwrap());
    println!(
        "canonicalize_error={original_error:?} compatibility_resolution={}",
        resolved.virtual_volume
    );
}
