use super::*;
use windows::Win32::Graphics::Gdi::{GetObjectW, BITMAP};
use windows::Win32::UI::WindowsAndMessaging::GetIconInfo;

#[test]
fn native_bitmap_and_icon_keep_colors_alpha_and_orientation() {
    let mut pixels = Pixmap::new(16, 16).unwrap();
    pixels.data_mut()[..4].copy_from_slice(&[64, 128, 192, 255]);
    pixels.data_mut()[4..8].copy_from_slice(&[32, 64, 96, 128]);
    for (pixel, alpha) in pixels.data_mut()[8..28]
        .chunks_exact_mut(4)
        .zip([1u8, 32, 64, 192, 254])
    {
        pixel.copy_from_slice(&[alpha / 4, alpha / 2, alpha, alpha]);
    }
    let bitmap = Bitmap::from_pixmap(&pixels).unwrap();
    let (rgba, width, height) =
        crate::infrastructure::windows::bitmap_conversion::hbitmap_to_rgba(bitmap.0).unwrap();
    assert_eq!((width, height), (16, 16));
    assert_eq!(rgba, pixels.data());
    let icon = Icon::from_pixmap(&pixels).unwrap();
    let mut info = ICONINFO::default();
    unsafe { GetIconInfo(icon.0, &mut info).unwrap() };
    let color = Bitmap(info.hbmColor);
    let _mask = Bitmap(info.hbmMask);
    let (stored, _, _) =
        crate::infrastructure::windows::bitmap_conversion::hbitmap_to_rgba(color.0).unwrap();
    assert_eq!(
        &stored[4..8],
        &[64, 128, 191, 128],
        "straight-alpha icon pixel"
    );
    let (rgba, _, _) =
        crate::infrastructure::windows::bitmap_conversion::hicon_to_rgba(icon.0).unwrap();
    assert_eq!(&rgba[..4], &[64, 128, 192, 255]);
    // Straight-alpha conversion and GDI blending can lose one RGB level.
    // Alpha must remain exact; applying it twice would greatly exceed this bound.
    for (actual, expected) in rgba.chunks_exact(4).zip(pixels.data().chunks_exact(4)) {
        assert_eq!(actual[3], expected[3]);
        for channel in 0..3 {
            assert!(
                actual[channel].abs_diff(expected[channel]) <= 1,
                "rendered {actual:?}, expected {expected:?}"
            );
        }
    }
    assert_eq!(rgba[rgba.len() - 1], 0);
}

#[test]
fn owned_native_graphics_are_released_on_drop() {
    let mut pixels = Pixmap::new(32, 32).unwrap();
    pixels.fill(tiny_skia::Color::from_rgba8(50, 100, 200, 255));
    let bitmap = Bitmap::from_pixmap(&pixels).unwrap();
    let raw = bitmap.0;
    drop(bitmap);
    let mut info = BITMAP::default();
    assert_eq!(
        unsafe {
            GetObjectW(
                raw.into(),
                size_of::<BITMAP>() as i32,
                Some((&mut info as *mut BITMAP).cast()),
            )
        },
        0
    );
    let icon = Icon::from_pixmap(&pixels).unwrap();
    let raw = icon.0;
    drop(icon);
    let mut info = ICONINFO::default();
    let invalid = unsafe { GetIconInfo(raw, &mut info) }.is_err();
    if !invalid {
        unsafe {
            let _ = DeleteObject(info.hbmColor.into());
            let _ = DeleteObject(info.hbmMask.into());
        }
    }
    assert!(invalid);
}
