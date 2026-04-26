//! Stage 5: Media Foundation direct frame extraction (Nuclear Option)
//!
//! Bypasses the Windows thumbnail service entirely by directly reading
//! a video frame using IMFSourceReader. This works even when the thumbnail
//! cache is broken or returns 0x8004B205 (extraction pending) indefinitely.

use crate::infrastructure::windows::file_type::is_video_extension;
use crate::workers::thumbnail::processing::format_conversion::convert_nv12_to_rgba;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::{core::PCWSTR, Win32::Media::MediaFoundation::*};

/// Stage 5: Media Foundation direct frame extraction
///
/// This is the "nuclear option" - extracts a raw video frame when all else fails.
/// Only processes video files, returns None for non-video content.
pub fn extract(path: &Path) -> Option<(Vec<u8>, u32, u32)> {
    // Only for video files - use centralized extension check
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    if !is_video_extension(&ext) {
        return None;
    }

    let mf_start = std::time::Instant::now();
    log::trace!(
        "[Thumbnail] Stage 5 (Media Foundation) attempting: {:?}",
        path.file_name()
    );

    unsafe {
        // MFStartup/Shutdown - the thumbnail worker thread already has COM initialized
        // SAFETY: MF_VERSION = 0x00020070 (MF 2.0)
        // MFStartup is now called ONCE at thread start (see thumbnail_worker_loop)
        // so we don't need to call it here for every file.

        // Convert path to wide string
        let wide_path: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        // Create source reader
        let reader: IMFSourceReader =
            match MFCreateSourceReaderFromURL(PCWSTR(wide_path.as_ptr()), None) {
                Ok(r) => r,
                Err(e) => {
                    log::trace!(
                        "[Thumbnail] Stage 5: Failed to create source reader: {:?}",
                        e
                    );
                    return None;
                }
            };

        // Get the first video stream's native media type
        let media_type: IMFMediaType = match reader.GetNativeMediaType(
            0xFFFFFFFC, // MF_SOURCE_READER_FIRST_VIDEO_STREAM
            0,
        ) {
            Ok(mt) => mt,
            Err(e) => {
                log::trace!("[Thumbnail] Stage 5: No video stream found: {:?}", e);
                return None;
            }
        };

        // Get video dimensions
        let frame_size = media_type.GetUINT64(&MF_MT_FRAME_SIZE).ok()?;
        let width = (frame_size >> 32) as u32;
        let height = (frame_size & 0xFFFFFFFF) as u32;

        if width == 0 || height == 0 {
            log::trace!("[Thumbnail] Stage 5: Invalid dimensions");
            return None;
        }

        // Try RGB32 first, fallback to NV12 if not supported
        let output_type: IMFMediaType = match MFCreateMediaType() {
            Ok(mt) => mt,
            Err(_) => {
                return None;
            }
        };

        // MFMediaType_Video GUID
        let mf_video_guid = windows::core::GUID::from_u128(0x73646976_0000_0010_8000_00aa00389b71);
        // MFVideoFormat_RGB32 GUID
        let rgb32_guid = windows::core::GUID::from_u128(0x00000016_0000_0010_8000_00aa00389b71);
        // MFVideoFormat_NV12 GUID
        let nv12_guid = windows::core::GUID::from_u128(0x3231564e_0000_0010_8000_00aa00389b71);

        let _ = output_type.SetGUID(&MF_MT_MAJOR_TYPE, &mf_video_guid);
        let _ = output_type.SetGUID(&MF_MT_SUBTYPE, &rgb32_guid);

        // Try RGB32 first
        let use_nv12 = if reader
            .SetCurrentMediaType(0xFFFFFFFC, None, &output_type)
            .is_err()
        {
            log::trace!("[Thumbnail] Stage 5: RGB32 not supported, falling back to NV12");
            // Fallback to NV12 (universally supported by video decoders)
            let _ = output_type.SetGUID(&MF_MT_SUBTYPE, &nv12_guid);
            if reader
                .SetCurrentMediaType(0xFFFFFFFC, None, &output_type)
                .is_err()
            {
                log::trace!("[Thumbnail] Stage 5: Failed to set NV12 output");
                return None;
            }
            true
        } else {
            false
        };

        // Skip seeking for now - just read the first frame after position 0
        // This avoids complex PROPVARIANT handling

        // Read a video frame
        let mut stream_index: u32 = 0;
        let mut flags: u32 = 0;
        let mut timestamp: i64 = 0;
        let mut sample: Option<IMFSample> = None;

        let result = reader.ReadSample(
            0xFFFFFFFC, // MF_SOURCE_READER_FIRST_VIDEO_STREAM
            0,          // No control flags
            Some(&mut stream_index as *mut u32),
            Some(&mut flags as *mut u32),
            Some(&mut timestamp as *mut i64),
            Some(&mut sample as *mut Option<IMFSample>),
        );

        if result.is_err() {
            log::trace!("[Thumbnail] Stage 5: ReadSample failed: {:?}", result.err());
            return None;
        }

        let sample = match sample {
            Some(s) => s,
            None => {
                log::trace!("[Thumbnail] Stage 5: No sample returned");
                return None;
            }
        };

        // Convert sample to buffer
        let buffer = match sample.ConvertToContiguousBuffer() {
            Ok(b) => b,
            Err(e) => {
                log::trace!(
                    "[Thumbnail] Stage 5: ConvertToContiguousBuffer failed: {:?}",
                    e
                );
                return None;
            }
        };

        let mut data_ptr: *mut u8 = std::ptr::null_mut();
        let mut max_len: u32 = 0;
        let mut current_len: u32 = 0;

        if buffer
            .Lock(&mut data_ptr, Some(&mut max_len), Some(&mut current_len))
            .is_err()
        {
            log::trace!("[Thumbnail] Stage 5: Lock failed");
            return None;
        }

        // Convert to RGBA based on format
        let rgba_data = if use_nv12 {
            // NV12 format: Y plane (width*height bytes) + UV plane (width*height/2 bytes)
            let y_size = (width * height) as usize;
            let uv_size = y_size / 2;
            let expected_size = y_size + uv_size;

            if (current_len as usize) < expected_size {
                log::trace!(
                    "[Thumbnail] Stage 5: NV12 buffer size mismatch: {} vs expected {}",
                    current_len,
                    expected_size
                );
                let _ = buffer.Unlock();
                return None;
            }

            let nv12_slice = std::slice::from_raw_parts(data_ptr, expected_size);
            convert_nv12_to_rgba(nv12_slice, width, height)
        } else {
            // RGB32 format: straight BGRA copy and swap
            let expected_size = (width * height * 4) as usize;
            if (current_len as usize) < expected_size {
                log::trace!(
                    "[Thumbnail] Stage 5: RGB32 buffer size mismatch: {} vs expected {}",
                    current_len,
                    expected_size
                );
                let _ = buffer.Unlock();
                return None;
            }

            let mut rgba_data = vec![0u8; expected_size];
            std::ptr::copy_nonoverlapping(data_ptr, rgba_data.as_mut_ptr(), expected_size);
            rgba_data
        };

        let _ = buffer.Unlock();

        // Convert BGRA to RGBA if RGB32 was used (swap R and B channels)
        let mut rgba_data = rgba_data;
        if !use_nv12 {
            for pixel in rgba_data.chunks_exact_mut(4) {
                pixel.swap(0, 2); // Swap B and R
            }
        }

        let mf_elapsed = mf_start.elapsed();
        log::trace!(
            "[Thumbnail] Stage 5 SUCCESS: {:?} ({}x{}) in {:.2}s",
            path.file_name(),
            width,
            height,
            mf_elapsed.as_secs_f64()
        );
        Some((rgba_data, width, height))
    }
}
