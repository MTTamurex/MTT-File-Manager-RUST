use super::*;
use crate::window::{Theme, WindowAttributes};

#[test]
fn composition_creation_policy_is_scoped_and_unwind_safe() {
    assert!(!composition_creation_enabled());
    let _ = std::panic::catch_unwind(|| {
        with_composition_creation(|| {
            assert!(composition_creation_enabled());
            with_composition_creation(|| assert!(composition_creation_enabled()));
            assert!(composition_creation_enabled());
            panic!("simulated initialization failure");
        })
    });
    assert!(!composition_creation_enabled());
}

fn rect(left: i32, top: i32, width: i32, height: i32) -> RECT {
    RECT {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}

#[test]
fn disabled_windows_keep_the_original_resize_order() {
    let mut state = LiveResize::default();
    assert!(state
        .begin(
            rect(0, 0, 640, 480),
            PhysicalSize::new(320, 240),
            true,
            true
        )
        .is_none());
    assert!(state.pending_size().is_none());
}

#[test]
fn prospective_geometry_is_visible_during_early_render() {
    let mut state = LiveResize::default();
    state.set_enabled(true);
    let size = state.begin(
        rect(-400, 60, 640, 480),
        PhysicalSize::new(320, 240),
        true,
        true,
    );
    assert_eq!(size, Some(PhysicalSize::new(640, 480)));
    assert_eq!(state.pending_size(), size);
    assert_eq!(
        state.pending_position(),
        Some(PhysicalPosition::new(-400, 60))
    );
    // Reentrant window messages cannot overwrite the geometry being rendered.
    assert!(state
        .begin(
            rect(0, 0, 800, 600),
            PhysicalSize::new(320, 240),
            true,
            true
        )
        .is_none());
    assert_eq!(state.pending_size(), size);
}

#[test]
fn resize_never_dispatches_into_an_active_application_callback() {
    let mut state = LiveResize::default();
    state.set_enabled(true);
    assert!(state
        .begin(
            rect(0, 0, 640, 480),
            PhysicalSize::new(320, 240),
            true,
            false
        )
        .is_none());
    assert!(state.pending_size().is_none());
}

#[test]
fn programmatic_resize_move_only_and_empty_rectangles_are_ignored() {
    let mut state = LiveResize::default();
    state.set_enabled(true);
    assert!(state
        .begin(
            rect(0, 0, 640, 480),
            PhysicalSize::new(320, 240),
            false,
            true
        )
        .is_none());
    assert!(state
        .begin(
            rect(100, 100, 320, 240),
            PhysicalSize::new(320, 240),
            true,
            true
        )
        .is_none());
    for proposed in [rect(0, 0, 0, 10), rect(0, 0, 10, 0), rect(0, 0, -1, 10)] {
        assert!(state
            .begin(proposed, PhysicalSize::new(320, 240), true, true)
            .is_none());
    }
    assert!(rect_size(RECT {
        left: i32::MIN,
        right: i32::MAX,
        top: 0,
        bottom: 1
    })
    .is_none());
}

#[test]
fn pending_geometry_is_cleared_even_when_present_unwinds() {
    let mut initial = WindowState::new(&WindowAttributes::default(), 1.0, Theme::Light, None);
    initial.live_resize.set_enabled(true);
    initial
        .live_resize
        .begin(
            rect(0, 0, 640, 480),
            PhysicalSize::new(320, 240),
            true,
            true,
        )
        .unwrap();
    let state = Mutex::new(initial);
    let _ = std::panic::catch_unwind(|| {
        let _guard = PendingResize(&state);
        panic!("simulated rendering failure");
    });
    assert!(state.lock().unwrap().live_resize.pending_size().is_none());
}

#[test]
fn prepared_frame_suppresses_only_the_matching_final_resize_once() {
    let mut state = LiveResize::default();
    state.set_enabled(true);
    let size = PhysicalSize::new(640, 480);
    assert!(state.finish(size));
    assert!(state.on_native_size(size));
    assert!(!state.on_native_size(size));
    state.finish(size);
    assert!(!state.on_native_size(PhysicalSize::new(800, 600)));
    assert!(!state.on_native_size(size));
}

#[test]
fn reentrant_native_resize_invalidates_prepared_frame() {
    let mut state = LiveResize::default();
    state.set_enabled(true);
    let size = state
        .begin(
            rect(0, 0, 640, 480),
            PhysicalSize::new(320, 240),
            true,
            true,
        )
        .unwrap();
    assert!(!state.on_native_size(PhysicalSize::new(800, 600)));
    assert!(!state.finish(size));
    state.pending = None;
    assert!(!state.on_native_size(size));
}
