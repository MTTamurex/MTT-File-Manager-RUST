use crate::app::ImageViewerApp;
use crate::domain::file_entry::ViewMode;
use crate::workers::idle_warmup::IdleWarmupMessage;
use eframe::egui;
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

fn handle_space_preview_action(app: &mut ImageViewerApp, ctx: &egui::Context) -> bool {
    // Ignore while typing/editing any text input context
    if app.renaming_state.is_some()
        || app.sidebar_renaming.is_some()
        || app.is_address_editing
        || app.global_search.active
        || ctx.wants_keyboard_input()
    {
        return false;
    }

    let space_pressed = ctx.input(|i| {
        i.key_pressed(egui::Key::Space)
            && !i.modifiers.ctrl
            && !i.modifiers.alt
            && !i.modifiers.shift
            && !i.modifiers.mac_cmd
            && !i.modifiers.command
    });

    if !space_pressed {
        return false;
    }

    app.trigger_selected_preview_overlay_action()
}

pub fn handle_input(app: &mut ImageViewerApp, ctx: &egui::Context) {
    let mut user_active = false;
    if app.renaming_state.is_none() && app.sidebar_renaming.is_none() && !app.is_address_editing {
        // Handle media hardware input first (overrides normal navigation when player focused)
        if handle_media_hardware_input(app, ctx) {
            return;
        }

        // While the global search modal is open, keep focus/input inside it.
        // Prevent routing shortcuts/quick-search to the main file views.
        if app.global_search.active {
            if ctx.input(|i| i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(egui::Key::F)) {
                app.global_search.active = false;
                app.global_search.focus_request = false;
                app.global_search.pending_query_dispatch_at = None;
                app.global_search.size_cache.clear();
                app.global_search.tooltip_texture_cache.clear();
                app.global_search.metadata_cache.clear();
                user_active = true;
            }

            if !user_active {
                user_active = ctx.input(|i| {
                    i.pointer.any_pressed()
                        || i.pointer.any_click()
                        || i.raw_scroll_delta != egui::Vec2::ZERO
                        || !i.events.is_empty()
                });
            }

            if user_active {
                let _ = app
                    .file_operation_state
                    .idle_warmup_sender
                    .send(IdleWarmupMessage::UserActive);
            }
            return;
        }
        // Detect key events
        let mut do_copy = false;
        let mut do_cut = false;
        let mut do_paste = false;
        let text_input_active = ctx.wants_keyboard_input();

        ctx.input(|i| {
            // INTERACTION MODE DETECTION
            // 1. Mouse detection (ONLY intentional actions: Click or Press)
            // CRITICAL: Do NOT detect passive mouse movement (delta) to avoid interfering with keyboard navigation
            if i.pointer.any_pressed() || i.pointer.any_click() {
                app.last_input = crate::app::state::LastInput::Mouse;
                user_active = true;
            }
            if i.raw_scroll_delta != egui::Vec2::ZERO {
                user_active = true;
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
                            user_active = true;
                            match key {
                                egui::Key::ArrowDown
                                | egui::Key::ArrowUp
                                | egui::Key::ArrowLeft
                                | egui::Key::ArrowRight
                                | egui::Key::PageDown
                                | egui::Key::PageUp
                                | egui::Key::Home
                                | egui::Key::End => {
                                    app.last_input = crate::app::state::LastInput::Keyboard;
                                }
                                _ => {}
                            }
                        }

                        if *pressed && modifiers.ctrl {
                            match key {
                                egui::Key::C if !text_input_active => do_copy = true,
                                egui::Key::X if !text_input_active => do_cut = true,
                                egui::Key::V if !text_input_active => do_paste = true,
                                // TAB MANAGEMENT SHORTCUTS
                                egui::Key::T => {
                                    let prev_view_mode = app.view_mode;
                                    let prev_sort_mode = app.sort_mode;
                                    let prev_sort_descending = app.sort_descending;
                                    let prev_folders_position = app.folders_position;
                                    app.sync_to_tab();
                                    app.tab_manager.new_tab();
                                    let active = app.tab_manager.active_mut();
                                    active.view_mode = prev_view_mode;
                                    active.sort_mode = prev_sort_mode;
                                    active.sort_descending = prev_sort_descending;
                                    active.folders_position = prev_folders_position;
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
                    egui::Event::Copy if !text_input_active => {
                        do_copy = true;
                        user_active = true;
                    }
                    egui::Event::Cut if !text_input_active => {
                        do_cut = true;
                        user_active = true;
                    }
                    egui::Event::Paste(_) if !text_input_active => {
                        do_paste = true;
                        user_active = true;
                    }
                    _ => {}
                }
            }
        });

        // Fallback: use Windows GetAsyncKeyState for hardware-level detection.
        // CRITICAL: Only process when our window has OS focus — GetAsyncKeyState
        // reads global hardware state and would capture keystrokes from other apps.
        let app_has_focus = ctx.input(|i| i.viewport().focused.unwrap_or(false));
        if app_has_focus {
            let ctrl_down = unsafe { GetAsyncKeyState(0x11) < 0 };
            let v_down = unsafe { GetAsyncKeyState(0x56) < 0 };
            let shift_down = unsafe { GetAsyncKeyState(0x10) < 0 };
            let del_down = unsafe { GetAsyncKeyState(0x2E) < 0 };

            // Debounced paste detection
            if ctrl_down && v_down && !app.paste_key_debounce && !text_input_active {
                do_paste = true;
                app.paste_key_debounce = true;
            } else if !v_down {
                app.paste_key_debounce = false;
            }

            // Debounced Shift+Delete detection (permanente)
            if shift_down && del_down && !app.delete_key_debounce && !text_input_active {
                app.delete_permanently_for_idx(None);
                app.delete_key_debounce = true;
                user_active = true;
            } else if !del_down {
                app.delete_key_debounce = false;
            }
        } else {
            // Reset debounce state when unfocused to avoid stale triggers on refocus.
            app.paste_key_debounce = false;
            app.delete_key_debounce = false;
        }

        // Execute clipboard actions
        if do_copy {
            app.command_copy(None);
        }
        if do_cut {
            app.command_cut(None);
        }
        if do_paste {
            app.command_paste(None);
        }

        // Delete: Excluir para lixeira (somente sem Shift)
        // Skip when a text input (search bar, address bar) has keyboard focus.
        if !text_input_active
            && ctx.input(|i| !i.modifiers.shift && i.key_pressed(egui::Key::Delete))
        {
            app.delete_with_shell_for_idx(None);
            user_active = true;
        }

        // Shift+Delete: tratado via GetAsyncKeyState abaixo (egui não entrega confiavelmente)

        // Alt+Enter: Propriedades
        // Skip when a text input has keyboard focus.
        if !text_input_active
            && ctx.input(|i| i.modifiers.alt && i.key_pressed(egui::Key::Enter))
        {
            app.show_properties_for_idx(None);
            user_active = true;
        }

        // Ctrl+L: Focar barra de endereços
        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::L)) {
            app.navigation_state.path_input = app.navigation_state.current_path.clone();
            app.is_address_editing = true;
            app.show_address_history_menu = false;
            app.address_bar_focus_request = true;
            user_active = true;
        }

        // Ctrl+Scroll: Ajustar tamanho dos thumbnails (somente no modo Grade)
        // eframe/winit converte Ctrl+Scroll em zoom_delta antes do smooth_scroll_delta
        // Lemos zoom_delta e resetamos o fator de zoom da UI de volta a 1.0
        let zoom_delta = ctx.input(|i| i.zoom_delta());
        if (zoom_delta - 1.0).abs() > 0.001 && app.view_mode == ViewMode::Grid {
            // zoom_delta > 1.0 = scroll para cima (aumentar), < 1.0 = diminuir
            // Escala: cada entalhe da roda gera delta ~0.1 → 0.1 × 24 = ~2.4px por entalhe
            let change = (zoom_delta - 1.0) * 24.0;
            app.thumbnail_size = (app.thumbnail_size + change).clamp(64.0, 256.0);
            // Impede que o egui aplique o zoom à própria UI
            ctx.set_zoom_factor(1.0);
            app.save_preferences();
            user_active = true;
        }

        // Ctrl + Shift + N: Nova Pasta
        if ctx.input(|i| i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(egui::Key::N))
            && !app.navigation_state.is_computer_view
            && !app.navigation_state.is_recycle_bin_view
        {
            app.create_new_folder();
            user_active = true;
        }

        // Ctrl + Shift + F: Global Search
        if ctx.input(|i| i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(egui::Key::F)) {
            app.global_search.active = !app.global_search.active;
            app.global_search.selected_index = None;
            if app.global_search.active {
                app.global_search.focus_request = true;
                app.global_search.query.clear();
                app.global_search.results.clear();
                app.global_search.loading = false;
                app.global_search.pending_query_dispatch_at = None;
                app.global_search.has_more_results = false;
                app.global_search.requested_offset = 0;
                app.global_search.requested_limit = 200;
                // Check service availability
                if let Err(e) = app
                    .global_search
                    .sender
                    .send(crate::workers::global_search_worker::GlobalSearchRequest::CheckStatus)
                {
                    log::error!("[GLOBAL-SEARCH] Failed to queue status check: {}", e);
                }
            } else {
                app.global_search.focus_request = false;
                app.global_search.pending_query_dispatch_at = None;
                app.global_search.has_more_results = false;
                app.global_search.requested_offset = 0;
                app.global_search.requested_limit = 200;
            }
            user_active = true;
        }

        // Space: same behavior as preview overlay action for selected media/image/pdf
        let consumed_space_preview_action = handle_space_preview_action(app, ctx);
        if consumed_space_preview_action {
            user_active = true;
        }

        // QUICK SEARCH: Type-to-search like Explorer
        if !consumed_space_preview_action {
            handle_quick_search(app, ctx);
        }
    } else {
        // During rename: ESC cancels the operation
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            app.renaming_state = None;
            app.focus_rename = false;
            user_active = true;
        }
    }
    if user_active {
        let _ = app
            .file_operation_state
            .idle_warmup_sender
            .send(IdleWarmupMessage::UserActive);
    }
}

