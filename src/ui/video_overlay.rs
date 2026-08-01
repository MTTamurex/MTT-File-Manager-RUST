use eframe::egui;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct VideoOverlay {
    pub rect: egui::Rect,
    pub corner_radius: f32,
}

#[derive(Clone, Default)]
struct FrameState {
    video_rect: Option<egui::Rect>,
    overlays: Vec<VideoOverlay>,
    seen_popup_ids: HashSet<egui::Id>,
}

fn frame_state_id() -> egui::Id {
    egui::Id::new("video_overlay_frame_state")
}

fn shadow_suppression_id() -> egui::Id {
    egui::Id::new("video_overlay_shadow_suppression")
}

pub fn begin_frame(ctx: &egui::Context) {
    ctx.data_mut(|data| data.insert_temp(frame_state_id(), FrameState::default()));
}

pub fn set_video_rect(ctx: &egui::Context, rect: egui::Rect) {
    if rect.is_positive() {
        ctx.data_mut(|data| {
            data.get_temp_mut_or_default::<FrameState>(frame_state_id())
                .video_rect = Some(rect);
        });
    }
}

pub fn should_suppress_shadow(ctx: &egui::Context, popup_id: egui::Id) -> bool {
    ctx.data(|data| {
        data.get_temp::<HashMap<egui::Id, bool>>(shadow_suppression_id())
            .and_then(|states| states.get(&popup_id).copied())
            .unwrap_or(false)
    })
}

pub fn popup_frame(ui: &egui::Ui, popup_id: egui::Id) -> egui::Frame {
    let frame = egui::Frame::popup(ui.style());
    if should_suppress_shadow(ui.ctx(), popup_id) {
        frame.shadow(egui::epaint::Shadow::NONE)
    } else {
        frame
    }
}

pub fn without_shadow_if_needed<R>(
    ctx: &egui::Context,
    popup_id: egui::Id,
    render: impl FnOnce() -> R,
) -> R {
    if !should_suppress_shadow(ctx, popup_id) {
        return render();
    }

    let original_shadow = ctx.global_style().visuals.popup_shadow;
    ctx.global_style_mut(|style| style.visuals.popup_shadow = egui::epaint::Shadow::NONE);
    let result = render();
    ctx.global_style_mut(|style| {
        if style.visuals.popup_shadow == egui::epaint::Shadow::NONE {
            style.visuals.popup_shadow = original_shadow;
        }
    });
    result
}

pub fn register_rect(ctx: &egui::Context, popup_id: egui::Id, rect: egui::Rect) {
    if !rect.is_positive() {
        return;
    }

    let style = ctx.global_style();
    let radius = style.visuals.menu_corner_radius;
    let corner_radius = radius.nw.max(radius.ne).max(radius.sw).max(radius.se) as f32;
    let intersects_video = ctx.data_mut(|data| {
        let state = data.get_temp_mut_or_default::<FrameState>(frame_state_id());
        state.overlays.push(VideoOverlay {
            rect,
            corner_radius,
        });
        state.seen_popup_ids.insert(popup_id);
        state
            .video_rect
            .is_some_and(|video_rect| video_rect.intersect(rect).is_positive())
    });

    let changed = ctx.data_mut(|data| {
        let states =
            data.get_temp_mut_or_default::<HashMap<egui::Id, bool>>(shadow_suppression_id());
        let previous = states.get(&popup_id).copied().unwrap_or(false);
        if intersects_video {
            states.insert(popup_id, true);
        } else {
            states.remove(&popup_id);
        }
        previous != intersects_video
    });

    if changed {
        ctx.request_discard("popup shadow changed at native video boundary");
        ctx.request_repaint();
    }
}

pub fn current_rects(ctx: &egui::Context) -> Vec<VideoOverlay> {
    ctx.data(|data| {
        data.get_temp::<FrameState>(frame_state_id())
            .map(|state| state.overlays)
            .unwrap_or_default()
    })
}

pub fn finish_frame(ctx: &egui::Context) {
    let seen_popup_ids = ctx.data(|data| {
        data.get_temp::<FrameState>(frame_state_id())
            .map(|state| state.seen_popup_ids)
            .unwrap_or_default()
    });
    ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<HashMap<egui::Id, bool>>(shadow_suppression_id())
            .retain(|popup_id, _| seen_popup_ids.contains(popup_id));
    });
}

#[cfg(test)]
mod tests {
    use super::{begin_frame, finish_frame, register_rect, set_video_rect, should_suppress_shadow};
    use eframe::egui;

    #[test]
    fn suppresses_only_popup_intersecting_video() {
        let ctx = egui::Context::default();
        let overlapping = egui::Id::new("overlapping");
        let separate = egui::Id::new("separate");
        begin_frame(&ctx);
        set_video_rect(
            &ctx,
            egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(100.0, 100.0)),
        );

        register_rect(
            &ctx,
            overlapping,
            egui::Rect::from_min_size(egui::pos2(50.0, 50.0), egui::vec2(100.0, 100.0)),
        );
        register_rect(
            &ctx,
            separate,
            egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(25.0, 25.0)),
        );

        assert!(should_suppress_shadow(&ctx, overlapping));
        assert!(!should_suppress_shadow(&ctx, separate));
    }

    #[test]
    fn removes_suppression_after_popup_closes() {
        let ctx = egui::Context::default();
        let popup_id = egui::Id::new("popup");
        begin_frame(&ctx);
        set_video_rect(
            &ctx,
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100.0, 100.0)),
        );
        register_rect(
            &ctx,
            popup_id,
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(50.0, 50.0)),
        );
        finish_frame(&ctx);
        assert!(should_suppress_shadow(&ctx, popup_id));

        begin_frame(&ctx);
        finish_frame(&ctx);
        assert!(!should_suppress_shadow(&ctx, popup_id));
    }
}
