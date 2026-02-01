use crate::ui::components::mpv_preview::TrackInfo;
use eframe::egui;
use std::time::Instant;

const MENU_WIDTH: f32 = 160.0;
const SUBMENU_WIDTH: f32 = 200.0;

#[derive(Clone)]
pub struct VideoMenuState {
    pub is_open: bool,
    pub position: egui::Pos2,
    pub active_submenu: Option<String>, // "audio" or "subtitle"
    pub submenu_position: Option<egui::Pos2>,
    pub main_menu_rect: Option<egui::Rect>,
    pub submenu_rect: Option<egui::Rect>,
    /// Time when menu was opened - to ignore the click that opened it
    pub menu_opened_at: Option<Instant>,
}

impl Default for VideoMenuState {
    fn default() -> Self {
        Self {
            is_open: false,
            position: egui::Pos2::ZERO,
            active_submenu: None,
            submenu_position: None,
            main_menu_rect: None,
            submenu_rect: None,
            menu_opened_at: None,
        }
    }
}

#[derive(PartialEq)]
pub enum VideoMenuAction {
    None,
    TogglePlay,
    ToggleMute,
    ToggleAudioNormalizer,
    SetAudioTrack(i64),
    SetSubtitleTrack(i64),
    ToggleFullscreen,
    Close,
    /// Menu was closed by right-click outside - caller should reopen at new position
    RightClickOutside(egui::Pos2),
}

/// Helper to create a menu item with arrow aligned to the right
fn menu_item(ui: &mut egui::Ui, text: &str, has_submenu: bool) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::click());

    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact_selectable(&response, false);

        // Draw background on hover
        if response.hovered() {
            ui.painter().rect_filled(rect, 0.0, visuals.bg_fill);
        }

        // Draw text on the left
        ui.painter().text(
            rect.left_center() + egui::vec2(4.0, 0.0),
            egui::Align2::LEFT_CENTER,
            text,
            egui::FontId::default(),
            visuals.text_color(),
        );

        // Draw arrow on the right if has submenu
        if has_submenu {
            ui.painter().text(
                rect.right_center() - egui::vec2(8.0, 0.0),
                egui::Align2::RIGHT_CENTER,
                "›",
                egui::FontId::default(),
                visuals.text_color(),
            );
        }
    }

    response
}

/// Helper to create a submenu item with checkmark for selection
fn submenu_item(ui: &mut egui::Ui, text: &str, is_selected: bool) -> egui::Response {
    let display_text = if is_selected {
        format!("✓ {}", text)
    } else {
        format!("   {}", text)
    };
    ui.add(egui::SelectableLabel::new(false, display_text))
}

