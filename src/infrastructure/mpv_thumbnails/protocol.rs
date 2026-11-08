//! Versioned, bounded binary frames over inherited private stdin/stdout pipes.
use std::io::{self, Read, Write};
use std::path::Path;

use crate::domain::thumbnail::{is_valid_rgba_buffer, MAX_THUMBNAIL_SIDE};

const MAGIC: &[u8; 8] = b"MTTMPV02";
const MAX_PATH_BYTES: usize = 128 * 1024;

#[derive(Debug)]
pub struct Request {
    pub id: u64,
    pub max_side: u32,
    pub budget_ms: u32,
    pub path: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Failure {
    Decode = 1,
    Unavailable = 2,
    IncompleteDownload = 3,
    WriteLocked = 4,
    RecentlyChanging = 5,
    Timeout = 6,
}

#[derive(Debug)]
pub struct Frame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub type Response = Result<Frame, Failure>;

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Invalid thumbnail IPC frame")
}

impl Request {
    fn validate(&self) -> io::Result<()> {
        if self.id == 0
            || !(1..=MAX_THUMBNAIL_SIDE).contains(&self.max_side)
            || !(1..=15_000).contains(&self.budget_ms)
            || self.path.is_empty()
            || self.path.len() > MAX_PATH_BYTES
            || self.path.contains('\0')
            || !Path::new(&self.path).is_absolute()
        {
            return Err(invalid());
        }
        Ok(())
    }
}

pub fn write_request(writer: &mut impl Write, request: &Request) -> io::Result<()> {
    request.validate()?;
    writer.write_all(MAGIC)?;
    writer.write_all(&request.id.to_le_bytes())?;
    writer.write_all(&request.max_side.to_le_bytes())?;
    writer.write_all(&request.budget_ms.to_le_bytes())?;
    writer.write_all(&(request.path.len() as u32).to_le_bytes())?;
    writer.write_all(request.path.as_bytes())?;
    writer.flush()
}

pub fn read_request(reader: &mut impl Read) -> io::Result<Option<Request>> {
    let mut header = [0; 28];
    // Clean EOF is only accepted at the boundary between requests.
    if reader.read(&mut header[..1])? == 0 {
        return Ok(None);
    }
    reader.read_exact(&mut header[1..])?;
    if &header[..8] != MAGIC {
        return Err(invalid());
    }
    let len = u32::from_le_bytes(header[24..28].try_into().unwrap()) as usize;
    if len == 0 || len > MAX_PATH_BYTES {
        return Err(invalid());
    }
    let mut bytes = vec![0; len];
    reader.read_exact(&mut bytes)?;
    let request = Request {
        id: u64::from_le_bytes(header[8..16].try_into().unwrap()),
        max_side: u32::from_le_bytes(header[16..20].try_into().unwrap()),
        budget_ms: u32::from_le_bytes(header[20..24].try_into().unwrap()),
        path: String::from_utf8(bytes).map_err(|_| invalid())?,
    };
    request.validate()?;
    Ok(Some(request))
}

pub fn write_response(writer: &mut impl Write, id: u64, response: &Response) -> io::Result<()> {
    let (status, width, height, data) = match response {
        Ok(frame) => {
            if !is_valid_rgba_buffer(
                frame.width,
                frame.height,
                MAX_THUMBNAIL_SIDE,
                frame.rgba.len(),
            ) {
                return Err(invalid());
            }
            (0, frame.width, frame.height, frame.rgba.as_slice())
        }
        Err(failure) => (*failure as u32, 0, 0, &[][..]),
    };
    writer.write_all(MAGIC)?;
    writer.write_all(&id.to_le_bytes())?;
    for word in [status, width, height, data.len() as u32] {
        writer.write_all(&word.to_le_bytes())?;
    }
    writer.write_all(data)?;
    writer.flush()
}

