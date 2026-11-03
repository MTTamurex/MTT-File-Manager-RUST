use eframe::egui;

/// Keep the settings layout usable without imposing its size on the window.
pub(super) fn show<R>(
    ui: &mut egui::Ui,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::scroll_area::ScrollAreaOutput<R> {
    egui::ScrollArea::both()
        .id_salt("settings_window_body")
        .auto_shrink([false, false])
        .min_scrolled_width(0.0)
        .min_scrolled_height(0.0)
        .show(ui, |ui| {
            ui.set_min_size(egui::vec2(700.0, 420.0));
            add_contents(ui)
        })
}

#[cfg(test)]
mod tests;
