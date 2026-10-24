//! Virtualized column layout, including group headers and scroll geometry.

use eframe::egui::{self, Color32, FontId, Rect, Sense, Ui};

use super::column::{background_action, render_row, MillerColumnContext, MillerColumnOutput};
use super::grouping::ColumnGroups;
use super::layout::COL_ROW_HEIGHT;
use crate::ui::theme;
use crate::ui::views::group_header::{render_group_header, GROUP_HEADER_HEIGHT};
use crate::ui::views::rectangle_selection::{
    paint_overlay, ListRectangleMetrics, RectangleSelectionFrame, RectangleSelectionMetrics,
    RectangleSelectionSource,
};

pub fn render_miller_column(
    ui: &mut Ui,
    id_salt: impl egui::AsIdSalt,
    ctx: &mut MillerColumnContext,
) -> MillerColumnOutput {
    let dark = ui.visuals().dark_mode;
    let rect = ui.max_rect();
    let row_height = crate::ui::fonts::layout::row_height(ui, COL_ROW_HEIGHT, 13.0, 6.0);
    let column_id = ui.make_persistent_id(id_salt);
    let background_response =
        ui.interact(rect, column_id.with("background"), Sense::click_and_drag());
    let pointer_pos = ctx
        .external_drop_pointer_pos
        .or_else(|| ui.input(|i| i.pointer.hover_pos()));
    if ctx.is_item_dragging && pointer_pos.is_some_and(|position| rect.contains(position)) {
        *ctx.drop_target = Some(ctx.directory.to_path_buf());
    }
    let sep_color = if dark {
        theme::palette::border(ui.ctx(), Color32::from_gray(60))
    } else {
        Color32::from_gray(210)
    };
    ui.painter().vline(
        rect.right(),
        rect.top()..=rect.bottom(),
        egui::Stroke::new(1.0, sep_color),
    );
    if ctx.items.is_empty() {
        if ctx.is_loading {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "…",
                FontId::proportional(13.0),
                Color32::from_gray(120),
            );
        }
        return MillerColumnOutput {
            action: background_action(&background_response, ui),
            toggled_group: None,
            rectangle_selection_frame: RectangleSelectionFrame::default(),
            rename_update: None,
        };
    }

    let groups = ctx
        .group_projection
        .is_grouped()
        .then(|| ColumnGroups::new(ctx.group_projection, ctx.collapsed_groups, row_height));
    let content_height = groups
        .as_ref()
        .map_or(ctx.items.len() as f32 * row_height, |groups| {
            groups.content_height
        });
    let scroll_id = ui.make_persistent_id(column_id);
    let custom_wheel_offset = if !ctx.scroll_interpolation_enabled
        && ui.rect_contains_pointer(rect)
        && ui.ctx().dragged_id().is_none()
    {
        let delta = ui
            .input(|i| crate::ui::views::common::raw_wheel_delta(i, rect.height()))
            .y;
        let current =
            egui::scroll_area::State::load(ui.ctx(), scroll_id).map_or(0.0, |state| state.offset.y);
        raw_wheel_scroll_offset(current, delta, (content_height - rect.height()).max(0.0))
    } else {
        None
    };
    if custom_wheel_offset.is_some() {
        // Do not let the enclosing horizontal strip consume this vertical wheel.
        ui.ctx()
            .input_mut(|input| input.smooth_scroll_delta.y = 0.0);
    }
    let scroll_source = if ctx.scroll_interpolation_enabled {
        egui::scroll_area::ScrollSource::SCROLL_BAR | egui::scroll_area::ScrollSource::MOUSE_WHEEL
    } else {
        egui::scroll_area::ScrollSource::SCROLL_BAR
    };
    let scroll_area = egui::ScrollArea::vertical()
        .id_salt(column_id)
        .auto_shrink([false, false])
        .scroll_source(scroll_source)
        .animated(ctx.scroll_interpolation_enabled);
    let scroll_area = if let Some(offset) = custom_wheel_offset {
        scroll_area.vertical_scroll_offset(offset)
    } else {
        scroll_area
    };
    let mut action = None;
    let mut toggled_group = None;
    let mut rename_update = None;
    let mut rectangle_start = None;
    let item_count = ctx.items.len();
    ui.spacing_mut().item_spacing.y = 0.0;
    let scroll_output = scroll_area.show_viewport(ui, |ui, viewport| {
        ui.set_height(content_height);
        let origin = ui.max_rect().min;
        let width = ui.available_width();
        let mut draw_row = |ui: &mut Ui, index: usize, y: f32| {
            let Some(item) = ctx.items.get(index) else {
                return;
            };
            let row_rect =
                Rect::from_min_size(origin + egui::vec2(0.0, y), egui::vec2(width, row_height));
            ui.scope_builder(
                egui::UiBuilder::new()
                    .id_salt((column_id, index))
                    .max_rect(row_rect),
                |ui| {
                    if let Some(a) = render_row(
                        ui,
                        index,
                        item,
                        ctx,
                        dark,
                        &mut rectangle_start,
                        &mut rename_update,
                    ) {
                        action = Some(a);
                    }
                },
            );
        };
        if let Some(groups) = &groups {
            for section in &groups.sections {
                let header = Rect::from_min_size(
                    origin + egui::vec2(0.0, section.header_y),
                    egui::vec2(width, GROUP_HEADER_HEIGHT),
                );
                if ui.is_rect_visible(header)
                    && render_group_header(
                        ui,
                        header,
                        &section.key,
                        section.item_indices.len(),
                        section.collapsed,
                    )
                    .clicked()
                {
                    toggled_group = Some(section.key.clone());
                }
                if section.collapsed {
                    continue;
                }
                let top = section.header_y + GROUP_HEADER_HEIGHT;
                let first = ((viewport.min.y - top) / row_height).floor().max(0.0) as usize;
                let last = ((viewport.max.y - top) / row_height).ceil().max(0.0) as usize;
                for position in first..last.min(section.item_indices.len()) {
                    draw_row(
                        ui,
                        section.item_indices[position],
                        top + position as f32 * row_height,
                    );
                }
            }
        } else {
            let first = (viewport.min.y / row_height).floor().max(0.0) as usize;
            let last = (viewport.max.y / row_height).ceil().max(0.0) as usize;
            for index in first..last.min(item_count) {
                draw_row(ui, index, index as f32 * row_height);
            }
        }
    });
    if action.is_none()
        && toggled_group.is_none()
        && background_response.drag_started_by(egui::PointerButton::Primary)
    {
        rectangle_start = ui.input(|input| input.pointer.press_origin());
    }
    let current_scroll_y = scroll_output.state.offset.y;
    let max_scroll_y = (scroll_output.content_size.y - scroll_output.inner_rect.height()).max(0.0);
    let metrics = groups.as_ref().map_or_else(
        || {
            RectangleSelectionMetrics::List(ListRectangleMetrics {
                count: ctx.items.len(),
                row_height,
                content_width: scroll_output.inner_rect.width(),
                content_height,
            })
        },
        |groups| {
            groups.rectangle_metrics(
                ctx.items.len(),
                row_height,
                scroll_output.inner_rect.width(),
            )
        },
    );
    let mut rectangle_selection_frame = RectangleSelectionFrame::default();
    rectangle_selection_frame.begin(
        scroll_output.inner_rect,
        0.0,
        current_scroll_y,
        0.0,
        max_scroll_y,
        Some(metrics),
    );
    rectangle_selection_frame.source = RectangleSelectionSource::MillerAncestor {
        directory: ctx.directory.to_path_buf(),
    };
    if let Some(start) = rectangle_start {
        rectangle_selection_frame.request_start(start);
    }
    if let Some(state) = ctx.rectangle_selection_state {
        paint_overlay(ui, state, scroll_output.inner_rect, 0.0, current_scroll_y);
    }
    MillerColumnOutput {
        action: action.or_else(|| {
            if toggled_group.is_none() {
                background_action(&background_response, ui)
            } else {
                None
            }
        }),
        toggled_group,
        rectangle_selection_frame,
        rename_update,
    }
}

fn raw_wheel_scroll_offset(current_offset: f32, wheel_delta: f32, max_offset: f32) -> Option<f32> {
    let next_offset = (current_offset - wheel_delta).clamp(0.0, max_offset);
    (wheel_delta != 0.0 && next_offset != current_offset).then_some(next_offset)
}

#[cfg(test)]
mod tests {
    use super::raw_wheel_scroll_offset;

    #[test]
    fn raw_wheel_scroll_changes_only_the_column_offset() {
        assert_eq!(raw_wheel_scroll_offset(20.0, -15.0, 100.0), Some(35.0));
        assert_eq!(raw_wheel_scroll_offset(20.0, 15.0, 100.0), Some(5.0));
        assert_eq!(raw_wheel_scroll_offset(0.0, 15.0, 100.0), None);
        assert_eq!(raw_wheel_scroll_offset(95.0, -15.0, 100.0), Some(100.0));
        assert_eq!(raw_wheel_scroll_offset(20.0, 0.0, 100.0), None);
    }
}
