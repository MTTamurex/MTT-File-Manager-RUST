use eframe::egui;
use crate::ui::components::mpv_preview::TrackInfo;

const MENU_WIDTH: f32 = 200.0;

#[derive(Clone, Default)]
pub struct VideoMenuState {
    pub is_open: bool,
    pub position: egui::Pos2,
    pub active_submenu: Option<String>, // "audio" or "subtitle"
    pub submenu_position: Option<egui::Pos2>,
    pub main_menu_rect: Option<egui::Rect>,
    pub submenu_rect: Option<egui::Rect>,
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
        state.main_menu_rect = None;
        state.submenu_rect = None;
        return action;
    }

    // Capture submenu info before main menu rendering
    let submenu_to_render = state.active_submenu.clone();
    let submenu_pos = state.submenu_position;

    // --- MAIN MENU VIEWPORT ---
    let viewport_id = egui::ViewportId::from_hash_of("video_context_menu");
    let menu_pos = state.position;
    
    ctx.show_viewport_immediate(
        viewport_id,
        egui::ViewportBuilder::default()
            .with_title("Video Menu")
            .with_decorations(false)
            .with_always_on_top()
            .with_visible(true)
            .with_taskbar(false)
            .with_transparent(true)
            .with_inner_size([MENU_WIDTH, 100.0])
            .with_position(state.position),
        |ctx, _class| {
            egui::CentralPanel::default().frame(egui::Frame::popup(&ctx.style())).show(ctx, |ui| {
                ui.set_max_width(250.0);
                
                let audio_btn = ui.add(egui::Button::new("🔊 Áudio ›").frame(false));
                if audio_btn.hovered() {
                    state.active_submenu = Some("audio".to_string());
                    // Position submenu to the right side of main menu with 2px gap
                    let submenu_y = menu_pos.y + audio_btn.rect.min.y;
                    state.submenu_position = Some(egui::pos2(menu_pos.x + MENU_WIDTH + 2.0, submenu_y));
                }

                let sub_btn = ui.add(egui::Button::new("💬 Legendas ›").frame(false));
                if sub_btn.hovered() {
                    state.active_submenu = Some("subtitle".to_string());
                    // Position submenu to the right side of main menu with 2px gap
                    let submenu_y = menu_pos.y + sub_btn.rect.min.y;
                    state.submenu_position = Some(egui::pos2(menu_pos.x + MENU_WIDTH + 2.0, submenu_y));
                }

                ui.separator();

                if ui.button("Fechar menu").clicked() {
                    action = VideoMenuAction::Close;
                }

                if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    action = VideoMenuAction::Close;
                }
                
                // Store the menu rect for click-outside detection
                state.main_menu_rect = Some(egui::Rect::from_min_size(menu_pos, ui.min_size()));
            });
        },
    );

    // --- SUBMENU VIEWPORT (rendered separately, not nested) ---
    // Only render if menu is still open and we have a submenu to show
    let mut submenu_was_rendered = false;
    if action == VideoMenuAction::None {
        if let (Some(submenu), Some(pos)) = (submenu_to_render, submenu_pos) {
            submenu_was_rendered = true;
            let submenu_id = egui::ViewportId::from_hash_of(format!("video_submenu_{}", submenu));
            
            ctx.show_viewport_immediate(
                submenu_id,
                egui::ViewportBuilder::default()
                    .with_title("Submenu")
                    .with_decorations(false)
                    .with_always_on_top()
                    .with_visible(true)
                    .with_taskbar(false)
                    .with_transparent(true)
                    .with_inner_size([MENU_WIDTH, 300.0])
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
                        
                        // Store submenu rect for click-outside detection
                        state.submenu_rect = Some(egui::Rect::from_min_size(pos, ui.min_size()));
                    });
                },
            );
        }
    }
    
    // Clear submenu rect if no submenu rendered
    if !submenu_was_rendered {
        state.submenu_rect = None;
    }
    
    // --- CLICK OUTSIDE DETECTION ---
    // Check if user clicked outside both menus
    if action == VideoMenuAction::None {
        let pointer_pos = ctx.input(|i| i.pointer.latest_pos());
        let clicked = ctx.input(|i| i.pointer.any_click());
        
        if clicked {
            if let Some(pos) = pointer_pos {
                let inside_main = state.main_menu_rect.map(|r| r.contains(pos)).unwrap_or(false);
                let inside_submenu = state.submenu_rect.map(|r| r.contains(pos)).unwrap_or(false);
                
                if !inside_main && !inside_submenu {
                    action = VideoMenuAction::Close;
                }
            }
        }
    }
    
    // Handle closing logic
    if action == VideoMenuAction::Close {
        // Close everything at once
        state.active_submenu = None;
        state.submenu_position = None;
        state.main_menu_rect = None;
        state.submenu_rect = None;
        state.is_open = false;
    } else if matches!(action, VideoMenuAction::SetAudioTrack(_)) || matches!(action, VideoMenuAction::SetSubtitleTrack(_)) {
        // Close everything when a selection is made
        state.active_submenu = None;
        state.submenu_position = None;
        state.main_menu_rect = None;
        state.submenu_rect = None;
        state.is_open = false;
    }

    action
}
