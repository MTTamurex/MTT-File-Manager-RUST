use crate::ui::components::MediaPreview;
use crate::ui::preview_panel::actions::PreviewPanelAction;
use crate::ui::preview_panel::video_preview::controls::draw_video_controls;
use crate::ui::svg_icons::SvgIconManager;
use eframe::egui;

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
    preview.show(ui, frame);

    // Controls bar BELOW the video (uses shared draw_video_controls)
    ui.add_space(8.0);
    let mut vol_action = None;
    ui.vertical(|ui| {
        if let Some(vol) = draw_video_controls(
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
            vol_action = Some(PreviewPanelAction::VolumeChanged(vol));
        }
    });
    vol_action
}
