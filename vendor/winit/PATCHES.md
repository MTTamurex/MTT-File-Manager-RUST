# Local Windows live-resize patch

Upstream: winit 0.30.13, with its original license and source retained.

The build dependency `cfg_aliases` requires at least 0.2.2, which fixes trailing
semicolons in macro expressions rejected by newer Rust compilers. No warning
suppression is applied.

`WindowExtWindows::set_present_before_resize(true)` opts one window into an
additional synchronous `Resized` callback after `WM_NCCALCSIZE` calculates the
prospective client rectangle, but before that rectangle becomes visible.

During the callback, `inner_size()` and `inner_position()` expose the prospective
geometry so eframe sizes its surface and UI consistently. An unwind-safe guard
restores normal native geometry queries afterwards. A matching subsequent
`WM_SIZE` updates native flags without a redundant `Resized` callback: otherwise
eframe would release/reconfigure the just-presented buffers again. A different
size, or a reentrant native resize during the callback, keeps the normal event.

The patch is disabled by default and does not dispatch early events for moves
without size changes, empty/minimized clients, programmatic resizing outside a
live move/resize, nested early renders, or when winit must buffer events because
an application callback is already active.

The file `src/platform_impl/windows/live_resize.rs` contains the implementation
and tests. Existing Windows source files contain only API/state wiring and the
notification after non-client calculation. Non-Windows backends are unchanged.

The application enables this only for standalone DX12 viewers and uses a
DirectComposition swapchain. The manager, Vulkan and OpenGL do not opt in.

`with_direct_composition_window` scopes window creation on the calling thread
to set `WS_EX_NOREDIRECTIONBITMAP` at creation time (Windows rejects setting it
afterwards). winit retains that flag in its own cached window state. The scope
restores the previous policy on normal exit or unwind, including failed backend
initialization followed by Vulkan/OpenGL fallback. After an early frame, the
non-client calculation returns zero to avoid copying stale client pixels.
