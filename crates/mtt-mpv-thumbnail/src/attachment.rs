//! Bounded Matroska attachment access for recovery only; not a video demuxer.
//! Read metadata before the first Cluster, then follow SeekHead references.
use anyhow::{ensure, Context, Result};
use std::{
    collections::{HashSet, VecDeque},
    fs::File,
    io::{BufReader, Read},
    path::Path,
    time::Instant,
};

const EBML: u32 = 0x1a45dfa3;
const SEGMENT: u32 = 0x18538067;
const SEEK_HEAD: u32 = 0x114d9b74;
const ATTACHMENTS: u32 = 0x1941a469;
const CLUSTER: u32 = 0x1f43b675;
const ATTACHED_FILE: u32 = 0x61a7;
const FILE_NAME: u32 = 0x466e;
const FILE_DATA: u32 = 0x465c;
const MAX_COVER_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy)]
struct Element {
    id: u32,
    data: u64,
    end: u64,
}

struct Reader {
    input: BufReader<File>,
    position: u64,
    deadline: Instant,
    elements: u32,
}

impl Reader {
    fn seek(&mut self, position: u64) -> Result<()> {
        self.check()?;
        self.input
            .seek_relative(i64::try_from(position)? - i64::try_from(self.position)?)?;
        self.position = position;
        Ok(())
    }

    fn check(&self) -> Result<()> {
        ensure!(
            Instant::now() < self.deadline,
            "Cover attachment deadline exceeded"
        );
        Ok(())
    }

    fn byte(&mut self, end: u64) -> Result<u8> {
        ensure!(self.position < end, "Truncated EBML header");
        let mut byte = [0];
        self.input.read_exact(&mut byte)?;
        self.position += 1;
        Ok(byte[0])
    }

    fn vint(&mut self, end: u64, id: bool) -> Result<(u64, bool)> {
        let first = self.byte(end)?;
        let length = first.leading_zeros() + 1;
        ensure!(length <= if id { 4 } else { 8 }, "Invalid EBML integer");
        let mut value = if id {
            u64::from(first)
        } else {
            u64::from(first & (0xffu16 >> length) as u8)
        };
        for _ in 1..length {
            value = (value << 8) | u64::from(self.byte(end)?);
        }
        Ok((value, !id && value == (1u64 << (7 * length)) - 1))
    }

    fn next(&mut self, end: u64) -> Result<Option<Element>> {
        self.check()?;
        if self.position == end {
            return Ok(None);
        }
        ensure!(self.position < end, "Invalid EBML position");
        self.elements += 1;
        ensure!(
            self.elements <= 4096,
            "Cover metadata element limit exceeded"
        );
        let id = u32::try_from(self.vint(end, true)?.0)?;
        let (size, unknown) = self.vint(end, false)?;
        ensure!(
            !unknown || matches!(id, SEGMENT | CLUSTER),
            "Unknown-length cover metadata"
        );
        let data = self.position;
        let element_end = if unknown {
            end
        } else {
            data.checked_add(size).context("EBML size overflow")?
        };
        ensure!(element_end <= end, "EBML element exceeds parent");
        Ok(Some(Element {
            id,
            data,
            end: element_end,
        }))
    }

    fn bytes(&mut self, element: Element, limit: u64) -> Result<Vec<u8>> {
        let size = element.end - element.data;
        ensure!(size <= limit, "Cover attachment exceeds byte limit");
        self.seek(element.data)?;
        let mut data = vec![0; usize::try_from(size)?];
        for chunk in data.chunks_mut(64 * 1024) {
            self.check()?;
            self.input.read_exact(chunk)?;
            self.position += chunk.len() as u64;
        }
        self.check()?;
        Ok(data)
    }

    fn attached_file(&mut self, element: Element) -> Result<Option<Vec<u8>>> {
        let mut name_matches = false;
        let mut data = None;
        let mut saw_name = false;
        while let Some(child) = self.next(element.end)? {
            match child.id {
                FILE_NAME => {
                    ensure!(!saw_name, "Duplicate attachment filename");
                    saw_name = true;
                    if matches!(child.end - child.data, 9 | 10) {
                        let name = self.bytes(child, 10)?;
                        name_matches = matches!(name.as_slice(), b"cover.jpg" | b"cover.jpeg");
                    }
                }
                FILE_DATA => {
                    ensure!(data.is_none(), "Duplicate attachment payload");
                    data = Some(child);
                }
                _ => {}
            }
            self.seek(child.end)?;
        }
        if !name_matches {
            return Ok(None);
        }
        let data = data.context("Missing cover attachment payload")?;
        ensure!(data.end > data.data, "Empty cover attachment");
        self.bytes(data, MAX_COVER_BYTES).map(Some)
    }

