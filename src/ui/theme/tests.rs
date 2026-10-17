use super::*;

#[test]
fn drive_usage_ratio_handles_capacity_boundaries() {
    assert_eq!(drive_usage_ratio(0, 0), None);
    assert_eq!(drive_usage_ratio(100, 100), Some(0.0));
    assert_eq!(drive_usage_ratio(100, 0), Some(1.0));
}

#[test]
fn drive_usage_ratio_handles_free_space_above_total() {
    assert_eq!(drive_usage_ratio(100, 120), Some(0.0));
}

#[test]
fn drive_usage_becomes_critical_only_above_ninety_percent() {
    assert_eq!(drive_usage_color(0.9), DRIVE_USAGE_NORMAL_COLOR);
    assert_eq!(drive_usage_color(0.91), DRIVE_USAGE_CRITICAL_COLOR);
}

#[test]
fn legacy_themes_keep_their_visuals_and_colors() {
    let ctx = egui::Context::default();
    for (mode, expected) in [
        (ThemeMode::Light, DRIVE_USAGE_LIGHT_BACKGROUND_COLOR),
        (ThemeMode::Dark, DRIVE_USAGE_DARK_BACKGROUND_COLOR),
    ] {
        apply_theme_visuals(&ctx, mode);
        assert_eq!(drive_usage_background_color(&ctx), expected);
        let default = if mode.resolve_dark() {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        assert_eq!(ctx.global_style().visuals.panel_fill, default.panel_fill);
    }
}

#[test]
fn every_theme_preference_round_trips() {
    for mode in [
        ThemeMode::Light,
        ThemeMode::Dark,
        ThemeMode::DarkDeep,
        ThemeMode::GitHubDark,
        ThemeMode::System,
    ] {
        assert_eq!(ThemeMode::from_preference(mode.preference()), mode);
    }
    assert_eq!(ThemeMode::from_preference("unknown"), ThemeMode::Light);
}

#[test]
fn new_themes_use_the_requested_backgrounds() {
    let deep = palette::for_mode(ThemeMode::DarkDeep).unwrap();
    let github = palette::for_mode(ThemeMode::GitHubDark).unwrap();
    assert_eq!(deep.background, Color32::from_rgb(24, 24, 24));
    assert!(deep.background.r() < viewer_visuals(true).panel_fill.r());
    assert_eq!(github.background, Color32::from_rgb(0x0d, 0x11, 0x17));
    assert_eq!(github.surface, Color32::from_rgb(0x16, 0x1b, 0x22));
    assert_eq!(github.border, Color32::from_rgb(0x30, 0x36, 0x3d));
    assert_eq!(github.accent, Color32::from_rgb(0x58, 0xa6, 0xff));
}

#[test]
fn native_captions_match_the_main_tab_strip_for_custom_themes() {
    let ctx = egui::Context::default();
    for mode in [ThemeMode::DarkDeep, ThemeMode::GitHubDark] {
        apply_viewer_visuals(&ctx, mode);
        let (caption, text) = native::title_bar_colors(&ctx).unwrap();
        let tab_strip = palette::background(&ctx, Color32::TRANSPARENT);
        assert_eq!(caption, [tab_strip.r(), tab_strip.g(), tab_strip.b()]);
        let foreground = text_color(&ctx);
        assert_eq!(text, [foreground.r(), foreground.g(), foreground.b()]);
    }
    apply_viewer_visuals(&ctx, ThemeMode::GitHubDark);
    assert_eq!(native::title_bar_colors(&ctx).unwrap().0, [13, 17, 23]);
}

#[test]
fn switching_back_to_legacy_themes_restores_native_caption_defaults() {
    let ctx = egui::Context::default();
    for mode in [ThemeMode::Light, ThemeMode::Dark, ThemeMode::System] {
        apply_viewer_visuals(&ctx, ThemeMode::GitHubDark);
        assert!(native::title_bar_colors(&ctx).is_some());
        apply_viewer_visuals(&ctx, mode);
        assert_eq!(native::title_bar_colors(&ctx), None);
    }
}

#[test]
fn new_palettes_apply_to_main_window_and_viewers() {
    let ctx = egui::Context::default();
    for mode in [ThemeMode::DarkDeep, ThemeMode::GitHubDark] {
        let p = palette::for_mode(mode).unwrap();
        for apply in [apply_theme_visuals, apply_viewer_visuals] {
            apply(&ctx, mode);
            let style = ctx.global_style();
            let v = &style.visuals;
            assert!(v.dark_mode);
            assert_eq!(current_mode(&ctx), mode);
            assert_eq!(v.panel_fill, p.background);
            assert_eq!(v.window_fill, p.surface);
            assert_eq!(v.extreme_bg_color, p.input);
            assert_eq!(v.widgets.hovered.bg_fill, p.hover);
            assert_eq!(v.widgets.active.bg_fill, p.active);
            assert_eq!(text_color(&ctx), p.text);
            assert_eq!(secondary_text_color(&ctx), p.secondary);
            assert_eq!(input_bg_color(&ctx), p.input);
            assert_eq!(selection_color(&ctx), p.selection);
            assert_eq!(selection_text_color(&ctx), p.selection_text);
            assert_eq!(selection_hover_color(&ctx), p.hover);
            assert_eq!(accent_color(&ctx), p.accent);
            assert_eq!(viewer_bar_colors(&ctx), (p.surface, p.border));
        }
    }
}

#[test]
fn switching_between_dark_palettes_updates_colors_without_leaking_between_contexts() {
    let first = egui::Context::default();
    let second = egui::Context::default();
    apply_theme_visuals(&first, ThemeMode::DarkDeep);
    apply_theme_visuals(&second, ThemeMode::GitHubDark);
    assert_ne!(text_color(&first), text_color(&second));
    apply_theme_visuals(&first, ThemeMode::Dark);
    assert_eq!(text_color(&first), Color32::from_gray(220));
    assert_eq!(current_mode(&second), ThemeMode::GitHubDark);
}

#[test]
fn custom_palettes_have_readable_text_contrast() {
    fn luminance(c: Color32) -> f32 {
        let channel = |v: u8| {
            let s = v as f32 / 255.0;
            if s <= 0.04045 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
    }
    for mode in [ThemeMode::DarkDeep, ThemeMode::GitHubDark] {
        let p = palette::for_mode(mode).unwrap();
        for bg in [
            p.background,
            p.surface,
            p.input,
            p.raised,
            p.hover,
            p.active,
        ] {
            for fg in [p.text, p.secondary] {
                assert!(
                    (luminance(fg) + 0.05) / (luminance(bg) + 0.05) >= 4.5,
                    "{mode:?}: {fg:?} on {bg:?}"
                );
            }
        }
        assert!((luminance(p.selection_text) + 0.05) / (luminance(p.selection) + 0.05) >= 4.5);
    }
}
