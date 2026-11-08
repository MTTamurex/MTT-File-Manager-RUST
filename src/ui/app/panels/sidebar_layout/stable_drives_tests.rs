use super::*;
use egui::{vec2, Context, Id, RawInput, Rect, Sense};

struct Geometry {
    drives_height: f32,
    tags: Rect,
    overflow: f32,
    offset: f32,
}

fn frame(
    ctx: &Context,
    content_height: f32,
    available_height: f32,
    saved_height: Option<f32>,
    collapsed: bool,
    scroll_offset: f32,
) -> Geometry {
    let mut geometry = Geometry {
        drives_height: 0.0,
        tags: Rect::NOTHING,
        overflow: 0.0,
        offset: 0.0,
    };
    let _ = ctx.run_ui(RawInput::default(), |ui| {
        ui.set_max_size(vec2(260.0, available_height));
        ui.set_clip_rect(ui.max_rect());
        let available =
            drives_available_height(ui.available_height(), true, ui.spacing().item_spacing.y);
        let drives = render_stable_drives_block(
            ui,
            saved_height.unwrap_or(available).min(available),
            saved_height.is_some(),
            scroll_offset,
            Id::new(("automatic_drives_test", collapsed)),
            |ui| {
                ui.allocate_exact_size(vec2(240.0, content_height), Sense::hover());
            },
        );
        geometry.drives_height = drives.response.rect.height();
        geometry.overflow = drives.inner.content_size.y - drives.inner.inner_rect.height();
        geometry.offset = drives.inner.state.offset.y;
        let tags = render_tags_block(
            ui,
            TagsBlockLayout {
                collapsed: false,
                drives_height: geometry.drives_height,
                drives_min_height: 24.0,
                drives_max_height: available,
            },
            |ui| {
                ui.allocate_exact_size(vec2(240.0, 600.0), Sense::hover());
            },
        );
        geometry.tags = tags.interaction_rect;
    });
    geometry
}

#[test]
fn expanding_folders_preserves_tags_and_scrolls_drives() {
    let ctx = Context::default();
    let before = frame(&ctx, 160.0, 500.0, None, false, 0.0);
    for height in [300.0, 800.0, 1200.0] {
        let expanded = frame(&ctx, height, 500.0, None, false, 60.0);
        assert_eq!(expanded.drives_height, before.drives_height);
        assert_eq!(expanded.tags, before.tags);
        assert!(expanded.overflow > 0.0);
        assert!(expanded.offset > 0.0);
    }
    let restored = frame(&ctx, 160.0, 500.0, None, false, 60.0);
    assert_eq!(restored.tags, before.tags);
    assert_eq!(restored.offset, 0.0);
}

#[test]
fn section_collapse_and_manual_divider_keep_independent_sizes() {
    let ctx = Context::default();
    let before = frame(&ctx, 160.0, 500.0, None, false, 0.0);
    let collapsed = frame(&ctx, 48.0, 500.0, None, true, 0.0);
    assert!(collapsed.tags.top() < before.tags.top());
    let restored = frame(&ctx, 800.0, 500.0, None, false, 0.0);
    assert_eq!(restored.tags, before.tags);
    let resized = frame(&ctx, 800.0, 500.0, Some(280.0), false, 0.0);
    assert_eq!(resized.drives_height, 280.0);
    assert!(resized.tags.top() > restored.tags.top());
}

#[test]
fn short_windows_do_not_replace_the_automatic_content_height() {
    let ctx = Context::default();
    let short = frame(&ctx, 160.0, 100.0, None, false, 0.0);
    assert!(short.drives_height < 160.0);
    let tall = frame(&ctx, 160.0, 500.0, None, false, 0.0);
    assert_eq!(tall.drives_height, 160.0);
    let expanded = frame(&ctx, 800.0, 500.0, None, false, 0.0);
    assert_eq!(expanded.tags, tall.tags);
}

#[test]
fn resetting_the_divider_recalculates_automatic_height() {
    let ctx = Context::default();
    let before = frame(&ctx, 160.0, 500.0, None, false, 0.0);
    let expanded = frame(&ctx, 300.0, 500.0, None, false, 0.0);
    assert_eq!(expanded.tags, before.tags);
    ctx.data_mut(|data| data.remove::<f32>(Id::new(("automatic_drives_test", false))));
    let reset = frame(&ctx, 300.0, 500.0, None, false, 0.0);
    assert_eq!(reset.drives_height, 300.0);
    let expanded_again = frame(&ctx, 800.0, 500.0, None, false, 0.0);
    assert_eq!(expanded_again.tags, reset.tags);
}
