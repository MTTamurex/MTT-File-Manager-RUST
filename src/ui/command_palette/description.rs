use crate::app::command_palette::{
    catalog::{Command, COMMANDS},
    input_hints,
};
use crate::app::shortcuts::ShortcutBindings;
use eframe::egui;
use rust_i18n::t;

pub(super) fn render(ui: &mut egui::Ui, command: Option<Command>, shortcuts: &ShortcutBindings) {
    ui.label(egui::RichText::new(t!("command_palette.description")).strong());
    ui.add_space(ui.text_style_height(&egui::TextStyle::Body));
    if let Some(command) = command {
        ui.add(egui::Label::new(input_hints::help(command, shortcuts)).wrap());
    }
}

pub(super) fn required_height(ui: &egui::Ui, width: f32, shortcuts: &ShortcutBindings) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let color = ui.visuals().text_color();
    let title_height = ui
        .painter()
        .layout(
            t!("command_palette.description").to_string(),
            font.clone(),
            color,
            width,
        )
        .size()
        .y;
    // Measure every command so navigating or filtering never resizes the window.
    let text_height = COMMANDS
        .iter()
        .map(|&command| {
            ui.painter()
                .layout(
                    input_hints::help(command, shortcuts),
                    font.clone(),
                    color,
                    width,
                )
                .size()
                .y
        })
        .fold(0.0, f32::max);
    title_height
        + ui.spacing().item_spacing.y
        + ui.text_style_height(&egui::TextStyle::Body)
        + text_height
}
