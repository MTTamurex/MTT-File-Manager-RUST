use crate::ui::components::MediaPreview;
use crate::ui::preview_panel::actions::PreviewPanelAction;
use crate::ui::preview_panel::video_preview::controls::{draw_video_controls, ControlAction};
use crate::ui::svg_icons::SvgIconManager;
use eframe::egui;

fn docked_bounds(available: egui::Rect, clip: egui::Rect, max_width: f32) -> egui::Rect {
    let left = available.left().max(clip.left());
    let right = available.right().min(clip.right()).max(left);
    let width = max_width.max(0.0).min(right - left);
    let left = left + (right - left - width) * 0.5;
    egui::Rect::from_min_max(
        egui::pos2(left, available.top()),
        egui::pos2(left + width, available.bottom()),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn render_docked_video(
    ui: &mut egui::Ui,
    preview: &mut MediaPreview,
    svg_manager: &mut SvgIconManager,
    frame: Option<&eframe::Frame>,
    max_preview_width: f32,
    current_time: f64,
    duration: f64,
    volume: f32,
    is_muted: bool,
    is_playing: bool,
) -> Option<PreviewPanelAction> {
    // A scroll area's content can be wider than its viewport after a resize.
    // Bound both the native video surface and its controls to the visible width.
    let bounds = docked_bounds(
        ui.available_rect_before_wrap(),
        ui.clip_rect(),
        max_preview_width,
    );
    let width = bounds.width();
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(bounds)
            .layout(egui::Layout::top_down(egui::Align::Center)),
        |ui| {
            render_docked_content(
                ui,
                preview,
                svg_manager,
                frame,
                width,
                current_time,
                duration,
                volume,
                is_muted,
                is_playing,
            )
        },
    )
    .inner
}

#[allow(clippy::too_many_arguments)]
fn render_docked_content(
    ui: &mut egui::Ui,
    preview: &mut MediaPreview,
    svg_manager: &mut SvgIconManager,
    frame: Option<&eframe::Frame>,
    max_preview_width: f32,
    current_time: f64,
    duration: f64,
    volume: f32,
    is_muted: bool,
    is_playing: bool,
) -> Option<PreviewPanelAction> {
    preview.show(ui, frame);

    // Controls bar BELOW the video (uses shared draw_video_controls)
    ui.add_space(8.0);
    let mut action = None;
    ui.vertical(|ui| {
        if let Some(ctrl) = draw_video_controls(
            ui,
            preview,
            max_preview_width,
            svg_manager,
            is_playing,
            current_time,
            duration,
            volume,
            is_muted,
            false, // is_detached = false for docked mode
        ) {
            match ctrl {
                ControlAction::VolumeChanged(vol) => {
                    action = Some(PreviewPanelAction::VolumeChanged(vol));
                }
                ControlAction::DetachRequested => {
                    // Build DetachVideo action with current player state
                    if let Some(path) = preview.path().map(|p| p.to_path_buf()) {
                        action = Some(PreviewPanelAction::DetachVideo {
                            path,
                            position: current_time,
                            volume,
                        });
                    }
                }
            }
        }
    });
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_bounds_follow_viewport_instead_of_previous_content_width() {
        let available = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(500.0, 600.0));
        for width in [226.0, 210.0, 190.0, 400.0, 190.0] {
            let clip = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 800.0));
            let bounds = docked_bounds(available, clip, 484.0);
            assert_eq!(bounds.left(), available.left());
            assert_eq!(bounds.right(), clip.right());
            assert_eq!(bounds.top(), available.top());
        }
    }

    #[test]
    fn video_remains_centered_with_preview_margins() {
        let available = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(226.0, 600.0));
        let bounds = docked_bounds(available, available, 210.0);
        assert_eq!(bounds.width(), 210.0);
        assert_eq!(bounds.center().x, available.center().x);
    }
}
