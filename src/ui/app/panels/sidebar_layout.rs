//! Content-sized sidebar sections retain independent scrolling and resize limits.

mod tags;
pub(super) use tags::{render_tags_block, TagsBlockLayout};

pub(super) const QUICK_ACCESS_MIN_HEIGHT: f32 = 8.0 + 16.0;
pub(super) const QUICK_ACCESS_DIVIDER_HEIGHT: f32 = 9.0;
pub(super) const TAGS_MIN_HEIGHT: f32 = 22.0 + 8.0;
pub(super) const TAGS_DIVIDER_HEIGHT: f32 = 9.0;

pub(super) fn quick_access_content_height(
    collapsed: bool,
    pinned_count: usize,
    show_recycle_bin: bool,
    spacing: f32,
) -> f32 {
    if collapsed {
        return QUICK_ACCESS_MIN_HEIGHT;
    }
    QUICK_ACCESS_MIN_HEIGHT
        + 4.0
        + if show_recycle_bin { 28.0 } else { 0.0 }
        + pinned_count as f32 * 30.0
        + 8.0
        + (1 + usize::from(show_recycle_bin) + pinned_count) as f32 * spacing
}

pub(super) fn tags_content_height(collapsed: bool, tag_count: usize, spacing: f32) -> f32 {
    if collapsed {
        TAGS_MIN_HEIGHT + spacing
    } else {
        TAGS_MIN_HEIGHT + 4.0 + tag_count as f32 * 26.0 + (1 + tag_count) as f32 * spacing
    }
}

/// A saved divider height is a cap, not a reservation of empty space.
pub(super) fn section_height(content: f32, saved: Option<f32>, min: f32, max: f32) -> f32 {
    let max = max.max(0.0);
    let min = min.min(max).max(0.0);
    content
        .max(0.0)
        .min(saved.unwrap_or(max).clamp(min, max))
        .min(max)
}

#[cfg(test)]
mod tests {
    use super::{quick_access_content_height, section_height, tags_content_height};
    use eframe::egui::{self, pos2, vec2, Context, Rect, Sense};

    #[test]
    fn collapsed_headers_do_not_depend_on_item_count() {
        assert_eq!(quick_access_content_height(true, 0, false, 3.0), 24.0);
        assert_eq!(quick_access_content_height(true, 20, true, 3.0), 24.0);
        assert_eq!(tags_content_height(true, 1, 3.0), 33.0);
        assert_eq!(tags_content_height(true, 20, 3.0), 33.0);
        assert!(quick_access_content_height(false, 20, true, 3.0) > 24.0);
        assert!(tags_content_height(false, 20, 3.0) > 33.0);
    }

    #[test]
    fn saved_height_does_not_reserve_space_after_collapse() {
        assert_eq!(section_height(300.0, Some(180.0), 24.0, 400.0), 180.0);
        assert_eq!(section_height(24.0, Some(180.0), 24.0, 400.0), 24.0);
        assert_eq!(section_height(80.0, Some(180.0), 24.0, 400.0), 80.0);
        assert_eq!(section_height(300.0, Some(180.0), 24.0, 400.0), 180.0);
    }

    #[test]
    fn sections_fit_short_windows_and_hidden_content() {
        assert_eq!(section_height(300.0, Some(180.0), 24.0, 10.0), 10.0);
        assert_eq!(section_height(300.0, None, 24.0, 0.0), 0.0);
        assert_eq!(section_height(0.0, Some(180.0), 24.0, 400.0), 0.0);
    }

    #[test]
    fn following_sections_move_up_and_scroll_offset_clamps_after_collapse() {
        let ctx = Context::default();
        let render = |height: f32, offset: f32| {
            let mut next_top = 0.0;
            let mut actual_offset = 0.0;
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.set_max_size(vec2(220.0, 500.0));
                let output = egui::ScrollArea::both()
                    .id_salt("dynamic_sidebar_test")
                    .auto_shrink([false, true])
                    .max_height(300.0)
                    .min_scrolled_height(0.0)
                    .vertical_scroll_offset(offset)
                    .show(ui, |ui| {
                        ui.allocate_exact_size(vec2(200.0, height), Sense::hover());
                    });
                actual_offset = output.state.offset.y;
                next_top = ui.cursor().top();
                ui.allocate_rect(
                    Rect::from_min_size(pos2(0.0, next_top), vec2(200.0, 22.0)),
                    Sense::hover(),
                );
            });
            (next_top, actual_offset)
        };
        let expanded = render(600.0, 250.0);
        let collapsed = render(24.0, expanded.1);
        assert!(expanded.0 - collapsed.0 > 250.0);
        assert_eq!(collapsed.1, 0.0);
    }
}