fn handle_media_hardware_input(app: &mut ImageViewerApp, ctx: &egui::Context) -> bool {
    // Check focus and mode first
    if !app.is_media_keyboard_focused() {
        return false;
    }

    let preview = if let Some(p) = &mut app.media_preview {
        p
    } else {
        return false;
    };

    // Condition 3: Debounce (200ms)
    if app.last_media_key_press.elapsed() < std::time::Duration::from_millis(200) {
        return false;
    }

    // Detect keys via hardware-level check (AsyncKeyState).
    // Only when our window has OS focus — GetAsyncKeyState reads global state.
    let app_has_focus = ctx.input(|i| i.viewport().focused.unwrap_or(false));
    if !app_has_focus {
        return false;
    }

    let mut consumed = false;
    let mut new_session_vol: Option<f32> = None;
    unsafe {
        // VK_SPACE = 0x20, VK_UP = 0x26, VK_DOWN = 0x28, VK_RIGHT = 0x27, VK_LEFT = 0x25
        // VK_CTRL = 0x11, VK_SHIFT = 0x10, VK_U = 0x55, VK_A = 0x41
        let ctrl_down = (GetAsyncKeyState(0x11) as u16 & 0x8000) != 0;
        let shift_down = (GetAsyncKeyState(0x10) as u16 & 0x8000) != 0;
        let u_down = (GetAsyncKeyState(0x55) as u16 & 0x8000) != 0;
        let a_down = (GetAsyncKeyState(0x41) as u16 & 0x8000) != 0;

        if ctrl_down && shift_down && u_down {
            match preview.toggle_vsr() {
                Ok(_) => {
                    let vsr_on = preview.is_vsr_enabled();
                    let msg = if vsr_on {
                        "NVIDIA VSR: ON"
                    } else {
                        "NVIDIA VSR: OFF"
                    };
                    preview.show_osd(msg, 2000);
                    consumed = true;
                }
                Err(e) => {
                    log::error!("toggling VSR (Ctrl+Shift+U): {}", e);
                }
            }
        } else if ctrl_down && shift_down && a_down {
            preview.toggle_audio_normalizer();
            let normalizer_on = preview.is_audio_normalizer_enabled();
            let msg = if normalizer_on {
                "Audio Normalizer: ON"
            } else {
                "Audio Normalizer: OFF"
            };
            preview.show_osd(msg, 2000);
            consumed = true;
        } else if (GetAsyncKeyState(0x20) as u16 & 0x8000) != 0 {
            preview.toggle_play();
            consumed = true;
        } else if (GetAsyncKeyState(0x26) as u16 & 0x8000) != 0 {
            let vol = preview.get_video_state().map(|s| s.volume).unwrap_or(1.0);
            let new_vol = (vol + 0.05).min(1.0);
            preview.set_volume(new_vol);
            new_session_vol = Some(new_vol);
            let msg = format!("Volume: {}%", (new_vol * 100.0).round() as i32);
            preview.show_osd(&msg, 2000);
            consumed = true;
        } else if (GetAsyncKeyState(0x28) as u16 & 0x8000) != 0 {
            let vol = preview.get_video_state().map(|s| s.volume).unwrap_or(1.0);
            let new_vol = (vol - 0.05).max(0.0);
            preview.set_volume(new_vol);
            new_session_vol = Some(new_vol);
            let msg = format!("Volume: {}%", (new_vol * 100.0).round() as i32);
            preview.show_osd(&msg, 2000);
            consumed = true;
        } else if (GetAsyncKeyState(0x27) as u16 & 0x8000) != 0 {
            let state = preview.get_video_state();
            let current = state.as_ref().map(|s| s.current_time).unwrap_or(0.0);
            let duration = state.as_ref().map(|s| s.duration).unwrap_or(0.0);
            preview.seek_relative(5.0);
            let new_time = if duration > 0.0 {
                (current + 5.0).min(duration)
            } else {
                current + 5.0
            };
            let msg = if duration > 0.0 {
                format!(
                    "{} / {}",
                    crate::ui::components::media_preview::format_time(new_time),
                    crate::ui::components::media_preview::format_time(duration)
                )
            } else {
                crate::ui::components::media_preview::format_time(new_time)
            };
            preview.show_osd(&msg, 2000);
            consumed = true;
        } else if (GetAsyncKeyState(0x25) as u16 & 0x8000) != 0 {
            let state = preview.get_video_state();
            let current = state.as_ref().map(|s| s.current_time).unwrap_or(0.0);
            let duration = state.as_ref().map(|s| s.duration).unwrap_or(0.0);
            preview.seek_relative(-5.0);
            let new_time = (current - 5.0).max(0.0);
            let msg = if duration > 0.0 {
                format!(
                    "{} / {}",
                    crate::ui::components::media_preview::format_time(new_time),
                    crate::ui::components::media_preview::format_time(duration)
                )
            } else {
                crate::ui::components::media_preview::format_time(new_time)
            };
            preview.show_osd(&msg, 2000);
            consumed = true;
        }
    }

    if let Some(vol) = new_session_vol {
        app.session_volume = vol;
    }
    if consumed {
        app.last_media_key_press = std::time::Instant::now();
        ctx.request_repaint();
    }
    consumed
}

