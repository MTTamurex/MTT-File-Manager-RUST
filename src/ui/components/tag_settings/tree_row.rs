use crate::ui::theme;
use eframe::egui;
use rust_i18n::t;

pub(super) struct Row<'a> {
    pub name: &'a str,
    pub color: egui::Color32,
    pub depth: usize,
    pub has_children: bool,
    pub expanded: bool,
    pub selected: bool,
    pub searching: bool,
}

pub(super) struct Output {
    pub response: egui::Response,
    pub toggle: bool,
    #[cfg(test)]
    icon_left: f32,
    #[cfg(test)]
    text_left: f32,
}

pub(super) fn show(ui: &mut egui::Ui, row: Row<'_>) -> Output {
    let height = (ui.text_style_height(&egui::TextStyle::Button) + 14.0).max(32.0);
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
    let indent = (row.depth as f32 * 14.0).min(rect.width() * 0.25);
    // Every row reserves exactly the same disclosure lane, including leaves.
    // Only actual hierarchy depth can move the icon and label to the right.
    let arrow = egui::Rect::from_min_size(
        rect.min + egui::vec2(8.0 + indent, 0.0),
        egui::vec2(20.0, height),
    );
    let select_rect = egui::Rect::from_min_max(egui::pos2(arrow.right(), rect.top()), rect.max);
    let response = ui
        .interact(select_rect, ui.id().with("select"), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            row.selected,
            row.name,
        )
    });
    let visuals = ui.style().interact_selectable(&response, row.selected);
    if ui.is_rect_visible(rect) {
        if row.selected || response.hovered() || response.has_focus() {
            ui.painter().rect_filled(rect, 6.0, visuals.bg_fill);
        }
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect,
                6.0,
                egui::Stroke::new(1.0, theme::accent_color(ui.ctx())),
                egui::StrokeKind::Inside,
            );
        }
    }
    let toggle = if row.has_children {
        let label = if row.expanded {
            t!("tags.collapse_children")
        } else {
            t!("tags.expand_children")
        };
        let arrow_response = ui
            .interact(
                arrow,
                ui.id().with("expand"),
                if row.searching {
                    egui::Sense::hover()
                } else {
                    egui::Sense::click()
                },
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(label.clone());
        arrow_response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                ui.is_enabled() && !row.searching,
                label.as_ref(),
            )
        });
        if ui.is_rect_visible(rect) {
            if arrow_response.has_focus() {
                ui.painter().rect_stroke(
                    arrow.shrink(2.0),
                    4.0,
                    egui::Stroke::new(1.0, theme::accent_color(ui.ctx())),
                    egui::StrokeKind::Inside,
                );
            }
            crate::ui::disclosure::paint(
                ui.painter(),
                arrow.center(),
                row.expanded,
                crate::ui::disclosure::SECTION_SIZE,
                visuals.text_color(),
            );
        }
        arrow_response.clicked()
    } else {
        false
    };
    let icon_left = arrow.right() + 4.0;
    let text_left = icon_left + 14.0 + 10.0;
    if ui.is_rect_visible(rect) {
        crate::ui::tag_icon::paint_filled(
            ui.painter(),
            egui::pos2(icon_left + 7.0, rect.center().y),
            12.0,
            row.color,
        );
        let mut job = egui::text::LayoutJob::simple(
            row.name.to_owned(),
            egui::TextStyle::Button.resolve(ui.style()),
            visuals.text_color(),
            (rect.right() - text_left - 10.0).max(0.0),
        );
        job.wrap.max_rows = 1;
        let galley = ui.painter().layout_job(job);
        ui.painter()
            .with_clip_rect(rect.intersect(ui.clip_rect()))
            .galley(
                egui::pos2(text_left, rect.center().y - galley.size().y / 2.0),
                galley,
                visuals.text_color(),
            );
    }
    Output {
        response,
        toggle,
        #[cfg(test)]
        icon_left,
        #[cfg(test)]
        text_left,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disclosure_triangle_has_a_visible_size_in_both_directions() {
        for expanded in [false, true] {
            let points = crate::ui::disclosure::points(
                egui::pos2(20.0, 20.0),
                expanded,
                crate::ui::disclosure::SECTION_SIZE,
            );
            let bounds = egui::Rect::from_points(&points);
            let size = crate::ui::disclosure::SECTION_SIZE;
            assert!(bounds.width() >= size * 0.9 - 0.01);
            assert!(bounds.height() >= size * 0.9 - 0.01);
            assert!(bounds.width() <= size + 0.01);
            assert!(bounds.height() <= size + 0.01);
        }
    }

    #[test]
    fn clicking_disclosure_expands_without_selecting_the_tag() {
        let ctx = egui::Context::default();
        let mut fonts = egui::FontDefinitions::default();
        fonts.families.insert(
            egui::FontFamily::Name("icons".into()),
            fonts.families[&egui::FontFamily::Proportional].clone(),
        );
        ctx.set_fonts(fonts);
        let mut point = egui::Pos2::ZERO;
        for frame in 0..4 {
            let events = if frame >= 2 {
                vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed: frame == 2,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]
            } else {
                Vec::new()
            };
            let _ = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    ui.set_width(300.0);
                    let output = show(
                        ui,
                        Row {
                            name: "Category",
                            color: egui::Color32::BLUE,
                            depth: 0,
                            has_children: true,
                            expanded: false,
                            selected: false,
                            searching: false,
                        },
                    );
                    point = egui::pos2(
                        output.response.rect.left() - 10.0,
                        output.response.rect.center().y,
                    );
                    assert!(!output.response.clicked());
                    assert_eq!(output.toggle, frame == 3);
                },
            );
        }
    }

    #[test]
    fn roots_with_and_without_children_have_identical_icon_and_label_columns() {
        let ctx = egui::Context::default();
        let mut fonts = egui::FontDefinitions::default();
        fonts.families.insert(
            egui::FontFamily::Name("icons".into()),
            fonts.families[&egui::FontFamily::Proportional].clone(),
        );
        ctx.set_fonts(fonts);
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(300.0);
            let mut positions = Vec::new();
            for (index, (depth, children)) in
                [(0, false), (0, true), (1, false)].into_iter().enumerate()
            {
                ui.push_id(index, |ui| {
                    let output = show(
                        ui,
                        Row {
                            name: "Category",
                            color: egui::Color32::BLUE,
                            depth,
                            has_children: children,
                            expanded: false,
                            selected: false,
                            searching: false,
                        },
                    );
                    positions.push((output.icon_left, output.text_left));
                });
            }
            assert_eq!(positions[0], positions[1]);
            assert_eq!(positions[2].0 - positions[0].0, 14.0);
            assert_eq!(positions[2].1 - positions[0].1, 14.0);
        });
    }
}
