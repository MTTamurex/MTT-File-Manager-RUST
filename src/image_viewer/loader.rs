use image::DynamicImage;
use image::ImageDecoder;
use image::imageops::FilterType;
use image::ImageReader;
use memmap2::Mmap;
use once_cell::sync::Lazy;
use std::fs::File;
use std::io;
use std::io::BufReader;
use std::io::Cursor;
use std::path::Path;
use std::path::PathBuf;

const MMAP_THRESHOLD_BYTES: u64 = 1_048_576;

static VIEWER_THUMBNAIL_CACHE: Lazy<Option<crate::infrastructure::disk_cache::ThumbnailDiskCache>> =
    Lazy::new(|| {
        let cache_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("MTT-File-Manager")
            .join("thumbnails");

        match crate::infrastructure::disk_cache::ThumbnailDiskCache::new(cache_dir) {
            Ok(cache) => Some(cache),
            Err(err) => {
                log::warn!(
                    "[IMAGE-VIEWER] failed to open thumbnail cache for fast preview path: {}",
                    err
                );
                None
            }
        }
    });

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodePriority {
    Interactive,
    Background,
}

#[derive(Clone, Debug)]
pub struct DecodedFrame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// One frame of a multi-frame (animated) GIF.
#[derive(Clone, Debug)]
pub struct GifAnimationFrame {
    pub frame: DecodedFrame,
    /// How long this frame should be displayed, in milliseconds.
    pub delay_ms: u32,
}

/// Decodes all frames of an animated GIF. Returns an error if the file is not
/// a valid GIF or has no decodable frames. For single-frame / static GIFs the
/// returned `Vec` will contain exactly one element.
pub fn decode_gif_frames(path: &Path) -> io::Result<Vec<GifAnimationFrame>> {
    use image::AnimationDecoder;
    use image::codecs::gif::GifDecoder;

    let bytes = read_file_fast(path, DecodePriority::Interactive)?;
    let cursor = Cursor::new(bytes.as_slice());
    let decoder = GifDecoder::new(cursor)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

    let mut frames_out: Vec<GifAnimationFrame> = Vec::new();
    for frame_result in decoder.into_frames() {
        match frame_result {
            Ok(f) => {
                let (numer, denom) = f.delay().numer_denom_ms();
                // GIF spec: 0 delay → display "as fast as possible". Use 10 ms minimum.
                let delay_ms = if denom == 0 { 100 } else { (numer / denom).max(10) };
                let rgba = f.into_buffer();
                frames_out.push(GifAnimationFrame {
                    frame: DecodedFrame {
                        width: rgba.width(),
                        height: rgba.height(),
                        rgba: rgba.into_raw(),
                    },
                    delay_ms,
                });
            }
            Err(e) => {
                log::warn!("[IMAGE-VIEWER] GIF frame decode error: {}", e);
                break;
            }
        }
    }

    if frames_out.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "no decodable GIF frames",
        ));
    }

    Ok(frames_out)
}

pub fn decode_full_frame(path: &Path) -> io::Result<DecodedFrame> {
    decode_full_frame_with_priority(path, DecodePriority::Interactive)
}

pub fn decode_full_frame_with_priority(
    path: &Path,
    priority: DecodePriority,
) -> io::Result<DecodedFrame> {
    let image = decode_dynamic(path, priority)?;
    Ok(frame_from_dynamic(image))
}

pub fn decode_preview_frame(path: &Path, max_side: u32) -> io::Result<DecodedFrame> {
    decode_preview_frame_with_priority(path, max_side, DecodePriority::Interactive)
}

pub fn decode_preview_frame_with_priority(
    path: &Path,
    max_side: u32,
    priority: DecodePriority,
) -> io::Result<DecodedFrame> {
    if let Some(frame) = decode_preview_from_thumbnail_cache(path, max_side) {
        return Ok(frame);
    }

    let image = decode_dynamic(path, priority)?;
    if image.width() <= max_side && image.height() <= max_side {
        return Ok(frame_from_dynamic(image));
    }

    let use_nearest = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|ext| {
            ext.eq_ignore_ascii_case("gif")
                || ext.eq_ignore_ascii_case("ico")
                || ext.eq_ignore_ascii_case("bmp")
        })
        .unwrap_or(false);
    let filter = if use_nearest {
        FilterType::Nearest
    } else {
        FilterType::Triangle
    };
    let resized = image.resize(max_side, max_side, filter);
    Ok(frame_from_dynamic(resized))
}

