use super::*;

fn append_entry(
    builder: &mut tar::Builder<Vec<u8>>,
    name: &str,
    entry_type: u8,
    data: &[u8],
) -> io::Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_path(name)?;
    header.set_entry_type(tar::EntryType::new(entry_type));
    header.set_mode(0o644);
    header.set_size(data.len() as u64);
    header.set_cksum();
    builder.append(&header, data)
}

fn archive_bytes(with_pax: bool) -> io::Result<Vec<u8>> {
    let mut builder = tar::Builder::new(Vec::new());
    if with_pax {
        // GHSA-3pv8-6f4r-ffg2: size belongs to the file, not the GNU longname header.
        append_entry(&mut builder, "pax", b'x', b"13 size=2048\n")?;
        append_entry(&mut builder, "longname", b'L', b"renamed.txt\0")?;

        // Only PAX supplies the actual size. Ignoring PAX must also fail this fixture.
        let mut header = tar::Header::new_gnu();
        header.set_path("first.txt")?;
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_size(0);
        header.set_cksum();
        builder.append(&header, &[b'A'; 2048][..])?;
    } else {
        append_entry(&mut builder, "first.txt", b'0', &[b'A'; 2048])?;
    }
    append_entry(&mut builder, "second.txt", b'0', b"second file")?;
    builder.into_inner()
}

fn encode_archive(extension: &str, bytes: &[u8]) -> io::Result<Vec<u8>> {
    match extension {
        "tar" => Ok(bytes.to_vec()),
        "tar.gz" => {
            let mut encoder = flate2::write::GzEncoder::new(Vec::new(), Default::default());
            encoder.write_all(bytes)?;
            encoder.finish()
        }
        "tar.bz2" => {
            let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), Default::default());
            encoder.write_all(bytes)?;
            encoder.finish()
        }
        "tar.xz" => {
            let mut encoder = xz2::write::XzEncoder::new(Vec::new(), 1);
            encoder.write_all(bytes)?;
            encoder.finish()
        }
        "tar.zst" => zstd::stream::encode_all(bytes, 1),
        _ => unreachable!(),
    }
}

fn assert_tar_extraction(with_pax: bool, internal_paths: &[&str]) -> io::Result<()> {
    let bytes = archive_bytes(with_pax)?;
    let first_name = if with_pax { "renamed.txt" } else { "first.txt" };
    let expected_first = internal_paths.contains(&first_name);
    for extension in ["tar", "tar.gz", "tar.bz2", "tar.xz", "tar.zst"] {
        let temp = tempfile::tempdir()?;
        let archive_path = temp.path().join(format!("archive.{extension}"));
        let destination = temp.path().join("output");
        fs::create_dir(&destination)?;
        fs::write(&archive_path, encode_archive(extension, &bytes)?)?;

        let mut extracted = 0;
        let mut extracted_bytes = 0;
        let count = extract_from_archive(
            &archive_path,
            internal_paths,
            &destination,
            &new_shared_progress(),
            &new_cancel_flag(),
            &mut extracted,
            &mut extracted_bytes,
        )
        .unwrap_or_else(|error| panic!("extraction failed for {extension}: {error}"));

        assert_eq!(count, internal_paths.len(), "{extension}");
        assert_eq!(extracted, internal_paths.len(), "{extension}");
        let expected_bytes = if expected_first { 2048 + 11 } else { 11 };
        assert_eq!(extracted_bytes, expected_bytes, "{extension}");
        if expected_first {
            assert_eq!(fs::read(destination.join(first_name))?, vec![b'A'; 2048]);
        } else {
            assert!(!destination.join(first_name).exists(), "{extension}");
        }
        assert_eq!(fs::read(destination.join("second.txt"))?, b"second file");
        assert_eq!(
            fs::read_dir(&destination)?.count(),
            internal_paths.len(),
            "{extension}"
        );
    }
    Ok(())
}

#[test]
fn tar_pax_size_does_not_desynchronize_extension_headers() -> io::Result<()> {
    assert_tar_extraction(true, &["renamed.txt", "second.txt"])
}

#[test]
fn tar_pax_unselected_entry_does_not_hide_selected_entry() -> io::Result<()> {
    assert_tar_extraction(true, &["second.txt"])
}

#[test]
fn tar_regular_entries_extract_without_regression() -> io::Result<()> {
    assert_tar_extraction(false, &["first.txt", "second.txt"])
}
