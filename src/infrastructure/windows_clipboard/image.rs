use super::{set_preferred_drop_effect, DROPEFFECT_COPY};
use clipboard_win::{formats, Clipboard, Setter};
use std::path::Path;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HWND};
use windows::Win32::System::DataExchange::SetClipboardData;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

const CF_DIBV5: u32 = 17;
const BITMAPV5HEADER_SIZE: usize = 124;
const MAX_DIB_BYTES: usize = 128 * 1024 * 1024 + BITMAPV5HEADER_SIZE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageClipboardWriteResult {
    Complete,
    FilesOnly,
}

pub fn copy_image_file_and_bitmap_to_clipboard(
    path: &Path,
    width: u32,
    height: u32,
    rgba: &[u8],
    owner: HWND,
) -> Result<ImageClipboardWriteResult, String> {
    if owner.0.is_null() {
        return Err("No clipboard owner window available".to_string());
    }

    let dib = encode_cf_dibv5(width, height, rgba)?;
    let file_list = vec![path.to_string_lossy().to_string()];
    let _clip = Clipboard::new_attempts_for(owner.0, 10)
        .map_err(|error| format!("Failed to open clipboard: {error:?}"))?;

    clipboard_win::empty().map_err(|error| format!("Failed to clear clipboard: {error:?}"))?;
    formats::FileList
        .write_clipboard(&file_list)
        .map_err(|error| format!("Failed to write file list to clipboard: {error:?}"))?;

    if let Err(error) = set_preferred_drop_effect(DROPEFFECT_COPY) {
        log::warn!("[Clipboard] Failed to publish Preferred DropEffect: {error}");
    }

    if let Err(error) = set_global_bytes(CF_DIBV5, &dib) {
        log::warn!("[Clipboard] File was copied, but CF_DIBV5 failed: {error}");
        return Ok(ImageClipboardWriteResult::FilesOnly);
    }

    Ok(ImageClipboardWriteResult::Complete)
}

fn encode_cf_dibv5(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    if width == 0 || height == 0 {
        return Err("Image dimensions must be non-zero".to_string());
    }
    let width_i32 = i32::try_from(width).map_err(|_| "Image width is too large".to_string())?;
    let height_i32 = i32::try_from(height).map_err(|_| "Image height is too large".to_string())?;
    let pixel_bytes = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "Image dimensions overflow clipboard buffer".to_string())?;
    if rgba.len() != pixel_bytes {
        return Err("RGBA buffer size does not match image dimensions".to_string());
    }
    let total_bytes = BITMAPV5HEADER_SIZE
        .checked_add(pixel_bytes)
        .ok_or_else(|| "Clipboard buffer size overflow".to_string())?;
    if total_bytes > MAX_DIB_BYTES {
        return Err("Image is too large to copy as a bitmap".to_string());
    }

    let mut dib = Vec::new();
    dib.try_reserve_exact(total_bytes)
        .map_err(|_| "Not enough memory to prepare clipboard image".to_string())?;
    dib.resize(BITMAPV5HEADER_SIZE, 0);

    write_u32(&mut dib, 0, BITMAPV5HEADER_SIZE as u32);
    write_i32(&mut dib, 4, width_i32);
    write_i32(&mut dib, 8, -height_i32); // Top-down DIB.
    write_u16(&mut dib, 12, 1);
    write_u16(&mut dib, 14, 32);
    write_u32(&mut dib, 16, 3); // BI_BITFIELDS
    write_u32(&mut dib, 20, pixel_bytes as u32);
    write_u32(&mut dib, 40, 0x00ff_0000);
    write_u32(&mut dib, 44, 0x0000_ff00);
    write_u32(&mut dib, 48, 0x0000_00ff);
    write_u32(&mut dib, 52, 0xff00_0000);
    write_u32(&mut dib, 56, 0x7352_4742); // LCS_sRGB
    write_u32(&mut dib, 108, 4); // LCS_GM_IMAGES

    for pixel in rgba.chunks_exact(4) {
        dib.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }
    Ok(dib)
}

fn set_global_bytes(format: u32, bytes: &[u8]) -> Result<(), String> {
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len())
            .map_err(|error| format!("GlobalAlloc failed: {error:?}"))?;
        let pointer = GlobalLock(memory);
        if pointer.is_null() {
            let _ = GlobalFree(Some(memory));
            return Err("GlobalLock failed".to_string());
        }

        std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast::<u8>(), bytes.len());
        let _ = GlobalUnlock(memory);

        if let Err(error) = SetClipboardData(format, Some(HANDLE(memory.0))) {
            let _ = GlobalFree(Some(memory));
            return Err(format!("SetClipboardData failed: {error:?}"));
        }
    }
    Ok(())
}

fn write_u16(buffer: &mut [u8], offset: usize, value: u16) {
    buffer[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn write_u32(buffer: &mut [u8], offset: usize, value: u32) {
    buffer[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_i32(buffer: &mut [u8], offset: usize, value: i32) {
    buffer[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dibv5_has_top_down_bgra_pixels_and_alpha_masks() {
        let dib = encode_cf_dibv5(2, 1, &[10, 20, 30, 40, 50, 60, 70, 80]).expect("encode DIBV5");

        assert_eq!(dib.len(), BITMAPV5HEADER_SIZE + 8);
        assert_eq!(&dib[0..4], &124_u32.to_le_bytes());
        assert_eq!(&dib[4..8], &2_i32.to_le_bytes());
        assert_eq!(&dib[8..12], &(-1_i32).to_le_bytes());
        assert_eq!(&dib[40..44], &0x00ff_0000_u32.to_le_bytes());
        assert_eq!(&dib[52..56], &0xff00_0000_u32.to_le_bytes());
        assert_eq!(
            &dib[BITMAPV5HEADER_SIZE..],
            &[30, 20, 10, 40, 70, 60, 50, 80]
        );
    }

    #[test]
    fn dibv5_rejects_invalid_buffer_size() {
        let error = encode_cf_dibv5(2, 2, &[0; 4]).expect_err("buffer must be rejected");
        assert!(error.contains("does not match"));
    }

    #[test]
    fn dibv5_rejects_zero_dimensions() {
        assert!(encode_cf_dibv5(0, 1, &[]).is_err());
        assert!(encode_cf_dibv5(1, 0, &[]).is_err());
    }
}