pub fn read_response(reader: &mut impl Read, id: u64, max_side: u32) -> io::Result<Response> {
    let mut header = [0; 32];
    reader.read_exact(&mut header)?;
    if &header[..8] != MAGIC || u64::from_le_bytes(header[8..16].try_into().unwrap()) != id {
        return Err(invalid());
    }
    let word = |offset| u32::from_le_bytes(header[offset..offset + 4].try_into().unwrap());
    let (status, width, height, len) = (word(16), word(20), word(24), word(28) as usize);
    if status != 0 {
        if width != 0 || height != 0 || len != 0 {
            return Err(invalid());
        }
        return Ok(Err(match status {
            1 => Failure::Decode,
            2 => Failure::Unavailable,
            3 => Failure::IncompleteDownload,
            4 => Failure::WriteLocked,
            5 => Failure::RecentlyChanging,
            6 => Failure::Timeout,
            _ => return Err(invalid()),
        }));
    }
    // Validate before allocating: a peer cannot request an unbounded pixel buffer.
    if max_side == 0
        || max_side > MAX_THUMBNAIL_SIDE
        || !is_valid_rgba_buffer(width, height, max_side, len)
    {
        return Err(invalid());
    }
    let mut rgba = vec![0; len];
    reader.read_exact(&mut rgba)?;
    Ok(Ok(Frame {
        rgba,
        width,
        height,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trip_and_clean_eof() {
        let request = Request {
            id: 7,
            max_side: 256,
            budget_ms: 15_000,
            path: r"C:\Videos\clip.mp4".into(),
        };
        let mut bytes = Vec::new();
        write_request(&mut bytes, &request).unwrap();
        let mut reader = bytes.as_slice();
        let decoded = read_request(&mut reader).unwrap().unwrap();
        assert_eq!(decoded.id, 7);
        assert_eq!(decoded.path, request.path);
        assert_eq!(decoded.budget_ms, 15_000);
        assert!(read_request(&mut reader).unwrap().is_none());
        assert!(read_request(&mut &bytes[..10]).is_err());
    }

    #[test]
    fn rejects_bad_request_lengths_versions_paths_and_sizes() {
        for path in [
            "relative.mp4",
            "https://example.invalid/clip.mp4",
            "C:\\clip\0.mp4",
        ] {
            assert!(write_request(
                &mut Vec::new(),
                &Request {
                    id: 1,
                    max_side: 256,
                    budget_ms: 15_000,
                    path: path.into()
                }
            )
            .is_err());
        }
        let mut bytes = Vec::new();
        write_request(
            &mut bytes,
            &Request {
                id: 1,
                max_side: 256,
                budget_ms: 15_000,
                path: r"C:\Videos\clip.mp4".into(),
            },
        )
        .unwrap();
        let mut oversized = bytes.clone();
        oversized[24..28].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(read_request(&mut oversized.as_slice()).is_err());
        for size in [0, MAX_THUMBNAIL_SIDE + 1] {
            let mut invalid_size = bytes.clone();
            invalid_size[16..20].copy_from_slice(&size.to_le_bytes());
            assert!(read_request(&mut invalid_size.as_slice()).is_err());
        }
        for budget in [0u32, 15_001] {
            let mut invalid_budget = bytes.clone();
            invalid_budget[20..24].copy_from_slice(&budget.to_le_bytes());
            assert!(read_request(&mut invalid_budget.as_slice()).is_err());
        }
        let mut zero_id = bytes.clone();
        zero_id[8..16].copy_from_slice(&0u64.to_le_bytes());
        assert!(read_request(&mut zero_id.as_slice()).is_err());
        bytes[0] = 0;
        assert!(read_request(&mut bytes.as_slice()).is_err());
    }

    #[test]
    fn validates_response_id_bounds_and_payload_before_allocation() {
        let mut bytes = Vec::new();
        write_response(
            &mut bytes,
            7,
            &Ok(Frame {
                rgba: vec![1, 2, 3, 255],
                width: 1,
                height: 1,
            }),
        )
        .unwrap();
        assert_eq!(
            read_response(&mut bytes.as_slice(), 7, 256)
                .unwrap()
                .unwrap()
                .rgba,
            [1, 2, 3, 255]
        );
        assert!(read_response(&mut bytes.as_slice(), 8, 256).is_err());
        assert!(read_response(&mut &bytes[..34], 7, 256).is_err());
        bytes[28..32].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(read_response(&mut bytes.as_slice(), 7, 256).is_err());
    }

    #[test]
    fn failures_have_no_pixel_payload() {
        let mut bytes = Vec::new();
        write_response(&mut bytes, 1, &Err(Failure::WriteLocked)).unwrap();
        assert_eq!(
            read_response(&mut bytes.as_slice(), 1, 256)
                .unwrap()
                .unwrap_err(),
            Failure::WriteLocked
        );
        bytes[28..32].copy_from_slice(&4u32.to_le_bytes());
        assert!(read_response(&mut bytes.as_slice(), 1, 256).is_err());
    }
}