pub fn render_video_menu(
    ctx: &egui::Context,
    state: &mut VideoMenuState,
    audio_tracks: &[TrackInfo],
    sub_tracks: &[TrackInfo],
    is_fullscreen: bool,
    audio_normalizer_enabled: bool,
) -> VideoMenuAction {
    let mut action = VideoMenuAction::None;

    if !state.is_open {
        // Menu is closed - just clear state, NO rendering needed
        state.active_submenu = None;
        state.submenu_position = None;
        state.main_menu_rect = None;
        state.submenu_rect = None;
        return action;
    }

    // Custom frame for menus - solid background (no shadow for performance)
    let menu_frame = egui::Frame::new()
        .fill(ctx.style().visuals.window_fill)
        .stroke(ctx.style().visuals.window_stroke)
        .inner_margin(egui::Margin::ZERO) // No margin to ensure background fills the viewport edge
        .corner_radius(0.0);

    let menu_pos = state.position;

    // --- MAIN MENU using native viewport (appears above MPV HWND) ---
    let viewport_id = egui::ViewportId::from_hash_of("video_context_menu");
    
    ctx.show_viewport_immediate(
        viewport_id,
        egui::ViewportBuilder::default()
            .with_title("Video Menu")
            .with_decorations(false)
            .with_visible(true)
            .with_taskbar(false)
            .with_always_on_top()
            .with_resizable(false)
            .with_inner_size([MENU_WIDTH, 132.0]) // Slightly larger to avoid rounding/clipping issues
            .with_position(menu_pos),
        |ctx, _class| {
            egui::CentralPanel::default().frame(menu_frame).show(ctx, |ui| {
                // Audio menu item
                let audio_resp = menu_item(ui, "🔊 Áudio", true);
                if audio_resp.hovered() {
                    state.active_submenu = Some("audio".to_string());
                    let submenu_y = menu_pos.y + audio_resp.rect.min.y;
                    state.submenu_position = Some(egui::pos2(menu_pos.x + MENU_WIDTH, submenu_y));
                }

                let normalizer_text = if audio_normalizer_enabled {
                    "✓ Normalizar áudio"
                } else {
                    "Normalizar áudio"
                };
                if menu_item(ui, normalizer_text, false).clicked() {
                    action = VideoMenuAction::ToggleAudioNormalizer;
                }

                // Subtitle menu item
                let sub_resp = menu_item(ui, "💬 Legendas", true);
                if sub_resp.hovered() {
                    state.active_submenu = Some("subtitle".to_string());
                    let submenu_y = menu_pos.y + sub_resp.rect.min.y;
                    state.submenu_position = Some(egui::pos2(menu_pos.x + MENU_WIDTH, submenu_y));
                }

                ui.separator();

                // Fullscreen/Restore option
                let fs_text = if is_fullscreen { "⮌ Restaurar janela" } else { "⛶ Tela cheia" };
                if menu_item(ui, fs_text, false).clicked() {
                    action = VideoMenuAction::ToggleFullscreen;
                }

                // ESC to close
                if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    action = VideoMenuAction::Close;
                }
                
                // Store the menu rect for click-outside detection
                state.main_menu_rect = Some(egui::Rect::from_min_size(menu_pos, ui.min_size()));
            });
        },
    );

    // --- SUBMENU using native viewport ---
    let mut submenu_was_rendered = false;
    let submenu_to_render = state.active_submenu.clone();
    let submenu_pos = state.submenu_position;
    
    if action == VideoMenuAction::None {
        if let (Some(submenu), Some(pos)) = (submenu_to_render, submenu_pos) {
            submenu_was_rendered = true;
            let submenu_id = egui::ViewportId::from_hash_of(format!("video_submenu_{}", submenu));
            
            ctx.show_viewport_immediate(
                submenu_id,
                egui::ViewportBuilder::default()
                    .with_title("Submenu")
                    .with_decorations(false)
                    .with_visible(true)
                    .with_taskbar(false)
                    .with_always_on_top()
                    .with_resizable(false)
                    .with_inner_size([SUBMENU_WIDTH, 350.0])
                    .with_position(pos),
                |ctx, _class| {
                    egui::CentralPanel::default().frame(menu_frame).show(ctx, |ui| {
                        egui::ScrollArea::vertical().max_height(330.0).show(ui, |ui| {
                            match submenu.as_str() {
                                "audio" => {
                                    if audio_tracks.is_empty() {
                                        ui.label("Nenhuma faixa de áudio");
                                    } else {
                                        for track in audio_tracks {
                                            let label = track.title.as_deref().unwrap_or("Faixa sem título");
                                            let lang = track.lang.as_deref().unwrap_or("unk");
                                            let text = format!("{} ({})", label, lang);
                                            
                                            if submenu_item(ui, &text, track.selected).clicked() {
                                                action = VideoMenuAction::SetAudioTrack(track.id);
                                            }
                                        }
                                    }
                                }
                                "subtitle" => {
                                    // "Disable subtitles" option
                                    let no_sub_selected = sub_tracks.iter().all(|t| !t.selected);
                                    if submenu_item(ui, "Desativar legendas", no_sub_selected).clicked() {
                                        action = VideoMenuAction::SetSubtitleTrack(0);
                                    }

                                    if !sub_tracks.is_empty() {
                                        ui.separator();
                                        for track in sub_tracks {
                                            let label = track.title.as_deref().unwrap_or("Legenda");
                                            let lang = track.lang.as_deref().unwrap_or("unk");
                                            let text = format!("{} ({})", label, lang);
                                            
                                            if submenu_item(ui, &text, track.selected).clicked() {
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
    // Skip detection for the first 100ms after menu opens (to avoid closing from the right-click that opened it)
    let should_check_click = state
        .menu_opened_at
        .map(|t| t.elapsed().as_millis() > 100)
        .unwrap_or(true);

    if matches!(action, VideoMenuAction::None) && should_check_click {
        let pointer_pos = ctx.input(|i| i.pointer.latest_pos());
        let left_clicked = ctx.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary));
        let right_clicked = ctx.input(|i| i.pointer.button_clicked(egui::PointerButton::Secondary));

        if let Some(pos) = pointer_pos {
            let inside_main = state
                .main_menu_rect
                .map(|r| r.contains(pos))
                .unwrap_or(false);
            let inside_submenu = state
                .submenu_rect
                .map(|r| r.contains(pos))
                .unwrap_or(false);

            if !inside_main && !inside_submenu {
                if right_clicked {
                    // Right-click outside - signal to reopen at new position
                    action = VideoMenuAction::RightClickOutside(pos);
                } else if left_clicked {
                    // Left-click outside - just close
                    action = VideoMenuAction::Close;
                }
            }
        }
    }

    // Handle closing logic
    if matches!(
        action,
        VideoMenuAction::Close
            | VideoMenuAction::ToggleFullscreen
            | VideoMenuAction::ToggleAudioNormalizer
            | VideoMenuAction::RightClickOutside(_)
    ) {
        state.active_submenu = None;
        state.submenu_position = None;
        state.main_menu_rect = None;
        state.submenu_rect = None;
        state.menu_opened_at = None;
        state.is_open = false;
    } else if matches!(action, VideoMenuAction::SetAudioTrack(_))
        || matches!(action, VideoMenuAction::SetSubtitleTrack(_))
    {
        // Close everything when a selection is made
        state.active_submenu = None;
        state.submenu_position = None;
        state.main_menu_rect = None;
        state.submenu_rect = None;
        state.menu_opened_at = None;
        state.is_open = false;
    }

    action
}
