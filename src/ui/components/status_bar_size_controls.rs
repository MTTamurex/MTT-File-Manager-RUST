use crate::domain::file_entry::ViewMode;
use crate::ui::theme;
use eframe::egui;
use rust_i18n::t;

const BUTTON_WIDTH: f32 = 28.0;
const BUTTON_GAP: f32 = 4.0;
pub(crate) const RESERVED_WIDTH: f32 = BUTTON_WIDTH + BUTTON_GAP;

pub(crate) fn compact_width(_ui: &egui::Ui) -> f32 {
    // Space for the remaining status controls and the centered selection group.
    560.0
}

/// Reserve the rightmost slot before placing the minimized operations indicator.
pub(crate) fn rects(full: egui::Rect, row_height: f32) -> (egui::Rect, egui::Rect) {
    let left = (full.right() - BUTTON_WIDTH).max(full.left());
    (
        egui::Rect::from_min_max(
            full.min,
            egui::pos2((left - BUTTON_GAP).max(full.left()), full.bottom()),
        ),
        egui::Rect::from_min_max(
            egui::pos2(left, (full.bottom() - row_height).max(full.top())),
            full.max,
        ),
    )
}

pub(crate) fn render(
    ui: &mut egui::Ui,
    view_mode: ViewMode,
    thumbnail_size: &mut f32,
    interface_scale: &mut f32,
) -> egui::Response {
    let response = ui.add_sized(
        [BUTTON_WIDTH, ui.available_height()],
        egui::Button::new("").frame(false),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tooltip()));
    paint_icon(ui, &response);
    let popup_id = response.id.with("size_popup");
    let hover_id = response.id.with("last_hover");
    let hovered_id = response.id.with("was_hovered");
    let drag_id = response.id.with("dragging");
    let ctx = ui.ctx().clone();
    let now = ctx.input(|i| i.time);
    let was_hovered = ctx.data(|data| data.get_temp::<bool>(hovered_id).unwrap_or(false));
    if (response.hovered() && !was_hovered) || response.clicked() {
        egui::Popup::open_id(&ctx, popup_id);
    }
    ctx.data_mut(|data| data.insert_temp(hovered_id, response.hovered()));
    if response.hovered() {
        ctx.data_mut(|data| data.insert_temp(hover_id, now));
    }
    let popup = egui::Popup::from_response(&response)
        .id(popup_id)
        .open_memory(None)
        .align(egui::RectAlign::TOP_END)
        .gap(2.0)
        .width(320.0)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    let over_popup = popup.get_popup_rect().is_some_and(|rect| {
        ctx.pointer_hover_pos()
            .is_some_and(|pos| rect.expand(4.0).contains(pos))
    });
    let dragging = ctx.data(|data| data.get_temp::<bool>(drag_id).unwrap_or(false));
    if over_popup || dragging {
        ctx.data_mut(|data| data.insert_temp(hover_id, now));
    }
    let last_hover = ctx.data(|data| data.get_temp::<f64>(hover_id).unwrap_or(now));
    if !response.hovered() && !over_popup && !dragging && now - last_hover > 0.2 {
        egui::Popup::close_id(&ctx, popup_id);
    }
    if let Some(inner) =
        popup.show(|ui| render_popup(ui, view_mode, thumbnail_size, interface_scale))
    {
        let dragging = inner
            .inner
            .iter()
            .any(egui::Response::is_pointer_button_down_on);
        ctx.data_mut(|data| data.insert_temp(drag_id, dragging));
        // Allow crossing the small gap without closing the popup.
        ctx.request_repaint_after(std::time::Duration::from_millis(210));
    } else {
        ctx.data_mut(|data| data.remove::<bool>(drag_id));
    }
    response.on_hover_text(tooltip())
}

fn tooltip() -> String {
    format!(
        "{} / {}",
        t!("status_bar.thumbnail_size"),
        t!("settings.interface_scale")
    )
}

fn paint_icon(ui: &egui::Ui, response: &egui::Response) {
    let center = response.rect.center();
    let color = if response.hovered() {
        theme::accent_color(ui.ctx())
    } else {
        theme::text_color(ui.ctx())
    };
    let painter = ui.painter();
    for (x, y) in [(-6.0, -3.0), (0.0, 3.0), (6.0, -1.0)] {
        let x = center.x + x;
        painter.line_segment(
            [egui::pos2(x, center.y - 8.0), egui::pos2(x, center.y + 8.0)],
            egui::Stroke::new(1.5, color),
        );
        painter.circle_filled(egui::pos2(x, center.y + y), 2.5, color);
    }
}

fn render_popup(
    ui: &mut egui::Ui,
    view_mode: ViewMode,
    thumbnail_size: &mut f32,
    interface_scale: &mut f32,
) -> [egui::Response; 2] {
    ui.visuals_mut().selection.bg_fill = theme::accent_color(ui.ctx());
    ui.spacing_mut().slider_width = 150.0;
    egui::Grid::new("size_sliders")
        .num_columns(2)
        .min_col_width(0.0)
        .min_row_height(28.0)
        .spacing(egui::vec2(16.0, 8.0))
        .show(ui, |ui| {
            ui.add(egui::Label::new(t!("settings.interface_scale").to_string()).extend());
            let scale_response = render_scale_slider(ui, interface_scale);
            ui.end_row();
            ui.add(egui::Label::new(t!("status_bar.thumbnail_size").to_string()).extend());
            let enabled = !matches!(view_mode, ViewMode::List | ViewMode::ColumnList);
            let thumbnail_response = ui.add_enabled(
                enabled,
                egui::Slider::new(thumbnail_size, theme::THUMBNAIL_MIN..=256.0)
                    .show_value(false)
                    .trailing_fill(true),
            );
            show_value_tooltip(&thumbnail_response, format!("{thumbnail_size:.0} px"));
            ui.end_row();
            [scale_response, thumbnail_response]
        })
        .inner
}

fn show_value_tooltip(response: &egui::Response, value: String) {
    if response.hovered() || response.is_pointer_button_down_on() {
        let mut tooltip = egui::Tooltip::for_widget(response);
        tooltip.popup = tooltip.popup.align(egui::RectAlign::TOP);
        tooltip.show(|ui| {
            ui.label(value);
        });
    }
}

fn render_scale_slider(ui: &mut egui::Ui, interface_scale: &mut f32) -> egui::Response {
    let draft_id = ui.make_persistent_id("interface_scale_percent");
    let draft = ui.data(|data| data.get_temp::<f32>(draft_id));
    let mut percent = draft.unwrap_or(*interface_scale * 100.0);
    let response = ui.add(
        egui::Slider::new(
            &mut percent,
            100.0..=crate::ui::interface_scale::MAX * 100.0,
        )
        .step_by(5.0)
        .show_value(false)
        .trailing_fill(true),
    );
    if response.is_pointer_button_down_on() {
        // Changing scale while dragging would move the status bar under the pointer.
        ui.data_mut(|data| data.insert_temp(draft_id, percent));
    } else {
        ui.data_mut(|data| data.remove::<f32>(draft_id));
        if response.changed() || draft.is_some() {
            *interface_scale = percent / 100.0;
        }
    }
    show_value_tooltip(&response, format!("{percent:.0}%"));
    response
}

#[cfg(test)]
mod tests;
