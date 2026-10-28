use super::super::{
    graphics::Bitmap,
    model::{Entry, LocationKind, Snapshot, Workspace},
    tests::NativeWindow,
};
use super::*;
use crate::app::navigation_state::ThemeMode;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, GdiFlush, SelectObject, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
};

#[test]
fn offscreen_paint_renders_all_application_palettes_at_multiple_dpi_scales() {
    let windows = [
        NativeWindow::new(),
        NativeWindow::new(),
        NativeWindow::new(),
    ];
    for dpi in [96u32, 144, 192] {
        for (mode, expanded) in [
            ThemeMode::Light,
            ThemeMode::Dark,
            ThemeMode::DarkDeep,
            ThemeMode::GitHubDark,
        ]
        .into_iter()
        .flat_map(|mode| [(mode, false), (mode, true)])
        {
            let mut model = Model::new(
                windows
                    .iter()
                    .zip([
                        (r"C:\Documents\Example", LocationKind::Folder),
                        ("Tag: Example", LocationKind::Tag),
                        ("Computer", LocationKind::Computer),
                    ])
                    .map(|(window, (path, icon))| {
                        (
                            WindowRef::capture(window.0).unwrap(),
                            Snapshot {
                                active_path: path.into(),
                                active_icon: icon,
                                hidden_at: (icon != LocationKind::Computer)
                                    .then(|| "2026-01-01 12:30:00".into()),
                                workspaces: vec![Workspace {
                                    tab: Entry {
                                        id: 1,
                                        path: path.into(),
                                        icon,
                                    },
                                    panels: (icon == LocationKind::Folder).then(|| {
                                        [
                                            vec![Entry {
                                                id: 2,
                                                path: "Tag: Example".into(),
                                                icon: LocationKind::Tag,
                                            }],
                                            vec![Entry {
                                                id: 3,
                                                path: "C:\\".into(),
                                                icon: LocationKind::Folder,
                                            }],
                                        ]
                                    }),
                                }],
                            },
                        )
                    })
                    .collect(),
            );
            if expanded {
                model.toggle(0);
            }
            let width = layout::WIDTH as u32 * dpi / 96;
            let height =
                (model.content_height() + layout::HEADER + layout::FOOTER) as u32 * dpi / 96;
            let layout = Layout {
                dpi,
                bounds: RECT {
                    left: 0,
                    top: 0,
                    right: width as i32,
                    bottom: height as i32,
                },
                width: layout::WIDTH,
                height: (height * 96 / dpi) as i32,
            };
            let mut resources = Resources::new(dpi).unwrap();
            let ctx = eframe::egui::Context::default();
            crate::ui::theme::apply_theme_visuals(&ctx, mode);
            resources.colors = resources::Colors::application(&ctx);
            let state = State {
                model,
                layout,
                resources,
                owner: windows[0].0,
                anchor: POINT::default(),
                done: false,
                resize: false,
                pressed: None,
                hot_close: None,
                command: None,
            };
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width as i32,
                    biHeight: -(height as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let bitmap = Bitmap(unsafe {
                CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0).unwrap()
            });
            unsafe {
                let dc = CreateCompatibleDC(None);
                assert!(!dc.is_invalid());
                let previous = SelectObject(dc, bitmap.0.into());
                paint::paint(dc, &state);
                let _ = GdiFlush();
                SelectObject(dc, previous);
                let _ = DeleteDC(dc);
            }
            let (mut rgba, actual_width, actual_height) =
                crate::infrastructure::windows::bitmap_conversion::hbitmap_to_rgba(bitmap.0)
                    .unwrap();
            assert_eq!((actual_width, actual_height), (width, height));
            let colors = rgba
                .chunks_exact(4)
                .map(|pixel| &pixel[..3])
                .collect::<std::collections::HashSet<_>>();
            assert!(
                colors.len() > 100,
                "icons and antialiased text must be drawn"
            );
            // Opt-in test artifacts for visual review; never captures user data.
            if let Some(directory) = std::env::var_os("MTT_TRAY_PREVIEW_DIR") {
                for pixel in rgba.chunks_exact_mut(4) {
                    pixel[3] = 255;
                }
                let image = image::RgbaImage::from_raw(width, height, rgba).unwrap();
                let mode = mode.preference();
                let expansion = if expanded { "-expanded" } else { "" };
                image
                    .save(
                        std::path::PathBuf::from(directory)
                            .join(format!("tray-popup-{mode}-{dpi}{expansion}.png")),
                    )
                    .unwrap();
            }
        }
    }
}
