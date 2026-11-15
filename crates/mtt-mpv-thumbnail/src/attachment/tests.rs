use super::*;
use std::{io::Write, time::Duration};

fn header(id: u32, size: u64) -> Vec<u8> {
    let id = id.to_be_bytes();
    let mut bytes = id
        .iter()
        .skip_while(|b| **b == 0)
        .copied()
        .collect::<Vec<_>>();
    let mut size = size.to_be_bytes();
    size[0] |= 1; // Eight-byte size, exercising the full EBML VINT width.
    bytes.extend_from_slice(&size);
    bytes
}

fn element(id: u32, data: &[u8]) -> Vec<u8> {
    let mut bytes = header(id, data.len() as u64);
    bytes.extend_from_slice(data);
    bytes
}

fn file(name: &[u8], data: &[u8], payload_first: bool) -> Vec<u8> {
    let name = element(FILE_NAME, name);
    let data = element(FILE_DATA, data);
    element(
        ATTACHED_FILE,
        &if payload_first {
            [data, name].concat()
        } else {
            [name, data].concat()
        },
    )
}

fn source(segment: &[u8]) -> Vec<u8> {
    [element(EBML, &[]), element(SEGMENT, segment)].concat()
}

fn read(data: &[u8]) -> Result<Option<Vec<u8>>> {
    let mut path = tempfile::NamedTempFile::new().unwrap();
    path.write_all(data).unwrap();
    read_cover(path.path(), Instant::now() + Duration::from_secs(1))
}

fn seek_head(id: u32, offset: u64) -> Vec<u8> {
    let seek = [
        element(0x53ab, &id.to_be_bytes()),
        element(0x53ac, &offset.to_be_bytes()),
    ]
    .concat();
    element(SEEK_HEAD, &element(0x4dbb, &seek))
}

#[test]
fn accepts_only_exact_cover_names_and_either_field_order() {
    for name in [
        b"cover.jpg".as_slice(),
        b"cover.jpeg",
        b"Cover.jpg",
        b"other.jpg",
        b"folder/cover.jpg",
        b"cover.png",
    ] {
        for payload_first in [false, true] {
            let attachment = element(ATTACHMENTS, &file(name, b"payload", payload_first));
            let value = read(&source(&attachment)).unwrap();
            assert_eq!(
                value,
                matches!(name, b"cover.jpg" | b"cover.jpeg").then(|| b"payload".to_vec())
            );
        }
    }
}

#[test]
fn skips_unrelated_attachments_and_supports_unknown_segment_size() {
    let files = [
        file(b"font.ttf", b"font", false),
        file(b"cover.jpg", b"image", false),
    ]
    .concat();
    let mut data = element(EBML, &[]);
    data.extend_from_slice(&SEGMENT.to_be_bytes());
    data.push(0xff);
    data.extend_from_slice(&element(ATTACHMENTS, &files));
    assert_eq!(read(&data).unwrap().unwrap(), b"image");
}

#[test]
fn follows_attachment_index_after_cluster_without_scanning_video() {
    let attachment = element(ATTACHMENTS, &file(b"cover.jpeg", b"image", false));
    let cluster = element(CLUSTER, &attachment); // A decoy inside video data.
    assert!(read(&source(&cluster)).unwrap().is_none());
    let placeholder = seek_head(ATTACHMENTS, 0);
    let index = seek_head(ATTACHMENTS, (placeholder.len() + cluster.len()) as u64);
    let data = source(&[index, cluster, attachment].concat());
    assert_eq!(read(&data).unwrap().unwrap(), b"image");
}

#[test]
fn follows_secondary_indexes_and_bounds_cycles_and_link_counts() {
    let placeholder = seek_head(SEEK_HEAD, 0);
    let cluster = element(CLUSTER, b"not metadata");
    let second_offset = (placeholder.len() + cluster.len()) as u64;
    let secondary = seek_head(ATTACHMENTS, second_offset + placeholder.len() as u64);
    let attachment = element(ATTACHMENTS, &file(b"cover.jpg", b"image", false));
    assert_eq!(
        read(&source(
            &[
                seek_head(SEEK_HEAD, second_offset),
                cluster.clone(),
                secondary,
                attachment
            ]
            .concat()
        ))
        .unwrap()
        .unwrap(),
        b"image"
    );
    assert!(read(&source(&[seek_head(SEEK_HEAD, 0), cluster].concat()))
        .unwrap()
        .is_none());
    let links = (0..65)
        .flat_map(|_| seek_head(SEEK_HEAD, 0))
        .collect::<Vec<_>>();
    assert!(read(&source(&links)).is_err());
}

#[test]
fn rejects_truncation_overflows_wrong_index_targets_and_duplicate_fields() {
    assert!(read(&[0]).is_err());
    let mut truncated = source(&element(ATTACHMENTS, &file(b"cover.jpg", b"image", false)));
    truncated.pop();
    assert!(read(&truncated).is_err());
    assert!(read(&source(&seek_head(ATTACHMENTS, u64::MAX))).is_err());
    assert!(read(&source(
        &[seek_head(ATTACHMENTS, 0), element(CLUSTER, &[])].concat()
    ))
    .is_err());
    let fields = [
        element(FILE_NAME, b"cover.jpg"),
        element(FILE_NAME, b"cover.jpeg"),
        element(FILE_DATA, b"image"),
    ]
    .concat();
    assert!(read(&source(&element(
        ATTACHMENTS,
        &element(ATTACHED_FILE, &fields)
    )))
    .is_err());
}

#[test]
fn rejects_oversized_payload_without_allocating_or_reading_it() {
    let payload_size = MAX_COVER_BYTES + 1;
    let filename = element(FILE_NAME, b"cover.jpg");
    let file_size =
        filename.len() as u64 + header(FILE_DATA, payload_size).len() as u64 + payload_size;
    let attachments_size = header(ATTACHED_FILE, file_size).len() as u64 + file_size;
    let segment_size = header(ATTACHMENTS, attachments_size).len() as u64 + attachments_size;
    let mut data = element(EBML, &[]);
    data.extend(header(SEGMENT, segment_size));
    data.extend(header(ATTACHMENTS, attachments_size));
    data.extend(header(ATTACHED_FILE, file_size));
    data.extend(filename);
    data.extend(header(FILE_DATA, payload_size));
    let mut path = tempfile::NamedTempFile::new().unwrap();
    path.write_all(&data).unwrap();
    path.as_file()
        .set_len(data.len() as u64 + payload_size)
        .unwrap();
    assert!(read_cover(path.path(), Instant::now() + Duration::from_secs(1)).is_err());
}

#[test]
fn expired_deadline_stops_before_parsing() {
    let mut path = tempfile::NamedTempFile::new().unwrap();
    path.write_all(&source(&[])).unwrap();
    assert!(read_cover(path.path(), Instant::now()).is_err());
}

#[test]
fn metadata_element_count_is_bounded() {
    let data = element(0xec, &[]).repeat(4097); // Empty Void elements, not video data.
    assert!(read(&source(&data)).is_err());
}
