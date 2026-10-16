//! Application UI scaling, applied on top of the monitor's native Windows DPI.

use eframe::egui;

pub(crate) const PREFERENCE_KEY: &str = "interface_scale";
pub(crate) const DEFAULT: f32 = 1.0;
pub(crate) const MAX: f32 = 2.0;

pub(crate) fn from_preference(value: Option<&str>) -> f32 {
    value
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| value.is_finite())
        .map(|value| value.clamp(DEFAULT, MAX))
        .unwrap_or(DEFAULT)
}

pub(crate) fn apply(ctx: &egui::Context, scale: f32) {
    // The app owns zoom: Ctrl+wheel changes file views, not interface scale.
    ctx.options_mut(|options| options.zoom_with_keyboard = false);
    if (ctx.zoom_factor() - scale).abs() > f32::EPSILON {
        ctx.set_zoom_factor(scale);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_composes_with_native_dpi_and_can_be_reset() {
        let ctx = egui::Context::default();
        apply(&ctx, 1.5);
        let mut input = egui::RawInput::default();
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .native_pixels_per_point = Some(2.0);
        let _ = ctx.run_ui(input.clone(), |_| {});
        assert_eq!(ctx.pixels_per_point(), 3.0);
        assert!(!ctx.options(|options| options.zoom_with_keyboard));

        apply(&ctx, DEFAULT);
        let _ = ctx.run_ui(input, |_| {});
        assert_eq!(ctx.pixels_per_point(), 2.0);
    }
}
