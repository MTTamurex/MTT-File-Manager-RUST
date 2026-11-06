use super::model::Tab;
use eframe::egui;
use rust_i18n::t;

#[derive(Default)]
pub(super) struct Actions {
    pub add: bool,
    pub tab: Option<Tab>,
}

pub(super) fn show(ui: &mut egui::Ui, current: Tab) -> Actions {
    let labels = [
        if current == Tab::Tags {
            t!("tags.add")
        } else {
            t!("tags.auto_add")
        },
        t!("tags.section"),
        t!("tags.auto_section"),
    ];
    let mut actions = Actions::default();
    buttons(
        ui,
        [&labels[0], &labels[1], &labels[2]],
        current,
        |index| match index {
            0 => actions.add = true,
            1 => actions.tab = Some(Tab::Tags),
            _ => actions.tab = Some(Tab::AutoTag),
        },
    );
    actions
}

fn buttons(
    ui: &mut egui::Ui,
    labels: [&str; 3],
    current: Tab,
    mut clicked: impl FnMut(usize),
) -> [egui::Rect; 3] {
    let mut rects = [egui::Rect::NOTHING; 3];
    let font = egui::TextStyle::Button.resolve(ui.style());
    let text_width = labels
        .iter()
        .map(|label| {
            ui.painter()
                .layout_no_wrap(label.to_string(), font.clone(), ui.visuals().text_color())
                .size()
                .x
        })
        .fold(0.0_f32, f32::max);
    let width = (text_width + 32.0).max(96.0).min(ui.available_width());
    let height = (ui.text_style_height(&egui::TextStyle::Button) + 16.0).max(36.0);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(12.0, 12.0);
        ui.horizontal_wrapped(|ui| {
            for (index, label) in labels.into_iter().enumerate() {
                let mut button = egui::Button::new(label).corner_radius(6.0).truncate();
                if index > 0 {
                    button = button.selected(index == if current == Tab::Tags { 1 } else { 2 });
                }
                let response = ui.add_sized([width, height], button);
                rects[index] = response.rect;
                if response.clicked() {
                    clicked(index);
                }
            }
        });
    });
    rects
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toolbar_uses_equal_sizes_and_wraps_instead_of_overflowing() {
        for width in [240.0, 640.0] {
            let ctx = egui::Context::default();
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.set_width(width);
                let rects = buttons(ui, ["Add tag", "Tags", "Auto-tag"], Tab::Tags, |_| {});
                assert!(ui.min_rect().width() <= width + 1.0);
                assert_eq!(rects[0].size(), rects[1].size());
                assert_eq!(rects[1].size(), rects[2].size());
                assert!(rects[1].left() - rects[0].right() >= 11.9);
                if width > 300.0 {
                    assert!(rects[2].left() - rects[1].right() >= 11.9);
                } else {
                    assert!(rects[2].top() - rects[0].bottom() >= 11.9);
                }
            });
        }
    }
}
