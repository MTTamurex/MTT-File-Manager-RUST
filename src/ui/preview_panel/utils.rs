use eframe::egui;

// Helper function to truncate text to fit within a given width
pub fn truncate_text_to_fit(
    text: &str,
    max_width: f32,
    font_id: &egui::FontId,
    ui: &egui::Ui,
) -> String {
    let fonts = ui.fonts(|f| f.clone());
    let galley = fonts.layout_no_wrap(text.to_string(), font_id.clone(), egui::Color32::WHITE);

    if galley.rect.width() <= max_width {
        return text.to_string();
    }

    // Binary search for the right length
    let mut left = 0;
    let mut right = text.chars().count();
    let ellipsis = "...";
    let ellipsis_galley =
        fonts.layout_no_wrap(ellipsis.to_string(), font_id.clone(), egui::Color32::WHITE);
    let ellipsis_width = ellipsis_galley.rect.width();
    let available_width = max_width - ellipsis_width;

    while left < right {
        let mid = (left + right).div_ceil(2);
        let truncated: String = text.chars().take(mid).collect();
        let test_galley =
            fonts.layout_no_wrap(truncated.clone(), font_id.clone(), egui::Color32::WHITE);

        if test_galley.rect.width() <= available_width {
            left = mid;
        } else {
            right = mid - 1;
        }
    }

    if left == 0 {
        return ellipsis.to_string();
    }

    let truncated: String = text.chars().take(left).collect();
    format!("{}{}", truncated, ellipsis)
}
