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
        assert_eq!(style & WS_VISIBLE.0, 0);
        preview.restore_from_overlay();
        let style = unsafe { GetWindowLongPtrW(child, GWL_STYLE) as u32 };
        assert_eq!(style & WS_VISIBLE.0, 0);

        preview
            .first_frame_ready
            .store(true, std::sync::atomic::Ordering::Release);
        preview.set_visibility(true);
        let style = unsafe { GetWindowLongPtrW(child, GWL_STYLE) as u32 };
        assert_ne!(style & WS_VISIBLE.0, 0);

        preview.hide_for_overlay();
        assert_eq!(
            unsafe { GetWindowLongPtrW(child, GWL_STYLE) as u32 } & WS_VISIBLE.0,
            0
        );
        preview.restore_from_overlay();
        assert_ne!(
            unsafe { GetWindowLongPtrW(child, GWL_STYLE) as u32 } & WS_VISIBLE.0,
            0
        );
    }

    #[test]
    #[ignore = "requires libmpv D3D11 output and a Windows desktop"]
    fn real_d3d11_startup_releases_visibility_after_playback_restart() {
        use crate::ui::components::mpv_preview::MpvPreview;
        use std::sync::atomic::Ordering;
        use std::sync::Arc;
        use std::time::{Duration, Instant};
        use windows::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
        };

        let fixture = SurfaceFixture::new();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("frame.bmp");
        // A generated image exercises the real video output without an external
        // encoder, personal media, or a checked-in binary fixture.
        image::RgbImage::from_pixel(64, 64, image::Rgb([20, 40, 60]))
            .save(&path)
            .unwrap();
        let mut preview = MpvPreview::new(path.clone());
        let mpv = Arc::new(
            mpv::Mpv::with_initializer(|init| {
                init.set_option("config", false)?;
                init.set_option("load-scripts", false)?;
                init.set_option("vo", "gpu")?;
                init.set_option("gpu-api", "d3d11")?;
                init.set_option("gpu-context", "d3d11")?;
                init.set_option("pause", true)?;
                Ok(())
            })
            .unwrap(),
        );
        preview.surface.main_hwnd = fixture.surface.main_hwnd();
        preview.surface.ensure_child_window(&mpv);
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            preview.surface.sync_rect(
                ui,
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(320.0, 180.0)),
            );
        });
        preview.mpv = Some(mpv.clone());
        preview.start_event_loop_internal(mpv.clone(), ctx);
        preview.set_visibility(true);
        let child = preview.surface.hwnd().unwrap();
        assert_eq!(
            unsafe { GetWindowLongPtrW(child, GWL_STYLE) as u32 } & WS_VISIBLE.0,
            0
        );

        let generation = preview.begin_file_load();
        mpv.command("loadfile", &[path.to_str().unwrap()]).unwrap();
        preview.finish_file_load(generation, true);

        let deadline = Instant::now() + Duration::from_secs(15);
        while !preview.first_frame_ready.load(Ordering::Acquire) && Instant::now() < deadline {
            // MPV's native parent hook needs the owning thread's message loop.
            unsafe {
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            preview.first_frame_ready.load(Ordering::Acquire),
            "MPV did not reach PlaybackRestart while its host was hidden"
        );
        assert_eq!(mpv.get_property::<String>("current-vo").unwrap(), "gpu");
        preview.set_visibility(true);
        assert_ne!(
            unsafe { GetWindowLongPtrW(child, GWL_STYLE) as u32 } & WS_VISIBLE.0,
            0
        );
    }
}
