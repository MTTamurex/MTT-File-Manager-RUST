use image::DynamicImage;
use image::ImageDecoder;
use image::imageops::FilterType;
use image::ImageReader;
use memmap2::Mmap;
use once_cell::sync::Lazy;
use std::fs::File;
use std::io;
use std::io::BufReader;
use std::io::BufWriter;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportImageFormat {
    Png,
    Jpeg,
    WebP,
    Bmp,
    Tiff,
}

impl ExportImageFormat {
    pub const ALL: [Self; 5] = [Self::Png, Self::Jpeg, Self::WebP, Self::Bmp, Self::Tiff];

    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::WebP => "webp",
            Self::Bmp => "bmp",
            Self::Tiff => "tiff",
        }
    }

    pub fn filter_label(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Jpeg => "JPEG",
            Self::WebP => "WebP",
            Self::Bmp => "BMP",
            Self::Tiff => "TIFF",
        }
    }

    fn image_format(self) -> image::ImageFormat {
        match self {
            Self::Png => image::ImageFormat::Png,
            Self::Jpeg => image::ImageFormat::Jpeg,
            Self::WebP => image::ImageFormat::WebP,
            Self::Bmp => image::ImageFormat::Bmp,
            Self::Tiff => image::ImageFormat::Tiff,
        }
    }
}

/// One frame of a multi-frame (animated) GIF.
#[derive(Clone, Debug)]
pub struct GifAnimationFrame {
    pub frame: DecodedFrame,
    /// How long this frame should be displayed, in milliseconds.
    pub delay_ms: u32,
}

/// Hard cap on total RGBA bytes for GIF frames to prevent OOM on pathological
/// files (e.g. 1 000-frame 4K GIF ≈ 16 GB without this limit).
const GIF_MAX_TOTAL_RGBA_BYTES: usize = 512 * 1024 * 1024; // 512 MB
/// Hard cap on the number of decoded frames to bound memory and CPU time.
const GIF_MAX_FRAMES: usize = 500;

/// Decodes all frames of an animated GIF. Returns an error if the file is not
/// a valid GIF or has no decodable frames. For single-frame / static GIFs the
/// returned `Vec` will contain exactly one element.
///
/// Applies safety caps: at most [`GIF_MAX_FRAMES`] frames and
/// [`GIF_MAX_TOTAL_RGBA_BYTES`] of combined pixel data.  Remaining frames are
/// silently discarded so the viewer stays responsive.
pub fn decode_gif_frames(path: &Path) -> io::Result<Vec<GifAnimationFrame>> {
    use image::AnimationDecoder;
    use image::codecs::gif::GifDecoder;

    let bytes = read_file_fast(path, DecodePriority::Interactive)?;
    let cursor = Cursor::new(bytes.as_slice());
    let decoder = GifDecoder::new(cursor)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

    let mut frames_out: Vec<GifAnimationFrame> = Vec::new();
    let mut total_rgba_bytes: usize = 0;
    for frame_result in decoder.into_frames() {
        if frames_out.len() >= GIF_MAX_FRAMES {
            log::warn!(
                "[IMAGE-VIEWER] GIF frame cap reached ({} frames), truncating",
                GIF_MAX_FRAMES
            );
            break;
        }

        match frame_result {
            Ok(f) => {
                let (numer, denom) = f.delay().numer_denom_ms();
                // GIF spec: 0 delay → display "as fast as possible". Use 10 ms minimum.
                let delay_ms = if denom == 0 { 100 } else { (numer / denom).max(10) };
                let rgba = f.into_buffer();
                let frame_bytes = rgba.as_raw().len();
                total_rgba_bytes = total_rgba_bytes.saturating_add(frame_bytes);
                if total_rgba_bytes > GIF_MAX_TOTAL_RGBA_BYTES {
                    log::warn!(
                        "[IMAGE-VIEWER] GIF memory cap reached ({} MB), truncating at {} frames",
                        total_rgba_bytes / (1024 * 1024),
                        frames_out.len()
                    );
                    break;
                }
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
    if is_svg_path(path) {
        return decode_svg_frame(path, None, priority);
    }

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
    if is_svg_path(path) {
        return decode_svg_frame(path, Some(max_side), priority);
    }

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

pub fn normalize_export_path(path: &Path, format: ExportImageFormat) -> PathBuf {
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case(format.extension()))
        .unwrap_or(false)
    {
        return path.to_path_buf();
    }

    let mut normalized = path.to_path_buf();
    normalized.set_extension(format.extension());
    normalized
}

pub fn encode_frame_to_path(
    frame: DecodedFrame,
    format: ExportImageFormat,
    output_path: &Path,
) -> io::Result<()> {
    let Some(buffer) = image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba)
    else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "decoded frame buffer has invalid dimensions",
        ));
    };

    let image = DynamicImage::ImageRgba8(buffer);
    let file = File::create(output_path)?;
    let mut writer = BufWriter::new(file);
    image
        .write_to(&mut writer, format.image_format())
        .map_err(|err| io::Error::other(err.to_string()))
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

