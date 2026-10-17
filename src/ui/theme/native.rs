use super::current_palette;
use crate::infrastructure::windows::window_corners;
use eframe::egui::{Color32, Context};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, IsWindow, IsWindowVisible, GA_ROOT};

pub(super) fn title_bar_colors(ctx: &Context) -> Option<([u8; 3], [u8; 3])> {
    let rgb = |color: Color32| [color.r(), color.g(), color.b()];
    current_palette(ctx).map(|p| (rgb(p.background), rgb(p.text)))
}

/// Keep native viewer captions aligned with the main window's custom tab strip.
/// Legacy themes restore the OS defaults instead of retaining custom colors.
pub fn apply_native_title_bar(ctx: &Context, hwnd: HWND) {
    let mode = super::current_mode(ctx);
    let colors = title_bar_colors(ctx);
    let dark = ctx.global_style().visuals.dark_mode;
    let (valid, visible, root) = unsafe {
        (
            IsWindow(Some(hwnd)).as_bool(),
            IsWindowVisible(hwnd).as_bool(),
            GetAncestor(hwnd, GA_ROOT) == hwnd,
        )
    };
    let dark_result = window_corners::apply_dark_title_bar(hwnd, dark);
    let color_result = window_corners::apply_title_bar_colors(hwnd, colors);
    log::info!(
        "[NATIVE-THEME] mode={} dark={} valid={} visible={} root={} colors={:?} dark_result={:?} color_result={:?}",
        mode.preference(), dark, valid, visible, root, colors,
        dark_result.as_ref().map_err(|error| error.code()),
        color_result.as_ref().map_err(|error| (error.code(), error.message())),
    );
    if let Err(error) = color_result {
        if colors.is_some() {
            log::warn!("[NATIVE-THEME] Custom native caption colors were not applied: {error}");
        }
    }
}
