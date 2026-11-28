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

# Local identical-frame skip patch

Background bookkeeping (drive polling, memory maintenance, worker results)
wakes eframe for a UI pass even when nothing on screen changes. Upstream renders
and presents every such frame, which shows up as periodic GPU activity while the
app sits idle in the background, on both adapters when the render and display
GPUs differ.

`Painter::paint_and_update_textures` now hashes everything that determines a
surface's pixels (`src/frame_skip.rs`: size, pixels-per-point, clear color, clip
rects, texture ids, vertices and indices). When the hash matches the frame last
presented on that surface, no texture changed, no screenshot is requested, no
paint callback is present and the surface needs no reconfiguration, the frame
is skipped before any buffer upload, swapchain acquire or present. The
compositor keeps showing the last presented image. Any surface (re)configure or
acquire failure clears the stored hash, so the next frame always paints.

DWM does not keep the presented image across every window-manager transition:
after minimize/restore an unpainted window shows nothing. The skip is therefore
opt-in (`winit::set_identical_frame_skip`) and each stored hash carries a global
present epoch. A host that enables the skip must call
`winit::invalidate_presented_frames` whenever the OS may need new pixels. MTT
enables it from the main window subclass, which invalidates on `WM_SIZE`,
`WM_SHOWWINDOW`, `WM_DISPLAYCHANGE`, `WM_DPICHANGED` and `WM_PAINT` with a real
update region. Viewer processes do not opt in and keep upstream behavior.

Set `MTT_WGPU_PRESENT_UNCHANGED=1` to restore the upstream behavior for
comparison.

# Local root present size record

The host cloaks its main window during startup and reveals it once a frame
exists at the final window size. Counting egui passes is not enough: a pass can
run before the swapchain is reconfigured to the new size, and with a copy-based
present (Vulkan on Intel, "Composed: Copy with GPU GDI") the screen keeps
showing the previous, smaller frame after the reveal.

`Painter::paint_and_update_textures` records the surface size of every frame
presented on the root viewport (`src/present_record.rs`), and
`winit::last_root_present_size` exposes it to the host.
