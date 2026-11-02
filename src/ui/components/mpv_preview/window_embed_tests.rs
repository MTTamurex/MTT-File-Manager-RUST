use super::{overlay_pixel_regions, PixelRoundedRect};
use eframe::egui;

fn popup_overlay(rect: egui::Rect) -> crate::ui::video_overlay::VideoOverlay {
    crate::ui::video_overlay::VideoOverlay {
        rect,
        corner_radius: 6.0,
    }
}

#[test]
fn converts_overlay_intersection_to_video_local_pixels() {
    let video = egui::Rect::from_min_max(egui::pos2(100.0, 50.0), egui::pos2(300.0, 150.0));
    let overlay_rect = egui::Rect::from_min_max(egui::pos2(250.0, 25.0), egui::pos2(350.0, 100.0));

    assert_eq!(
        overlay_pixel_regions(video, &[popup_overlay(overlay_rect)], 1.5),
        vec![PixelRoundedRect {
            left: 225,
            top: -38,
            right: 375,
            bottom: 75,
            corner_diameter: 18,
        }]
    );
}

#[test]
fn ignores_overlays_outside_video() {
    let video = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(200.0, 100.0));
    let overlay_rect = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(20.0, 20.0));

    assert!(overlay_pixel_regions(video, &[popup_overlay(overlay_rect)], 1.25).is_empty());
}

#[cfg(target_os = "windows")]
mod native {
    use super::*;
    use crate::ui::components::mpv_preview::VideoSurface;
    use windows::core::w;
    use windows::Win32::Foundation::RECT;
    use windows::Win32::System::SystemServices::{SS_BLACKRECT, SS_TYPEMASK};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, GetWindowLongPtrW, GetWindowRect, GWL_STYLE,
        WINDOW_EX_STYLE, WINDOW_STYLE, WS_CHILD, WS_CLIPCHILDREN, WS_VISIBLE,
    };

    struct SurfaceFixture {
        surface: VideoSurface,
    }

    impl SurfaceFixture {
        fn new() -> Self {
            let parent = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("static"),
                    w!(""),
                    WINDOW_STYLE::default(),
                    0,
                    0,
                    640,
                    480,
                    None,
                    None,
                    None,
                    None,
                )
                .expect("create hidden test parent")
            };
            let mut surface = VideoSurface::new();
            surface.main_hwnd = Some(parent);
            surface.mpv_hwnd = Some(super::super::create_surface_window(parent).unwrap());
            Self { surface }
        }

        fn style(&self) -> u32 {
            unsafe { GetWindowLongPtrW(self.surface.hwnd().unwrap(), GWL_STYLE) as u32 }
        }
    }

    impl Drop for SurfaceFixture {
        fn drop(&mut self) {
            self.surface.destroy();
            unsafe {
                let _ = DestroyWindow(self.surface.main_hwnd().unwrap());
            }
        }
    }

    #[test]
    fn native_surface_starts_hidden_with_black_background() {
        let fixture = SurfaceFixture::new();
        assert_ne!(fixture.style() & WS_CHILD.0, 0);
        assert_ne!(fixture.style() & WS_CLIPCHILDREN.0, 0);
        assert_eq!(fixture.style() & WS_VISIBLE.0, 0);
        assert_eq!(fixture.style() & SS_TYPEMASK.0, SS_BLACKRECT.0);

        let mut bounds = RECT::default();
        unsafe { GetWindowRect(fixture.surface.hwnd().unwrap(), &mut bounds).unwrap() };
        assert_eq!(bounds.right - bounds.left, 1);
        assert_eq!(bounds.bottom - bounds.top, 1);
    }

    #[test]
    fn native_surface_only_shows_after_layout_and_can_be_restored() {
        let mut fixture = SurfaceFixture::new();
        fixture.surface.set_visible(true);
        assert_eq!(fixture.style() & WS_VISIBLE.0, 0);

        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::pos2(20.0, 30.0), egui::vec2(320.0, 180.0));
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            fixture.surface.sync_rect(ui, rect);
        });
        assert_eq!(fixture.style() & WS_VISIBLE.0, 0);
        fixture.surface.set_visible(true);
        assert_ne!(fixture.style() & WS_VISIBLE.0, 0);

        fixture.surface.set_visible(false);
        assert_eq!(fixture.style() & WS_VISIBLE.0, 0);
        fixture.surface.set_visible(true);
        assert_ne!(fixture.style() & WS_VISIBLE.0, 0);
    }

    #[test]
    fn preview_applies_visibility_requested_before_surface_creation() {
        use crate::ui::components::mpv_preview::MpvPreview;

        let fixture = SurfaceFixture::new();
        let mut preview = MpvPreview::new("video.mp4".into());
        preview.set_visibility(true);

        let parent = fixture.surface.main_hwnd().unwrap();
        let child = super::super::create_surface_window(parent).unwrap();
        preview.surface.main_hwnd = Some(parent);
        preview.surface.mpv_hwnd = Some(child);
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            preview.surface.sync_rect(
                ui,
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(320.0, 180.0)),
            );
        });

        preview.set_visibility(true);
        let style = unsafe { GetWindowLongPtrW(child, GWL_STYLE) as u32 };
        assert_ne!(style & WS_VISIBLE.0, 0);
    }
}