fn decode_preview_from_thumbnail_cache(path: &Path, max_side: u32) -> Option<DecodedFrame> {
    let cache = VIEWER_THUMBNAIL_CACHE.as_ref()?;
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;

    let entry = cache
        .get(path, modified)
        .or_else(|| cache.get_latest(path))?;

    let image = image::load_from_memory_with_format(&entry.data, image::ImageFormat::WebP).ok()?;
    let image = if image.width() > max_side || image.height() > max_side {
        image.resize(max_side, max_side, FilterType::Triangle)
    } else {
        image
    };

    Some(frame_from_dynamic(image))
}

fn decode_dynamic(path: &Path, priority: DecodePriority) -> io::Result<DynamicImage> {
    let bytes = read_file_fast(path, priority)?;

    // Fast path: decode with EXIF orientation using image crate.
    match decode_with_exif_orientation(bytes.as_slice()) {
        Ok(img) => Ok(img),
        Err(_) => {
            // Robust fallback on Windows: WIC path for problematic inputs.
            #[cfg(target_os = "windows")]
            {
                if let Some((rgba, w, h)) =
                    crate::workers::thumbnail::extraction::stage2_wic::extract(path)
                {
                    if let Some(buffer) = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(
                        w, h, rgba,
                    ) {
                        return Ok(DynamicImage::ImageRgba8(buffer));
                    }
                }
            }

            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "failed to decode image using primary and fallback pipelines",
            ))
        }
    }
}

fn frame_from_dynamic(image: DynamicImage) -> DecodedFrame {
    let rgba = image.to_rgba8();
    DecodedFrame {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    }
}

enum FileBytes {
    Owned(Vec<u8>),
    Mapped(Mmap),
}

impl FileBytes {
    fn as_slice(&self) -> &[u8] {
        match self {
            Self::Owned(v) => v.as_slice(),
            Self::Mapped(m) => m.as_ref(),
        }
    }
}

fn read_file_fast(path: &Path, priority: DecodePriority) -> io::Result<FileBytes> {
    #[cfg(target_os = "windows")]
    {
        let open_result = match priority {
            DecodePriority::Interactive => {
                crate::infrastructure::windows::file_flags::open_sequential(path)
            }
            DecodePriority::Background => {
                crate::infrastructure::windows::file_flags::open_sequential_background(path)
            }
        };

        if let Ok(file) = open_result {
            let meta = file.metadata()?;
            if meta.len() > MMAP_THRESHOLD_BYTES {
                // SAFETY: mapping read-only file descriptor for immutable read.
                let mmap = unsafe { Mmap::map(&file)? };
                return Ok(FileBytes::Mapped(mmap));
            }

            let mut reader = BufReader::with_capacity(64 * 1024, file);
            let mut out = Vec::with_capacity(meta.len() as usize);
            std::io::Read::read_to_end(&mut reader, &mut out)?;
            return Ok(FileBytes::Owned(out));
        }
    }

    let file = File::open(path)?;
    let meta = file.metadata()?;

    if meta.len() > MMAP_THRESHOLD_BYTES {
        // SAFETY: mapping read-only file descriptor for immutable read.
        let mmap = unsafe { Mmap::map(&file)? };
        return Ok(FileBytes::Mapped(mmap));
    }

    Ok(FileBytes::Owned(std::fs::read(path)?))
}

fn decode_with_exif_orientation(bytes: &[u8]) -> io::Result<DynamicImage> {
    let cursor = Cursor::new(bytes);
    let reader = ImageReader::new(cursor)
        .with_guessed_format()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

    match reader.into_decoder() {
        Ok(mut decoder) => {
            let orientation = decoder
                .orientation()
                .unwrap_or(image::metadata::Orientation::NoTransforms);
            let image = DynamicImage::from_decoder(decoder)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
            Ok(apply_orientation(image, orientation))
        }
        Err(_) => {
            let fallback = ImageReader::new(BufReader::new(Cursor::new(bytes)))
                .with_guessed_format()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?
                .decode()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
            Ok(fallback)
        }
    }
}

fn apply_orientation(img: DynamicImage, orientation: image::metadata::Orientation) -> DynamicImage {
    use image::metadata::Orientation::*;

    match orientation {
        NoTransforms => img,
        FlipHorizontal => img.fliph(),
        Rotate180 => img.rotate180(),
        FlipVertical => img.flipv(),
        Rotate90 => img.rotate90(),
        Rotate90FlipH => img.rotate90().fliph(),
        Rotate270 => img.rotate270(),
        Rotate270FlipH => img.rotate270().fliph(),
    }
}

