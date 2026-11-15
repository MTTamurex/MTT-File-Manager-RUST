use eframe::egui;

pub(super) const NEW_TAB_WIDTH: f32 = 36.0;
pub(super) const WINDOW_BUTTON_WIDTH: f32 = 46.0;

pub(super) struct StripLayout {
    pub tabs: egui::Rect,
    pub new_tab_and_drag: egui::Rect,
    pub controls: egui::Rect,
    pub tab_width: f32,
}

impl StripLayout {
    pub fn new(rect: egui::Rect, count: usize) -> Self {
        let controls_left = (rect.right() - WINDOW_BUTTON_WIDTH * 3.0).max(rect.left());
        let available = (controls_left - rect.left() - NEW_TAB_WIDTH).max(0.0);
        let tab_width = (available / count.max(1) as f32).clamp(100.0, 200.0);
        let tabs_right = rect.left() + (tab_width * count as f32).min(available);
        Self {
            tabs: egui::Rect::from_min_max(rect.min, egui::pos2(tabs_right, rect.bottom())),
            new_tab_and_drag: egui::Rect::from_min_max(
                egui::pos2(tabs_right, rect.top()),
                egui::pos2(controls_left, rect.bottom()),
            ),
            controls: egui::Rect::from_min_max(egui::pos2(controls_left, rect.top()), rect.max),
            tab_width,
        }
    }
}

pub(super) fn show<R>(
    ui: &mut egui::Ui,
    layout: &StripLayout,
    active_id: usize,
    active_index: usize,
    count: usize,
    render: impl FnOnce(&mut egui::Ui, bool) -> R,
) -> R {
    let state_id = ui.id().with("main_tab_active_scroll");
    let signature = (
        active_id,
        active_index,
        count,
        layout.tabs.width().to_bits(),
    );
    let reveal = ui.ctx().data_mut(|data| {
        let previous = data.get_temp::<(usize, usize, usize, u32)>(state_id);
        data.insert_temp(state_id, signature);
        previous != Some(signature)
    });
    ui.scope_builder(egui::UiBuilder::new().max_rect(layout.tabs), |ui| {
        ui.shrink_clip_rect(layout.tabs);
        egui::ScrollArea::horizontal()
            .id_salt("main_tab_strip")
            .scroll_source(
                egui::scroll_area::ScrollSource::SCROLL_BAR
                    | egui::scroll_area::ScrollSource::MOUSE_WHEEL,
            )
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
            .show(ui, |ui| {
                ui.set_min_height(layout.tabs.height());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    render(ui, reveal)
                })
                .inner
            })
            .inner
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overflowing_tabs_leave_fixed_controls_and_add_button_visible() {
        for width in [320.0, 800.0, 1920.0] {
            for count in [1, 2, 20, 100] {
                let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(width, 36.0));
                let layout = StripLayout::new(rect, count);
                assert_eq!(layout.controls.right(), rect.right());
                assert_eq!(layout.controls.width(), WINDOW_BUTTON_WIDTH * 3.0);
                assert!(layout.new_tab_and_drag.width() >= NEW_TAB_WIDTH);
                assert_eq!(layout.tabs.right(), layout.new_tab_and_drag.left());
                assert_eq!(layout.new_tab_and_drag.right(), layout.controls.left());
                assert!((100.0..=200.0).contains(&layout.tab_width));
            }
        }
    }

    #[test]
    fn short_tab_list_leaves_caption_drag_space() {
        let layout = StripLayout::new(
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 36.0)),
            2,
        );
        assert_eq!(layout.tabs.width(), 400.0);
        assert!(layout.new_tab_and_drag.width() > NEW_TAB_WIDTH);
    }

    #[test]
    fn extremely_narrow_layout_has_no_negative_regions() {
        for width in [0.0, 100.0, 150.0] {
            let layout = StripLayout::new(
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 36.0)),
                20,
            );
            assert!(layout.tabs.width() >= 0.0);
            assert!(layout.new_tab_and_drag.width() >= 0.0);
            assert!(layout.controls.width() >= 0.0);
        }
    }
}
