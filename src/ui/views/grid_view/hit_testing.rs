use super::GridViewContext;
use crate::domain::file_entry::{FileEntry, SyncStatus};
use eframe::egui::{self, Color32, Pos2, Rect, Vec2};

pub(super) fn grid_item_content_contains(
    ui: &egui::Ui,
    item: &FileEntry,
    rect: Rect,
    ctx: &GridViewContext<'_>,
    point: Pos2,
) -> bool {
    if ctx.is_computer_view {
        return true;
    }

    if item.drive_info.is_some() {
        return grid_drive_content_contains(rect, ctx.thumbnail_size, point);
    }
    if item.is_dir && !item.is_archive() {
        return grid_folder_content_contains(ui, item, rect, ctx, point);
    }
    grid_file_content_contains(ui, item, rect, ctx, point)
}

fn grid_file_content_contains(
    ui: &egui::Ui,
    item: &FileEntry,
    rect: Rect,
    ctx: &GridViewContext<'_>,
    point: Pos2,
) -> bool {
    let available_h = rect.height();
    let available_w = rect.width();
    let thumb_size = (ctx.thumbnail_size - 6.0).min(available_w - 4.0).max(1.0);
    let text_height = 18.0;
    let content_h = thumb_size + text_height;
    let vertical_margin = ((available_h - content_h) / 2.0).max(2.0);
    let x_offset = (available_w - thumb_size) / 2.0;
    let thumb_rect = Rect::from_min_size(
        rect.min + egui::vec2(x_offset.max(0.0), vertical_margin),
        egui::vec2(thumb_size, thumb_size),
    );

    let visual_rect = if item.is_media() && !ctx.is_recycle_bin_view && !ctx.skip_folder_media_reads
    {
        ctx.texture_cache
            .peek(&item.path)
            .map(|texture| aspect_fit_rect(texture.size_vec2(), thumb_rect))
            .unwrap_or_else(|| centered_square(thumb_rect, thumb_size * 0.5))
    } else {
        centered_square(thumb_rect, thumb_size * 0.5)
    };

    let text_rect = Rect::from_min_size(
        egui::pos2(rect.left(), thumb_rect.bottom() + 4.0),
        egui::vec2(rect.width(), 20.0),
    );

    visual_rect.expand(2.0).contains(point)
        || sync_badge_rect(thumb_rect, item.sync_status).is_some_and(|rect| rect.contains(point))
        || grid_text_contains(ui, &item.name, text_rect, 11.0, point)
}

fn grid_folder_content_contains(
    ui: &egui::Ui,
    item: &FileEntry,
    rect: Rect,
    ctx: &GridViewContext<'_>,
    point: Pos2,
) -> bool {
    let available_h = rect.height();
    let folder_w = ctx.thumbnail_size * 0.85;
    let folder_h = folder_w * 0.85;
    let text_height = 18.0;
    let content_h = folder_h + text_height;
    let vertical_margin = ((available_h - content_h) / 2.0).max(2.0);
    let x_offset = (rect.width() - folder_w) / 2.0;
    let folder_rect = Rect::from_min_size(
        rect.min + egui::vec2(x_offset.max(0.0), vertical_margin),
        egui::vec2(folder_w, folder_h),
    );

    let is_special = crate::infrastructure::onedrive::is_special_icon_folder(&item.path);
    let visual_rect = if is_special {
        Rect::from_center_size(folder_rect.center(), egui::vec2(folder_w, folder_w))
    } else {
        ctx.folder_preview_cache
            .peek(&item.path)
            .map(|texture| aspect_fit_rect(texture.size_vec2(), folder_rect))
            .unwrap_or(folder_rect)
    };

    let text_rect = Rect::from_min_size(
        egui::pos2(rect.left(), folder_rect.bottom() + 6.0),
        egui::vec2(rect.width(), 20.0),
    );
    let display_name = crate::ui::components::item_slot::display_name_for_item(item);

    visual_rect.expand(2.0).contains(point)
        || sync_badge_rect(folder_rect, item.sync_status).is_some_and(|rect| rect.contains(point))
        || grid_text_contains(ui, display_name.as_ref(), text_rect, 11.0, point)
}

fn grid_text_contains(
    ui: &egui::Ui,
    text: &str,
    text_rect: Rect,
    font_size: f32,
    point: Pos2,
) -> bool {
    if text.is_empty() || text_rect.width() <= 0.0 || text_rect.height() <= 0.0 {
        return false;
    }

    let font_id = egui::FontId::proportional(font_size);
    let raw_width = ui.fonts(|fonts| {
        fonts
            .layout_no_wrap(text.to_string(), font_id, Color32::WHITE)
            .rect
            .width()
    });
    let visual_width = raw_width.min(text_rect.width()).max(1.0);
    Rect::from_min_size(text_rect.min, egui::vec2(visual_width, text_rect.height()))
        .expand(2.0)
        .contains(point)
}

fn grid_drive_content_contains(rect: Rect, thumbnail_size: f32, point: Pos2) -> bool {
    let available_h = rect.height();
    let available_w = rect.width();
    let icon_size = (thumbnail_size * 0.4).min(available_w * 0.5);
    let progress_w = (available_w * 0.8).min(150.0);
    let text_height = 36.0;
    let content_h = icon_size + 12.0 + 8.0 + text_height;
    let vertical_margin = ((available_h - content_h) / 2.0).max(2.0);

    let mut current_y = rect.top() + vertical_margin;
    let icon_rect = Rect::from_center_size(
        egui::pos2(rect.center().x, current_y + icon_size / 2.0),
        egui::vec2(icon_size, icon_size),
    );
    current_y += icon_size + 8.0;

    let bar_rect = Rect::from_center_size(
        egui::pos2(rect.center().x, current_y + 6.0),
        egui::vec2(progress_w, 12.0),
    );
    current_y += 18.0;

    let name_rect = Rect::from_center_size(
        egui::pos2(rect.center().x, current_y + 9.0),
        egui::vec2(progress_w, 18.0),
    );
    current_y += 18.0;

    let details_rect = Rect::from_center_size(
        egui::pos2(rect.center().x, current_y + 9.0),
        egui::vec2(progress_w, 18.0),
    );

    [icon_rect, bar_rect, name_rect, details_rect]
        .into_iter()
        .any(|rect| rect.expand(2.0).contains(point))
}

fn aspect_fit_rect(texture_size: Vec2, container: Rect) -> Rect {
    if texture_size.x <= 0.0 || texture_size.y <= 0.0 || container.width() <= 0.0 {
        return container;
    }

    let aspect = texture_size.x / texture_size.y;
    let container_aspect = container.width() / container.height();
    let (draw_w, draw_h) = if aspect > container_aspect {
        (container.width(), container.width() / aspect)
    } else {
        (container.height() * aspect, container.height())
    };

    Rect::from_center_size(container.center(), egui::vec2(draw_w, draw_h))
}

fn centered_square(container: Rect, side: f32) -> Rect {
    let side = side.min(container.width()).min(container.height()).max(1.0);
    Rect::from_center_size(container.center(), egui::vec2(side, side))
}

fn sync_badge_rect(thumb_rect: Rect, status: SyncStatus) -> Option<Rect> {
    if status == SyncStatus::None {
        return None;
    }

    let badge_size = 18.0;
    Some(Rect::from_min_size(
        egui::pos2(
            thumb_rect.right() - badge_size - 2.0,
            thumb_rect.bottom() - badge_size - 2.0,
        ),
        egui::vec2(badge_size, badge_size),
    ))
}
