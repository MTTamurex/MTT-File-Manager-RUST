use eframe::egui;
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use crate::app::ImageViewerApp;

pub fn handle_input(app: &mut ImageViewerApp, ctx: &egui::Context) {
    if app.renaming_state.is_none() && !app.is_address_editing {
        // Handle media hardware input first (overrides normal navigation when player focused)
        if handle_media_hardware_input(app, ctx) {
            return;
        }

        // Detectar teclas através dos eventos (Key events)
        let mut do_copy = false;
        let mut do_cut = false;
        let mut do_paste = false;

        ctx.input(|i| {
            // INTERACTION MODE DETECTION
            // 1. Mouse detection (ONLY intentional actions: Click or Press)
            // CRITICAL: Do NOT detect passive mouse movement (delta) to avoid interfering with keyboard navigation
            if i.pointer.any_pressed() || i.pointer.any_click() {
                app.last_input = crate::app::state::LastInput::Mouse;
            }

            for event in &i.events {
                match event {
                    egui::Event::Key {
                        key,
                        pressed,
                        modifiers,
                        ..
                    } => {
                        // 2. Keyboard detection (Navigation keys)
                        if *pressed {
                            match key {
                                egui::Key::ArrowDown | egui::Key::ArrowUp | 
                                egui::Key::ArrowLeft | egui::Key::ArrowRight |
                                egui::Key::PageDown | egui::Key::PageUp |
                                egui::Key::Home | egui::Key::End => {
                                    app.last_input = crate::app::state::LastInput::Keyboard;
                                }
                                _ => {}
                            }
                        }

                        if *pressed && modifiers.ctrl {
                            match key {
                                egui::Key::C => do_copy = true,
                                egui::Key::X => do_cut = true,
                                egui::Key::V => do_paste = true,
                                // TAB MANAGEMENT SHORTCUTS
                                egui::Key::T => {
                                    app.sync_to_tab();
                                    app.tab_manager.new_tab();
                                    app.sync_from_tab();
                                    app.setup_computer_view();
                                    app.sync_to_tab();
                                    app.update_video_visibility();
                                }
                                egui::Key::W => {
                                    if app.tab_manager.close_active_tab() {
                                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                                    } else {
                                        app.sync_from_tab();
                                        app.update_video_visibility();
                                    }
                                }
                                egui::Key::Tab => {
                                    app.sync_to_tab();
                                    if modifiers.shift {
                                        app.tab_manager.prev_tab();
                                    } else {
                                        app.tab_manager.next_tab();
                                    }
                                    app.sync_from_tab();
                                    app.update_video_visibility();
                                }
                                _ => {}
                            }
                        }
                    }
                    egui::Event::Copy => do_copy = true,
                    egui::Event::Cut => do_cut = true,
                    egui::Event::Paste(_) => do_paste = true,
                    _ => {}
                }
            }
        });

        // Fallback: use Windows GetAsyncKeyState for hardware-level detection
        let ctrl_down = unsafe { GetAsyncKeyState(0x11) < 0 };
        let v_down = unsafe { GetAsyncKeyState(0x56) < 0 };

        // Debounced paste detection
        if ctrl_down && v_down && !app.paste_key_debounce {
            do_paste = true;
            app.paste_key_debounce = true;
        } else if !v_down {
            app.paste_key_debounce = false;
        }

        // Executar ações de clipboard
        if do_copy { app.command_copy(None); }
        if do_cut { app.command_cut(None); }
        if do_paste { app.command_paste(None); }

        // Delete: Excluir
        if ctx.input(|i| i.key_pressed(egui::Key::Delete)) {
            app.delete_with_shell_for_idx(None);
        }

        // Ctrl + Shift + N: Nova Pasta
        if ctx.input(|i| i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(egui::Key::N)) {
            app.create_new_folder();
        }
    } else {
        // Durante renomeação: ESC cancela a operação
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            app.renaming_state = None;
            app.focus_rename = false;
        }
    }
}

/// Handle resize borders for borderless window.
/// 
/// NOTE: This is now a no-op. Resize hit-testing is handled by the native
/// WM_NCHITTEST subclass in `infrastructure::windows::window_subclass`.
/// Keeping this function signature for compatibility but removing the
/// expensive egui Area overlays that were created every frame.
#[allow(dead_code)]
pub fn handle_resize_borders(_app: &mut ImageViewerApp, _ctx: &egui::Context) {
    // Resize borders are now handled natively via WM_NCHITTEST subclass
    // See: infrastructure/windows/window_subclass.rs
    //
    // The native approach:
    // 1. Has zero per-frame cost (no egui widgets)
    // 2. Provides proper Windows resize cursors automatically
    // 3. Integrates with Windows DWM for smooth resize
}

fn handle_media_hardware_input(app: &mut ImageViewerApp, ctx: &egui::Context) -> bool {
    // Check focus and mode first
    if !app.is_media_keyboard_focused() {
        return false;
    }

    let preview = if let Some(p) = &mut app.media_preview { p } else { return false; };
    
    // Condition 3: Debounce (200ms)
    if app.last_media_key_press.elapsed() < std::time::Duration::from_millis(200) {
        return false;
    }

    // Detect keys via hardware-level check (AsyncKeyState)
    // We check bits to ensure the key WAS pressed since last check
    let mut consumed = false;
    unsafe {
        // VK_SPACE = 0x20, VK_UP = 0x26, VK_DOWN = 0x28, VK_RIGHT = 0x27, VK_LEFT = 0x25
        if (GetAsyncKeyState(0x20) as u16 & 0x8000) != 0 { 
            preview.toggle_play();
            consumed = true;
        } else if (GetAsyncKeyState(0x26) as u16 & 0x8000) != 0 { 
            let vol = preview.get_video_state().map(|s| s.volume).unwrap_or(1.0);
            preview.set_volume((vol + 0.05).min(1.0));
            consumed = true;
        } else if (GetAsyncKeyState(0x28) as u16 & 0x8000) != 0 { 
            let vol = preview.get_video_state().map(|s| s.volume).unwrap_or(1.0);
            preview.set_volume((vol - 0.05).max(0.0));
            consumed = true;
        } else if (GetAsyncKeyState(0x27) as u16 & 0x8000) != 0 { 
            preview.seek_relative(5.0);
            consumed = true;
        } else if (GetAsyncKeyState(0x25) as u16 & 0x8000) != 0 { 
            preview.seek_relative(-5.0);
            consumed = true;
        }
    }

    if consumed {
        app.last_media_key_press = std::time::Instant::now();
        ctx.request_repaint();
    }
    consumed
}
