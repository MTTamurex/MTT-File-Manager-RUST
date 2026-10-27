use super::{breadcrumb_button_with_label, measure_text_width};
use crate::ui::preview_panel::utils::truncate_text_to_fit;
use eframe::egui;

const MIN_ROW_WIDTH: f32 = 120.0;
const MAX_ROW_WIDTH: f32 = 240.0;
const TEXT_PADDING: f32 = 8.0;

fn row_width(ui: &egui::Ui, segments: &[(String, String)], hidden: &[usize]) -> f32 {
    let font = egui::FontId::proportional(11.0);
    let text_width = hidden
        .iter()
        .map(|&idx| measure_text_width(ui, &segments[idx].0, &font))
        .fold(0.0, f32::max);
    (text_width + TEXT_PADDING * 2.0).clamp(MIN_ROW_WIDTH, MAX_ROW_WIDTH)
}

/// Renders hidden segments in path order, keeping the popup compact even for long names.
pub(super) fn render_overflow_popup(
    ui: &mut egui::Ui,
    segments: &[(String, String)],
    hidden_indices: &[usize],
    text_color: egui::Color32,
) -> Option<String> {
    let popup_id = egui::Id::new("details_breadcrumb_overflow_popup");
    let mut show_overflow = ui
        .ctx()
        .memory(|m| m.data.get_temp::<bool>(popup_id).unwrap_or(false));
    let ellipsis_resp = ui
        .scope(|ui| breadcrumb_button_with_label(ui, egui::RichText::new("…").color(text_color)))
        .inner;

    if ellipsis_resp.clicked() {
        show_overflow = !show_overflow;
        ui.ctx()
            .memory_mut(|m| m.data.insert_temp(popup_id, show_overflow));
    }
    if !show_overflow {
        return None;
    }

    let width = row_width(ui, segments, hidden_indices);
    let mut selected_path = None;
    let popup_response = egui::Area::new(popup_id)
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(
            ellipsis_resp.rect.left(),
            ellipsis_resp.rect.bottom() + 2.0,
        ))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing.y = 0.0;
                for &idx in hidden_indices {
                    let (display, actual_path) = &segments[idx];
                    if render_row(ui, display, width, text_color).clicked() {
                        selected_path = Some(actual_path.clone());
                    }
                }
            });
        });

    if let Some(path) = selected_path {
        ui.ctx()
            .memory_mut(|m| m.data.insert_temp::<bool>(popup_id, false));
        return Some(path);
    }
    if ui.ctx().input(|i| i.pointer.any_pressed()) {
        if let Some(pos) = ui.ctx().input(|i| i.pointer.press_origin()) {
            if !ellipsis_resp.rect.contains(pos) && !popup_response.response.rect.contains(pos) {
                ui.ctx()
                    .memory_mut(|m| m.data.insert_temp::<bool>(popup_id, false));
            }
        }
    }
    None
}

