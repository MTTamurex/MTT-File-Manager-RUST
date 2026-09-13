use super::{
    egui, ContextMenuItem, ICON_TEXT_GAP, ITEM_ICON_SIZE, ITEM_TEXT_FONT_SIZE, MENU_MAX_WIDTH,
    SHORTCUT_TEXT_FONT_SIZE,
};

pub(super) fn submenu_width(ui: &egui::Ui, items: &[ContextMenuItem]) -> f32 {
    let measure = |text: &str, font_size: f32| {
        ui.painter()
            .layout_no_wrap(
                text.to_owned(),
                egui::FontId::proportional(font_size),
                ui.visuals().text_color(),
            )
            .size()
            .x
    };
    items
        .iter()
        .filter(|item| !item.is_separator)
        .map(|item| {
            let label = if item.text.chars().count() > 45 {
                format!("{}…", item.text.chars().take(43).collect::<String>())
            } else {
                item.text.clone()
            };
            let trailing = if !item.sub_items.is_empty() || item.has_pending_submenu {
                24.0
            } else {
                item.keyboard_shortcut.as_deref().map_or(0.0, |shortcut| {
                    20.0 + measure(shortcut, SHORTCUT_TEXT_FONT_SIZE)
                })
            };
            10.0 + ITEM_ICON_SIZE
                + ICON_TEXT_GAP
                + measure(&label, ITEM_TEXT_FONT_SIZE)
                + trailing
                + 10.0
        })
        .fold(120.0, f32::max)
        .min(MENU_MAX_WIDTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submenu_width_fits_content_and_shortcuts() {
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            let folder = submenu_width(ui, &[ContextMenuItem::new(1, "Folder")]);
            let long = submenu_width(
                ui,
                &[ContextMenuItem::new(2, "Open with another application")],
            );
            let shortcut = submenu_width(
                ui,
                &[ContextMenuItem::new(1, "Folder").with_shortcut("Ctrl+Shift+N")],
            );
            assert!(folder < 220.0);
            assert!(long > folder);
            assert!(shortcut > folder);
            assert!(long <= MENU_MAX_WIDTH);

            let label_width = ui
                .painter()
                .layout_no_wrap(
                    "Folder".to_owned(),
                    egui::FontId::proportional(ITEM_TEXT_FONT_SIZE),
                    ui.visuals().text_color(),
                )
                .size()
                .x;
            let shortcut_width = ui
                .painter()
                .layout_no_wrap(
                    "Ctrl+Shift+N".to_owned(),
                    egui::FontId::proportional(SHORTCUT_TEXT_FONT_SIZE),
                    ui.visuals().text_color(),
                )
                .size()
                .x;
            let expected_shortcut_width = (10.0
                + ITEM_ICON_SIZE
                + ICON_TEXT_GAP
                + label_width
                + 20.0
                + shortcut_width
                + 10.0)
                .clamp(120.0, MENU_MAX_WIDTH);
            assert!((shortcut - expected_shortcut_width).abs() < 0.1);
        });
    }
}
