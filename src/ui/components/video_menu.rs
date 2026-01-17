use eframe::egui;
use crate::ui::components::mpv_preview::TrackInfo;

#[derive(Clone, Default)]
pub struct VideoMenuState {
    pub is_open: bool,
    pub position: egui::Pos2,
    pub active_submenu: Option<String>, // "audio" or "subtitle"
    pub submenu_position: Option<egui::Pos2>,
}

#[derive(PartialEq)]
pub enum VideoMenuAction {
    None,
    TogglePlay,
    ToggleMute,
    SetAudioTrack(i64),
    SetSubtitleTrack(i64),
    Close,
}

pub fn render_video_menu(
    ctx: &egui::Context,
    state: &mut VideoMenuState,
    audio_tracks: &[TrackInfo],
    sub_tracks: &[TrackInfo],
) -> VideoMenuAction {
    let mut action = VideoMenuAction::None;

    if !state.is_open {
        // Menu is closed - ensure submenu state is cleared
        state.active_submenu = None;
        state.submenu_position = None;
        return action;
    }

    // Capture submenu info before main menu rendering
    let submenu_to_render = state.active_submenu.clone();
    let submenu_pos = state.submenu_position;

    // --- MAIN MENU VIEWPORT ---
    let viewport_id = egui::ViewportId::from_hash_of("video_context_menu");
    
    ctx.show_viewport_immediate(
        viewport_id,
        egui::ViewportBuilder::default()
            .with_title("Video Menu")
            .with_decorations(false)
            .with_always_on_top()
            .with_visible(true)
            .with_taskbar(false)
            .with_inner_size([200.0, 100.0])
            .with_position(state.position),
        |ctx, _class| {
            egui::CentralPanel::default().frame(egui::Frame::popup(&ctx.style())).show(ctx, |ui| {
                ui.set_max_width(250.0);
                
                let audio_btn = ui.add(egui::Button::new("🔊 Áudio ›").frame(false));
                if audio_btn.hovered() {
                    state.active_submenu = Some("audio".to_string());
                    let offset = audio_btn.rect.right_top().to_vec2();
                    state.submenu_position = Some(state.position + offset);
                }

                let sub_btn = ui.add(egui::Button::new("💬 Legendas ›").frame(false));
                if sub_btn.hovered() {
                    state.active_submenu = Some("subtitle".to_string());
                    let offset = sub_btn.rect.right_top().to_vec2();
                    state.submenu_position = Some(state.position + offset);
                }

                ui.separator();

                if ui.button("Fechar menu").clicked() {
                    action = VideoMenuAction::Close;
                }

                if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    action = VideoMenuAction::Close;
                }
            });
        },
    );

    // --- SUBMENU VIEWPORT (rendered separately, not nested) ---
    // Only render if menu is still open and we have a submenu to show
    if action == VideoMenuAction::None {
        if let (Some(submenu), Some(pos)) = (submenu_to_render, submenu_pos) {
            let submenu_id = egui::ViewportId::from_hash_of(format!("video_submenu_{}", submenu));
            
            ctx.show_viewport_immediate(
                submenu_id,
                egui::ViewportBuilder::default()
                    .with_title("Submenu")
                    .with_decorations(false)
                    .with_always_on_top()
                    .with_visible(true)
                    .with_taskbar(false)
                    .with_inner_size([200.0, 300.0])
                    .with_position(pos),
                |ctx, _class| {
                    egui::CentralPanel::default().frame(egui::Frame::popup(&ctx.style())).show(ctx, |ui| {
                        egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                            match submenu.as_str() {
                                "audio" => {
                                    ui.label(egui::RichText::new("Faixas de Áudio").strong());
                                    ui.separator();
                                    if audio_tracks.is_empty() {
                                        ui.label("Nenhuma faixa de áudio");
                                    } else {
                                        for track in audio_tracks {
                                            let label = track.title.as_deref().unwrap_or("Faixa sem título");
                                            let lang = track.lang.as_deref().unwrap_or("unk");
                                            let text = format!("{} ({})", label, lang);
                                            
                                            let mut selected = track.selected;
                                            if ui.checkbox(&mut selected, text).clicked() {
                                                action = VideoMenuAction::SetAudioTrack(track.id);
                                            }
                                        }
                                    }
                                }
                                "subtitle" => {
                                    ui.label(egui::RichText::new("Legendas").strong());
                                    ui.separator();
                                    
                                    if ui.button("Desativar legendas").clicked() {
                                        action = VideoMenuAction::SetSubtitleTrack(0);
                                    }

                                    if sub_tracks.is_empty() {
                                        ui.label("Nenhuma legenda encontrada");
                                    } else {
                                        for track in sub_tracks {
                                            let label = track.title.as_deref().unwrap_or("Legenda");
                                            let lang = track.lang.as_deref().unwrap_or("unk");
                                            let text = format!("{} ({})", label, lang);
                                            
                                            let mut selected = track.selected;
                                            if ui.checkbox(&mut selected, text).clicked() {
                                                action = VideoMenuAction::SetSubtitleTrack(track.id);
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        });
                    });
                },
            );
        }
    }
    
    // Handle closing logic
    if action == VideoMenuAction::Close {
        // Close everything at once
        state.active_submenu = None;
        state.submenu_position = None;
        state.is_open = false;
    } else if matches!(action, VideoMenuAction::SetAudioTrack(_)) || matches!(action, VideoMenuAction::SetSubtitleTrack(_)) {
        // Close everything when a selection is made
        state.active_submenu = None;
        state.submenu_position = None;
        state.is_open = false;
    }

    action
}
