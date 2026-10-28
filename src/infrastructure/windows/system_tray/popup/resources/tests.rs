use super::*;
use crate::app::navigation_state::ThemeMode;

#[test]
fn popup_uses_the_owner_application_palette_instead_of_the_windows_theme() {
    for mode in [
        ThemeMode::Light,
        ThemeMode::Dark,
        ThemeMode::DarkDeep,
        ThemeMode::GitHubDark,
    ] {
        let ctx = eframe::egui::Context::default();
        crate::ui::theme::apply_theme_visuals(&ctx, mode);
        let colors = Colors::application(&ctx);
        let style = ctx.global_style();
        let reference = |color: eframe::egui::Color32| rgb(color.r(), color.g(), color.b());
        assert_eq!(colors.background, reference(style.visuals.window_fill));
        assert_eq!(colors.text, reference(style.visuals.text_color()));
        assert_eq!(colors.border, reference(style.visuals.window_stroke.color));
        assert_eq!(
            colors.accent,
            reference(crate::ui::theme::accent_color(&ctx))
        );
    }
}
