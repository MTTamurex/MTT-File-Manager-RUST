use super::*;

/// Renderiza um slot de drive (Este Computador)
pub(super) fn render_drive_slot(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    ctx: &mut ItemSlotContext,
    drive_info: &crate::domain::file_entry::DriveInfo,
) {
    let item = ctx.item;

    // Carrega ícone real do drive
    let drive_icon = ctx
        .icon_loader
        .get_or_load_drive_icon(ui.ctx(), &item.path.to_string_lossy());

    // GEOMETRIA
    let available_h = rect.height();
    let available_w = rect.width();
    let icon_size = (ctx.thumbnail_size * 0.4).min(available_w * 0.5);
    let progress_w = (available_w * 0.8).min(150.0);
    let text_height = 36.0; // Nome + Espaço Livre
    let content_h = icon_size + 12.0 + 8.0 + text_height; // Ícone + Barra + Padding + Texto

    let vertical_margin = ((available_h - content_h) / 2.0).max(2.0);

    // Use `rect` as base for calculation instead of allocating space
    let start_y = rect.top() + vertical_margin;
    let mut current_y = start_y;

    // 1. ÍCONE
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, current_y + icon_size / 2.0),
        egui::vec2(icon_size, icon_size),
    );

    if let Some(tex) = drive_icon {
        ui.put(
            icon_rect,
            egui::Image::new(&tex)
                .max_size(egui::vec2(icon_size, icon_size))
                .maintain_aspect_ratio(true),
        );
    } else {
        ui.painter().text(
            icon_rect.center(),
            egui::Align2::CENTER_CENTER,
            "💽",
            egui::FontId::proportional(icon_size * 0.8),
            egui::Color32::GRAY,
        );
    }

    current_y += icon_size + 8.0;

    // 2. BARRA DE PROGRESSO (Espaço Usado)
    if drive_info.total_space > 0 {
        let bar_rect = egui::Rect::from_center_size(
            egui::pos2(rect.center().x, current_y + 6.0),
            egui::vec2(progress_w, 12.0),
        );

        let used_space = drive_info.total_space - drive_info.free_space;
        let usage_ratio = used_space as f32 / drive_info.total_space as f32;

        // Cor da barra: azul ou vermelho se estiver cheio (> 90%)
        let bar_color = if usage_ratio > 0.9 {
            egui::Color32::from_rgb(230, 50, 50) // Vermelho
        } else {
            egui::Color32::from_rgb(30, 130, 230) // Azul Windows
        };

        let bg_color = egui::Color32::from_gray(230);

        ui.painter().rect_filled(bar_rect, 2.0, bg_color);

        let filled_w = progress_w * usage_ratio;
        let filled_rect = egui::Rect::from_min_size(bar_rect.min, egui::vec2(filled_w, 12.0));
        ui.painter().rect_filled(filled_rect, 2.0, bar_color);

        // Add hover interaction for the bar
        ui.interact(bar_rect, ui.id().with("drive_bar"), egui::Sense::hover());
    }

    current_y += 12.0 + 6.0;

    // 3. TEXTO (Nome e Espaço Livre)
    // Label for Name
    let name_rect = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, current_y + 9.0), // ~half text height
        egui::vec2(progress_w, 18.0),
    );

    ui.put(
        name_rect,
        egui::Label::new(egui::RichText::new(&item.name).size(11.0).strong()).truncate(),
    );

    current_y += 18.0;

    if drive_info.total_space > 0 {
        let free_gb = drive_info.free_space as f64 / (1024.0 * 1024.0 * 1024.0);
        let total_gb = drive_info.total_space as f64 / (1024.0 * 1024.0 * 1024.0);

        let (free_val, unit) = if total_gb >= 1000.0 {
            (free_gb / 1024.0, "TB")
        } else {
            (free_gb, "GB")
        };

        let (total_val, total_unit) = if total_gb >= 1000.0 {
            (total_gb / 1024.0, "TB")
        } else {
            (total_gb, "GB")
        };

        let details_rect = egui::Rect::from_center_size(
            egui::pos2(rect.center().x, current_y + 9.0),
            egui::vec2(progress_w, 18.0),
        );

        ui.put(
            details_rect,
            egui::Label::new(
                egui::RichText::new(format!(
                    "{:.1} {} livres de {:.1} {}",
                    free_val, unit, total_val, total_unit
                ))
                .size(9.0)
                .color(egui::Color32::from_gray(100)),
            )
            .truncate(),
        );
    }
}