/// Handle quick search (type-to-search like Explorer)
///
/// Captures alphanumeric keys and searches for matching items in the current folder.
/// Buffer is cleared after 1.5 seconds of inactivity.
/// Each tab has its own independent search buffer.
fn handle_quick_search(app: &mut ImageViewerApp, ctx: &egui::Context) {
    const QUICK_SEARCH_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1500);

    // Get active tab's quick search state
    let (buffer_is_empty, timeout_elapsed) = {
        let active_tab = app.tab_manager.active();
        (
            active_tab.quick_search_buffer.is_empty(),
            active_tab.quick_search_last_input.elapsed() > QUICK_SEARCH_TIMEOUT,
        )
    };

    // Clear buffer if timeout elapsed
    if timeout_elapsed && !buffer_is_empty {
        let active_tab = app.tab_manager.active_mut();
        active_tab.quick_search_buffer.clear();
        log::debug!(
            "[QUICK_SEARCH] Buffer cleared due to timeout (Tab {})",
            active_tab.id
        );
    }

    // Capture text input events (alphanumeric, space, etc.)
    let text_input = ctx.input(|i| {
        i.events.iter().find_map(|event| {
            if let egui::Event::Text(text) = event {
                // Filter out control characters
                let filtered: String = text.chars().filter(|c| !c.is_control()).collect();
                if !filtered.is_empty() {
                    return Some(filtered);
                }
            }
            None
        })
    });

    // Handle backspace separately
    let backspace_pressed = ctx.input(|i| i.key_pressed(egui::Key::Backspace));

    if backspace_pressed {
        let active_tab = app.tab_manager.active_mut();
        if !active_tab.quick_search_buffer.is_empty() {
            active_tab.quick_search_buffer.pop();
            active_tab.quick_search_last_input = std::time::Instant::now();
            log::debug!(
                "[QUICK_SEARCH] Backspace - Buffer: '{}' (Tab {})",
                active_tab.quick_search_buffer,
                active_tab.id
            );

            if !active_tab.quick_search_buffer.is_empty() {
                perform_quick_search(app);
            }
        }
    } else if let Some(text) = text_input {
        let active_tab = app.tab_manager.active_mut();
        active_tab.quick_search_buffer.push_str(&text);
        active_tab.quick_search_last_input = std::time::Instant::now();
        log::debug!(
            "[QUICK_SEARCH] Input: '{}' - Buffer: '{}' (Tab {})",
            text,
            active_tab.quick_search_buffer,
            active_tab.id
        );

        perform_quick_search(app);
    }
}