fn render_row(
    ui: &mut egui::Ui,
    display: &str,
    width: f32,
    text_color: egui::Color32,
) -> egui::Response {
    let font = egui::FontId::proportional(11.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 28.0), egui::Sense::click());
    let visuals = ui.style().interact(&response);
    if response.hovered() || response.highlighted() {
        ui.painter()
            .rect_filled(rect, visuals.corner_radius, visuals.weak_bg_fill);
    }
    let text_rect = rect.shrink2(egui::vec2(TEXT_PADDING, 0.0));
    let truncated = truncate_text_to_fit(display, text_rect.width(), &font, ui);
    ui.painter().text(
        egui::pos2(text_rect.left(), rect.center().y),
        egui::Align2::LEFT_CENTER,
        &truncated,
        font,
        text_color,
    );
    if truncated != display {
        response.on_hover_text(display)
    } else {
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_tracks_hidden_names_with_a_compact_upper_bound() {
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            let segments = vec![
                ("Folder".into(), r"C:\Folder".into()),
                ("Folder name ".repeat(30), r"C:\Folder\Subfolder".into()),
            ];
            assert_eq!(row_width(ui, &segments, &[0]), MIN_ROW_WIDTH);
            assert_eq!(row_width(ui, &segments, &[0, 1]), MAX_ROW_WIDTH);
            let medium = vec![(
                "Folder with a moderately long name".to_owned(),
                String::new(),
            )];
            let expected = measure_text_width(ui, &medium[0].0, &egui::FontId::proportional(11.0))
                + TEXT_PADDING * 2.0;
            assert!((row_width(ui, &medium, &[0]) - expected).abs() < 0.01);
        });
    }

    fn painted_text(output: &egui::FullOutput) -> Vec<&str> {
        output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape {
                    Some(text.galley.text())
                } else {
                    None
                }
            })
            .collect()
    }

    #[test]
    fn long_names_are_truncated_and_show_the_full_name_on_hover() {
        let ctx = egui::Context::default();
        let name = "Example folder 日本語 ".repeat(20);
        let mut rect = egui::Rect::NOTHING;
        let output = ctx.run_ui(
            egui::RawInput {
                time: Some(0.0),
                ..Default::default()
            },
            |ui| {
                rect = render_row(ui, &name, MAX_ROW_WIDTH, egui::Color32::WHITE).rect;
            },
        );
        assert_eq!(rect.width(), MAX_ROW_WIDTH);
        let texts = painted_text(&output);
        assert!(!texts.contains(&name.as_str()));
        assert!(texts.iter().any(|text| text.ends_with("...")));

        let mut tooltip_shown = false;
        for frame in 1..=5 {
            let output = ctx.run_ui(
                egui::RawInput {
                    time: Some(frame as f64),
                    events: if frame == 1 {
                        vec![egui::Event::PointerMoved(rect.center())]
                    } else {
                        Vec::new()
                    },
                    ..Default::default()
                },
                |ui| {
                    render_row(ui, &name, MAX_ROW_WIDTH, egui::Color32::WHITE);
                },
            );
            tooltip_shown |= painted_text(&output).contains(&name.as_str());
        }
        assert!(
            tooltip_shown,
            "The tooltip must contain the complete folder name"
        );
    }

    #[test]
    fn short_names_are_not_truncated() {
        let ctx = egui::Context::default();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            render_row(ui, "Folder", MIN_ROW_WIDTH, egui::Color32::WHITE);
        });
        assert!(painted_text(&output).contains(&"Folder"));
    }

    #[test]
    fn popup_shrinks_after_long_names_and_keeps_navigation_working() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("details_breadcrumb_overflow_popup");
        ctx.memory_mut(|m| m.data.insert_temp(id, true));
        let render = |segments: &[(String, String)], events| {
            let mut selected = None;
            let _ = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    selected = render_overflow_popup(ui, segments, &[0], egui::Color32::WHITE);
                },
            );
            selected
        };
        let long = vec![("Example folder ".repeat(30), r"C:\Folder".into())];
        for _ in 0..2 {
            render(&long, Vec::new());
        }
        let long_width = egui::AreaState::load(&ctx, id).unwrap().rect().width();
        assert!((MAX_ROW_WIDTH..MAX_ROW_WIDTH + 24.0).contains(&long_width));

        let short = vec![("Folder".into(), r"C:\Folder".into())];
        for _ in 0..2 {
            render(&short, Vec::new());
        }
        let rect = egui::AreaState::load(&ctx, id).unwrap().rect();
        assert!((MIN_ROW_WIDTH..MIN_ROW_WIDTH + 24.0).contains(&rect.width()));
        let pos = rect.min + egui::vec2(12.0, 12.0);
        render(
            &short,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let selected = render(
            &short,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert_eq!(selected.as_deref(), Some(r"C:\Folder"));
        assert!(!ctx.memory(|m| m.data.get_temp::<bool>(id).unwrap_or(false)));
    }

    #[test]
    fn truncated_popup_item_shows_full_tooltip_and_returns_unmodified_target() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("details_breadcrumb_overflow_popup");
        ctx.memory_mut(|m| m.data.insert_temp(id, true));
        let name = "Example folder 日本語 ".repeat(20);
        let target = format!(r"C:\Folder\{name}");
        let segments = vec![(name.clone(), target.clone())];
        let render = |time, events| {
            let mut selected = None;
            let output = ctx.run_ui(
                egui::RawInput {
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    selected = render_overflow_popup(ui, &segments, &[0], egui::Color32::WHITE);
                },
            );
            (selected, output)
        };
        render(0.0, Vec::new());
        let (_, output) = render(0.1, Vec::new());
        assert!(painted_text(&output)
            .iter()
            .any(|text| text.ends_with("...")));
        assert!(!painted_text(&output).contains(&name.as_str()));
        let rect = egui::AreaState::load(&ctx, id).unwrap().rect();
        let pos = rect.center();
        let mut tooltip_shown = false;
        for frame in 1..=5 {
            let events = if frame == 1 {
                vec![egui::Event::PointerMoved(pos)]
            } else {
                Vec::new()
            };
            let (_, output) = render(frame as f64, events);
            tooltip_shown |= painted_text(&output).contains(&name.as_str());
        }
        assert!(tooltip_shown);
        render(
            6.0,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let (selected, _) = render(
            6.1,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert_eq!(selected, Some(target));
        assert!(!ctx.memory(|m| m.data.get_temp::<bool>(id).unwrap_or(false)));
    }

    #[test]
    fn clicking_outside_closes_the_popup_without_navigating() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("details_breadcrumb_overflow_popup");
        ctx.memory_mut(|m| m.data.insert_temp(id, true));
        let segments = vec![("Folder".into(), r"C:\Folder".into())];
        for frame in 0..3 {
            let events = if frame == 2 {
                let pos = egui::pos2(600.0, 400.0);
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
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
                    assert!(
                        render_overflow_popup(ui, &segments, &[0], egui::Color32::WHITE).is_none()
                    );
                },
            );
        }
        assert!(!ctx.memory(|m| m.data.get_temp::<bool>(id).unwrap_or(false)));
    }
}
