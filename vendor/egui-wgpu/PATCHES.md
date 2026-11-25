# Local opaque-root-swapchain patch

Upstream: egui-wgpu 0.35.0 (crates.io), source otherwise unchanged.

eframe decides once, from the root viewport's `transparent` flag, whether every
wgpu surface gets an alpha-capable `CompositeAlphaMode`. MTT keeps the root
window transparent only so the floating tab-drag preview viewport can have a
transparent shadow area; the main UI itself always paints an opaque background.

An alpha-composited root swapchain made DWM blend the whole main window every
frame. On an integrated GPU that halved the presentation rate while scrolling
(frames alternated between 16 and 33 ms; CPU work stayed at 2-4 ms per frame).
With an opaque root swapchain the same scenario ran at a steady 60 fps.

`Painter::install_surface` therefore requests premultiplied/postmultiplied alpha
only for non-root viewports. Set `MTT_WGPU_ROOT_ALPHA=1` to restore the upstream
behavior for comparison.
