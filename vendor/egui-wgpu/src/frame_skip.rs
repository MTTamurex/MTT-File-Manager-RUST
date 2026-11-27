//! MTT patch (see PATCHES.md): skip GPU work for frames identical to the last
//! presented one.
//!
//! Background bookkeeping (drive polling, memory maintenance, worker results)
//! wakes eframe for a pass even when nothing on screen changes. Rendering and
//! presenting that identical frame costs GPU time on every adapter in the
//! presentation path, which shows up as GPU activity while the app is idle.
//! The compositor keeps showing the last presented frame, so an unchanged
//! frame does not need to be rendered again.

use std::hash::{BuildHasher, Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Off by default: only a host that reports native repaint needs through
/// [`invalidate_presented_frames`] can safely reuse what is on screen.
static SKIP_ENABLED: AtomicBool = AtomicBool::new(false);
/// Bumped whenever the OS may have discarded or invalidated presented content.
static PRESENT_EPOCH: AtomicU64 = AtomicU64::new(0);

/// Opts this process into skipping identical frames.
///
/// The caller must call [`invalidate_presented_frames`] whenever the window
/// manager may need a fresh image (minimize/restore, show/hide, display or
/// DPI changes, `WM_PAINT` with an update region): DWM does not keep the
/// last presented image across minimize, so skipping there leaves the window
/// empty.
pub fn set_identical_frame_skip(enabled: bool) {
    SKIP_ENABLED.store(enabled, Ordering::Release);
    invalidate_presented_frames();
}

/// Forces the next frame of every surface to be rendered and presented.
pub fn invalidate_presented_frames() {
    PRESENT_EPOCH.fetch_add(1, Ordering::AcqRel);
}

pub(crate) fn present_epoch() -> u64 {
    PRESENT_EPOCH.load(Ordering::Acquire)
}

/// Identity of everything that determines the pixels of one surface frame.
///
/// Returns `None` when the frame cannot be compared (paint callbacks draw
/// arbitrary content), which always forces a real paint.
pub(crate) fn frame_signature(
    size_in_pixels: [u32; 2],
    pixels_per_point: f32,
    clear_color: [f32; 4],
    clipped_primitives: &[epaint::ClippedPrimitive],
) -> Option<u64> {
    // Fixed seeds: signatures are compared only within one process run.
    let mut hasher = ahash::RandomState::with_seeds(
        0x6d74_745f_6672_616d,
        0x655f_736b_6970_5f73,
        0x6967_6e61_7475_7265,
        0x7761_7067_7075_2121,
    )
    .build_hasher();
    size_in_pixels.hash(&mut hasher);
    pixels_per_point.to_bits().hash(&mut hasher);
    for channel in clear_color {
        channel.to_bits().hash(&mut hasher);
    }
    clipped_primitives.len().hash(&mut hasher);
    for clipped in clipped_primitives {
        let rect = clipped.clip_rect;
        for value in [rect.min.x, rect.min.y, rect.max.x, rect.max.y] {
            value.to_bits().hash(&mut hasher);
        }
        match &clipped.primitive {
            epaint::Primitive::Mesh(mesh) => {
                mesh.texture_id.hash(&mut hasher);
                mesh.indices.len().hash(&mut hasher);
                hasher.write(bytemuck::cast_slice(&mesh.indices));
                mesh.vertices.len().hash(&mut hasher);
                hasher.write(bytemuck::cast_slice(&mesh.vertices));
            }
            epaint::Primitive::Callback(_) => return None,
        }
    }
    Some(hasher.finish())
}

/// Whether a frame can reuse what is already on screen.
/// `last_presented` is the signature and present epoch of the frame on screen.
pub(crate) fn can_skip_frame(
    signature: Option<u64>,
    epoch: u64,
    last_presented: Option<(u64, u64)>,
    textures_changed: bool,
    capture_requested: bool,
) -> bool {
    SKIP_ENABLED.load(Ordering::Acquire)
        && !skip_disabled()
        && !textures_changed
        && !capture_requested
        && signature.is_some_and(|signature| last_presented == Some((signature, epoch)))
}

/// `MTT_WGPU_PRESENT_UNCHANGED=1` restores upstream behavior (every requested
/// frame is rendered and presented) for A/B measurements.
fn skip_disabled() -> bool {
    static DISABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DISABLED.get_or_init(|| {
        std::env::var("MTT_WGPU_PRESENT_UNCHANGED").is_ok_and(|value| value.trim() == "1")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use epaint::{ClippedPrimitive, Color32, Mesh, Primitive, Rect, pos2};

    fn frame(color: Color32) -> Vec<ClippedPrimitive> {
        let mut mesh = Mesh::default();
        mesh.add_colored_rect(Rect::from_min_max(pos2(0.0, 0.0), pos2(10.0, 10.0)), color);
        vec![ClippedPrimitive {
            clip_rect: Rect::EVERYTHING,
            primitive: Primitive::Mesh(mesh),
        }]
    }

    fn signature(primitives: &[ClippedPrimitive]) -> Option<u64> {
        frame_signature([800, 600], 1.0, [0.0; 4], primitives)
    }

    #[test]
    fn identical_frames_share_a_signature() {
        assert_eq!(
            signature(&frame(Color32::RED)),
            signature(&frame(Color32::RED))
        );
    }

    #[test]
    fn any_visual_input_changes_the_signature() {
        let base = signature(&frame(Color32::RED));
        assert_ne!(base, signature(&frame(Color32::BLUE)));
        assert_ne!(
            base,
            frame_signature([801, 600], 1.0, [0.0; 4], &frame(Color32::RED))
        );
        assert_ne!(
            base,
            frame_signature([800, 600], 1.25, [0.0; 4], &frame(Color32::RED))
        );
        assert_ne!(
            base,
            frame_signature([800, 600], 1.0, [1.0, 0.0, 0.0, 1.0], &frame(Color32::RED))
        );
    }

    #[test]
    fn skip_requires_opt_in_a_matching_frame_and_the_same_epoch() {
        let sig = signature(&frame(Color32::RED));
        let shown = sig.map(|sig| (sig, 7));
        SKIP_ENABLED.store(false, Ordering::Release);
        assert!(!can_skip_frame(sig, 7, shown, false, false));

        SKIP_ENABLED.store(true, Ordering::Release);
        assert!(can_skip_frame(sig, 7, shown, false, false));
        // The OS invalidated the window since that present.
        assert!(!can_skip_frame(sig, 8, shown, false, false));
        assert!(!can_skip_frame(sig, 7, None, false, false));
        assert!(!can_skip_frame(sig, 7, shown, true, false));
        assert!(!can_skip_frame(sig, 7, shown, false, true));
        assert!(!can_skip_frame(None, 7, shown, false, false));
        SKIP_ENABLED.store(false, Ordering::Release);
    }
}