fn is_svg_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("svg"))
        .unwrap_or(false)
}

fn decode_svg_frame(
    path: &Path,
    max_side: Option<u32>,
    priority: DecodePriority,
) -> io::Result<DecodedFrame> {
    let bytes = read_file_fast(path, priority)?;
    decode_svg_bytes(bytes.as_slice(), max_side)
}

/// Absolute upper bound for SVG rasterisation to prevent multi-gigabyte
/// allocations from pathological viewBox values (e.g. viewBox="0 0 65535 65535").
const SVG_MAX_RENDER_SIDE: u32 = 8192;

fn decode_svg_bytes(bytes: &[u8], max_side: Option<u32>) -> io::Result<DecodedFrame> {
    let options = usvg::Options::default();
    let tree = usvg::Tree::from_data(bytes, &options)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;

    let svg_size = tree.size();
    let base_width = svg_size.width().max(1.0);
    let base_height = svg_size.height().max(1.0);

    // Apply user-requested limit, then additionally clamp to SVG_MAX_RENDER_SIDE
    // to guard against SVGs with absurdly large intrinsic dimensions.
    let effective_limit = match max_side.filter(|limit| *limit > 0) {
        Some(limit) => limit.min(SVG_MAX_RENDER_SIDE),
        None => SVG_MAX_RENDER_SIDE,
    };
    let scale = (effective_limit as f32 / base_width.max(base_height)).min(1.0);

    let render_width = (base_width * scale).round().max(1.0) as u32;
    let render_height = (base_height * scale).round().max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(render_width, render_height)
        .ok_or_else(|| io::Error::other("failed to allocate SVG render surface"))?;

    pixmap.fill(tiny_skia::Color::TRANSPARENT);
    let transform = tiny_skia::Transform::from_scale(
        render_width as f32 / base_width,
        render_height as f32 / base_height,
    );

    resvg::render(&tree, transform, &mut pixmap.as_mut());

    Ok(DecodedFrame {
        width: render_width,
        height: render_height,
        rgba: unpremultiply_rgba(pixmap.data()),
    })
}

fn unpremultiply_rgba(pixels: &[u8]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(pixels.len());

    for chunk in pixels.chunks_exact(4) {
        let alpha = chunk[3];
        if alpha == 0 {
            rgba.extend_from_slice(&[0, 0, 0, 0]);
            continue;
        }

        let alpha_u32 = alpha as u32;
        rgba.push(((chunk[0] as u32 * 255) / alpha_u32).min(255) as u8);
        rgba.push(((chunk[1] as u32 * 255) / alpha_u32).min(255) as u8);
        rgba.push(((chunk[2] as u32 * 255) / alpha_u32).min(255) as u8);
        rgba.push(alpha);
    }

    rgba
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_export_path_replaces_wrong_extension() {
        let path = PathBuf::from("sample.png");
        let normalized = normalize_export_path(&path, ExportImageFormat::WebP);

        assert_eq!(normalized, PathBuf::from("sample.webp"));
    }

    #[test]
    fn normalize_export_path_keeps_matching_extension() {
        let path = PathBuf::from("sample.JPG");
        let normalized = normalize_export_path(&path, ExportImageFormat::Jpeg);

        assert_eq!(normalized, PathBuf::from("sample.JPG"));
    }

    #[test]
    fn decode_svg_preview_scales_with_aspect_ratio() {
        let svg = br#"<svg xmlns='http://www.w3.org/2000/svg' width='100' height='50' viewBox='0 0 100 50'><rect width='100' height='50' fill='#ff0000'/></svg>"#;

        let frame = decode_svg_bytes(svg, Some(32)).expect("svg should decode");

        assert_eq!(frame.width, 32);
        assert_eq!(frame.height, 16);
        assert_eq!(frame.rgba.len(), (frame.width * frame.height * 4) as usize);
    }
}

