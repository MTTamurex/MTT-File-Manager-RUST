//! Content-sized sidebar sections retain independent scrolling and resize limits.

use eframe::egui;

mod tags;
pub(super) use tags::{render_tags_block, TagsBlockLayout};

pub(super) const QUICK_ACCESS_MIN_HEIGHT: f32 = 8.0 + 16.0;
pub(super) const QUICK_ACCESS_DIVIDER_HEIGHT: f32 = 9.0;
pub(super) const TAGS_MIN_HEIGHT: f32 = 22.0 + 8.0;
pub(super) const TAGS_DIVIDER_HEIGHT: f32 = 9.0;

/// Automatic layouts follow content, but an explicitly dragged divider must be
/// allowed to move beyond it so Tags can shrink independently of drive count.
pub(super) fn render_drives_block<R>(
    ui: &mut egui::Ui,
    height: f32,
    manually_sized: bool,
    scroll_offset: f32,
    render: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<egui::scroll_area::ScrollAreaOutput<R>> {
    ui.scope(|ui| {
        ui.spacing_mut().scroll.fade.strength = 0.0;
        egui::ScrollArea::both()
            .id_salt("sidebar_scroll")
            .auto_shrink([false, !manually_sized])
            .max_height(height)
            .min_scrolled_height(0.0)
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
            .vertical_scroll_offset(scroll_offset)
            .show(ui, render)
    })
}

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

pub(super) fn drives_available_height(available: f32, tags_visible: bool, spacing: f32) -> f32 {
    let tags_header = if tags_visible {
        TAGS_MIN_HEIGHT + TAGS_DIVIDER_HEIGHT + 3.0 * spacing
    } else {
        0.0
    };
    (available - tags_header).max(0.0)
}

/// Remember separate divider limits so resizing a collapsed layout does not
/// overwrite the size used when those sections are expanded again.
pub(super) fn restore_drives_height(
    previous: [bool; 4],
    current: [bool; 4],
    height_limit: &mut Option<f32>,
    remembered: &mut std::collections::HashMap<[bool; 4], Option<f32>>,
) {
    if previous != current {
        remembered.insert(previous, *height_limit);
        *height_limit = remembered.get(&current).copied().flatten();
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
    use super::{
        drives_available_height, quick_access_content_height, restore_drives_height, section_height,
    };
    use eframe::egui::{self, pos2, vec2, Context, Rect, Sense};

    #[test]
    fn collapsed_headers_do_not_depend_on_item_count() {
        assert_eq!(quick_access_content_height(true, 0, false, 3.0), 24.0);
        assert_eq!(quick_access_content_height(true, 20, true, 3.0), 24.0);
        assert!(quick_access_content_height(false, 20, true, 3.0) > 24.0);
    }

    #[test]
    fn expanding_a_section_restores_its_previous_height() {
        let expanded = [false; 4];
        let collapsed = [true; 4];
        let mut limit = Some(180.0);
        let mut remembered = Default::default();
        restore_drives_height(expanded, collapsed, &mut limit, &mut remembered);
        assert_eq!(limit, None);
        limit = Some(60.0);
        restore_drives_height(collapsed, expanded, &mut limit, &mut remembered);
        assert_eq!(limit, Some(180.0));
        assert_eq!(section_height(300.0, limit, 24.0, 400.0), 180.0);
        restore_drives_height(expanded, collapsed, &mut limit, &mut remembered);
        assert_eq!(limit, Some(60.0));
    }

    #[test]
    fn reopening_sections_also_remembers_automatic_sizing() {
        let mut remembered = Default::default();
        let mut limit = None;
        let expanded = [false; 4];
        let partial = [true, false, false, false];
        let collapsed = [true; 4];
        restore_drives_height(expanded, partial, &mut limit, &mut remembered);
        limit = Some(140.0);
        restore_drives_height(partial, collapsed, &mut limit, &mut remembered);
        limit = Some(60.0);
        restore_drives_height(collapsed, partial, &mut limit, &mut remembered);
        assert_eq!(limit, Some(140.0));
        restore_drives_height(partial, expanded, &mut limit, &mut remembered);
        assert_eq!(limit, None);
    }

    #[test]
    fn drives_reserve_only_the_tags_header() {
        assert_eq!(drives_available_height(500.0, true, 3.0), 452.0);
        assert_eq!(drives_available_height(500.0, false, 3.0), 500.0);
        assert_eq!(drives_available_height(10.0, true, 3.0), 0.0);
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
