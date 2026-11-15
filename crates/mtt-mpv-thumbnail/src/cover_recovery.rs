//! Recover an accepted cover whose declared MIME/codec does not match its bytes.
use crate::{attachment, pixels::Screenshot};
use anyhow::{ensure, Result};
use image::{imageops::FilterType, ImageFormat, ImageReader, Limits};
use std::{io::Cursor, path::Path, time::Instant};

pub(super) fn capture(path: &Path, max_side: u32, deadline: Instant) -> Result<Option<Screenshot>> {
    let Some(bytes) = attachment::read_cover(path, deadline)? else {
        return Ok(None);
    };
    decode(bytes, max_side, deadline).map(Some)
}

fn decode(bytes: Vec<u8>, max_side: u32, deadline: Instant) -> Result<Screenshot> {
    ensure!(
        Instant::now() < deadline,
        "Cover recovery deadline exceeded"
    );
    let format = image::guess_format(&bytes)?;
    ensure!(
        matches!(
            format,
            ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP
        ),
        "Unsupported recovered cover format"
    );
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode()?;
    ensure!(
        Instant::now() < deadline,
        "Cover decoding deadline exceeded"
    );
    let (width, height) = (image.width(), image.height());
    let image = image
        .resize(max_side, max_side, FilterType::Triangle)
        .into_rgba8();
    ensure!(Instant::now() < deadline, "Cover resize deadline exceeded");
    Ok(Screenshot {
        image,
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn malformed_or_unsupported_bytes_are_not_accepted_as_a_cover() {
        for bytes in [
            b"not an image".as_slice(),
            b"RIFF\x08\x00\x00\x00WEBPVP8 ",
            b"\x89PNG\r\n\x1a\n",
            b"GIF89a",
        ] {
            assert!(decode(bytes.to_vec(), 256, Instant::now() + Duration::from_secs(1)).is_err());
        }
    }

    #[test]
    fn expired_deadline_rejects_recovery() {
        assert!(decode(Vec::new(), 256, Instant::now()).is_err());
    }
}