    fn attachments(&mut self, element: Element) -> Result<Option<Vec<u8>>> {
        while let Some(child) = self.next(element.end)? {
            if child.id == ATTACHED_FILE {
                if let Some(bytes) = self.attached_file(child)? {
                    return Ok(Some(bytes));
                }
            }
            self.seek(child.end)?;
        }
        Ok(None)
    }

    fn index(
        &mut self,
        element: Element,
        segment: Element,
        pending: &mut VecDeque<(u32, u64)>,
    ) -> Result<()> {
        while let Some(entry) = self.next(element.end)? {
            if entry.id == 0x4dbb {
                // Seek
                let mut id = None;
                let mut offset = None;
                while let Some(field) = self.next(entry.end)? {
                    if matches!(field.id, 0x53ab | 0x53ac) {
                        // SeekID / SeekPosition
                        let bytes = self.bytes(field, if field.id == 0x53ab { 4 } else { 8 })?;
                        ensure!(!bytes.is_empty(), "Empty cover index value");
                        let value = bytes.iter().fold(0u64, |n, b| (n << 8) | u64::from(*b));
                        if field.id == 0x53ab {
                            ensure!(id.is_none(), "Duplicate cover index ID");
                            id = Some(value);
                        } else {
                            ensure!(offset.is_none(), "Duplicate cover index position");
                            offset = Some(value);
                        }
                    }
                    self.seek(field.end)?;
                }
                if let (Some(id), Some(offset)) = (id, offset) {
                    if id == u64::from(ATTACHMENTS) || id == u64::from(SEEK_HEAD) {
                        let position = segment
                            .data
                            .checked_add(offset)
                            .context("Cover index overflow")?;
                        ensure!(position < segment.end, "Cover index outside segment");
                        ensure!(pending.len() < 64, "Cover index link limit exceeded");
                        pending.push_back((id as u32, position));
                    }
                }
            }
            self.seek(entry.end)?;
        }
        Ok(())
    }
}

pub(super) fn read_cover(path: &Path, deadline: Instant) -> Result<Option<Vec<u8>>> {
    ensure!(
        Instant::now() < deadline,
        "Cover attachment deadline exceeded"
    );
    let file = File::open(path).context("Cannot open cover attachment source")?;
    let length = file.metadata()?.len();
    ensure!(length <= i64::MAX as u64, "Cover source is too large");
    let mut reader = Reader {
        input: BufReader::new(file),
        position: 0,
        deadline,
        elements: 0,
    };
    let header = reader.next(length)?.context("Missing EBML header")?;
    ensure!(header.id == EBML, "Not an EBML source");
    reader.seek(header.end)?;
    let segment = loop {
        let element = reader.next(length)?.context("Missing Matroska segment")?;
        if element.id == SEGMENT {
            break element;
        }
        reader.seek(element.end)?;
    };
    let mut pending = VecDeque::new();
    while let Some(element) = reader.next(segment.end)? {
        match element.id {
            ATTACHMENTS => {
                if let Some(bytes) = reader.attachments(element)? {
                    return Ok(Some(bytes));
                }
            }
            SEEK_HEAD => reader.index(element, segment, &mut pending)?,
            CLUSTER => break, // Never scan video clusters looking for a cover.
            _ => {}
        }
        reader.seek(element.end)?;
    }
    let mut visited = HashSet::new();
    while let Some((id, position)) = pending.pop_front() {
        if !visited.insert(position) {
            continue;
        }
        ensure!(visited.len() <= 64, "Cover index traversal limit exceeded");
        reader.seek(position)?;
        let element = reader
            .next(segment.end)?
            .context("Missing indexed cover metadata")?;
        ensure!(element.id == id, "Mismatched cover index target");
        if id == ATTACHMENTS {
            if let Some(bytes) = reader.attachments(element)? {
                return Ok(Some(bytes));
            }
        } else {
            reader.index(element, segment, &mut pending)?;
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests;
