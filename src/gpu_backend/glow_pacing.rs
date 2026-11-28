//! Frame pacing for the Glow (OpenGL) backend.
//!
//! On Windows the driver may queue several OpenGL frames behind `SwapBuffers`
//! and block in bursts, while DWM composes on its own clock. The uneven present
//! cadence shows up as random hitches in interpolated scrolling. Calling
//! `glFinish` at the start of a frame (right after the previous swap) waits for
//! that swap, so no frame is queued ahead of the display.

/// Wait for the previous frame's present. Must run at the start of a Glow
/// frame, with the frame's GL context current; no-op on other backends.
pub fn wait_for_previous_present(frame: &eframe::Frame) {
    if let Some(gl) = frame.gl() {
        use eframe::glow::HasContext as _;
        // SAFETY: eframe makes the viewport's GL context current before
        // calling into the app on this thread.
        unsafe { gl.finish() };
    }
}
