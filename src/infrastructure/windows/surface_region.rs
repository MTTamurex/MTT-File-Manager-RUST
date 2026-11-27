//! Window region of a native video surface: the visible part (clip) minus
//! rounded cutouts where egui overlays must show through.

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, CreateRoundRectRgn, DeleteObject, SetWindowRgn, RGN_DIFF,
};

/// Rounded rectangle in surface coordinates (physical pixels).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cutout {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub corner_diameter: i32,
}

/// Visible part `(left, top, right, bottom)` of a surface at client position
/// `(x, y)` with `size`, inside a client area of `client_size`, in surface
/// coordinates. `None` when the whole surface is inside.
pub fn clip_to_client(
    (x, y): (i32, i32),
    (width, height): (i32, i32),
    (client_width, client_height): (i32, i32),
) -> Option<(i32, i32, i32, i32)> {
    let left = (-x).clamp(0, width);
    let top = (-y).clamp(0, height);
    let right = (client_width - x).clamp(left, width);
    let bottom = (client_height - y).clamp(top, height);
    (left != 0 || top != 0 || right != width || bottom != height)
        .then_some((left, top, right, bottom))
}

/// Applies the region to `hwnd`. Returns false when Windows rejected it.
pub fn apply(
    hwnd: HWND,
    (width, height): (i32, i32),
    clip: Option<(i32, i32, i32, i32)>,
    cutouts: &[Cutout],
) -> bool {
    unsafe {
        if cutouts.is_empty() && clip.is_none() {
            return SetWindowRgn(hwnd, None, true) != 0;
        }
        let (left, top, right, bottom) = clip.unwrap_or((0, 0, width.max(1), height.max(1)));
        let window_region = CreateRectRgn(left, top, right, bottom);
        if window_region.is_invalid() {
            return false;
        }
        for rect in cutouts {
            let cutout = CreateRoundRectRgn(
                rect.left,
                rect.top,
                rect.right,
                rect.bottom,
                rect.corner_diameter,
                rect.corner_diameter,
            );
            if !cutout.is_invalid() {
                let _ = CombineRgn(
                    Some(window_region),
                    Some(window_region),
                    Some(cutout),
                    RGN_DIFF,
                );
                let _ = DeleteObject(cutout.into());
            }
        }
        if SetWindowRgn(hwnd, Some(window_region), true) != 0 {
            // Windows owns window_region after a successful call.
            true
        } else {
            let _ = DeleteObject(window_region.into());
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_inside_the_client_area_needs_no_clip() {
        assert_eq!(clip_to_client((10, 20), (300, 200), (800, 600)), None);
    }

    #[test]
    fn surface_is_clipped_to_the_client_area_on_every_edge() {
        // Overhangs right and bottom.
        assert_eq!(
            clip_to_client((700, 500), (300, 200), (800, 600)),
            Some((0, 0, 100, 100))
        );
        // Overhangs left and top.
        assert_eq!(
            clip_to_client((-50, -20), (300, 200), (800, 600)),
            Some((50, 20, 300, 200))
        );
        // Entirely outside: an empty region hides it.
        assert_eq!(
            clip_to_client((900, 10), (300, 200), (800, 600)),
            Some((0, 0, 0, 200))
        );
    }
}