/// Find and scroll to the first item matching the search buffer
fn perform_quick_search(app: &mut ImageViewerApp) {
    let search_lower = app.tab_manager.active().quick_search_buffer.to_lowercase();

    if search_lower.is_empty() {
        return;
    }

    // Search in current items
    let found_index = app
        .items
        .iter()
        .position(|item| item.name.to_lowercase().starts_with(&search_lower));

    if let Some(index) = found_index {
        let tab_id = app.tab_manager.active().id;
        log::debug!(
            "[QUICK_SEARCH] Found match at index {} - '{}' (Tab {})",
            index,
            app.items[index].name,
            tab_id
        );

        // Update selection
        app.selected_item = Some(index);
        app.selected_file = Some(app.items[index].clone());

        // Clear multi-selection and add selected item (shows dark blue border)
        app.multi_selection.clear();
        app.multi_selection.insert(app.items[index].path.clone());

        // Update selection anchor for shift+click support
        app.selection_anchor = Some(index);

        // Trigger scroll to selected item
        app.scroll_to_selected = true;

        // Mark keyboard as last input (strict hover control)
        app.last_input = crate::app::state::LastInput::Keyboard;
    } else {
        let tab_id = app.tab_manager.active().id;
        log::debug!(
            "[QUICK_SEARCH] No match found for '{}' (Tab {})",
            search_lower,
            tab_id
        );
    }
}
