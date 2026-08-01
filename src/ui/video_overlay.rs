use eframe::egui;

fn overlay_rects_id() -> egui::Id {
    egui::Id::new("video_overlay_rects")
}

pub fn begin_frame(ctx: &egui::Context) {
    ctx.data_mut(|data| data.insert_temp(overlay_rects_id(), Vec::<egui::Rect>::new()));
}

pub fn register_rect(ctx: &egui::Context, rect: egui::Rect) {
    if !rect.is_positive() {
        return;
    }

    ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<Vec<egui::Rect>>(overlay_rects_id())
            .push(rect);
    });
}

pub fn current_rects(ctx: &egui::Context) -> Vec<egui::Rect> {
    ctx.data(|data| data.get_temp(overlay_rects_id()).unwrap_or_default())
}
