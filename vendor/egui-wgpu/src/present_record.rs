//! MTT patch (see PATCHES.md): size of the last frame presented on the root
//! viewport.
//!
//! The host hides its window during startup and reveals it only once a frame
//! exists at the final window size. Counting egui passes is not enough: a pass
//! can run before the swapchain was reconfigured to the new size, and with a
//! copy-based present (Vulkan on Intel) the screen keeps showing the previous,
//! smaller frame. This records what actually reached `present`.

use std::sync::atomic::{AtomicU64, Ordering};

/// `width << 32 | height` of the last root present; 0 before the first one.
static ROOT_PRESENT_SIZE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn record_root_present(size_in_pixels: [u32; 2]) {
    let packed = (u64::from(size_in_pixels[0]) << 32) | u64::from(size_in_pixels[1]);
    ROOT_PRESENT_SIZE.store(packed, Ordering::Release);
}

/// Size in pixels of the last frame presented on the root viewport, or `None`
/// before the first present.
pub fn last_root_present_size() -> Option<[u32; 2]> {
    let packed = ROOT_PRESENT_SIZE.load(Ordering::Acquire);
    (packed != 0).then_some([(packed >> 32) as u32, packed as u32])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_the_last_root_present_size() {
        record_root_present([2560, 1369]);
        assert_eq!(last_root_present_size(), Some([2560, 1369]));
    }
}
