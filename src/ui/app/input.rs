use crate::app::shortcuts::{ShortcutAction, ShortcutBinding};
use crate::app::ImageViewerApp;
use crate::domain::file_entry::ViewMode;
use crate::infrastructure::windows::is_virtual_key_down;
use crate::ui::address_bar;
use crate::workers::idle_warmup::IdleWarmupMessage;
use eframe::egui;

fn handle_preview_shortcut_action(app: &mut ImageViewerApp, ctx: &egui::Context) -> bool {
    // Ignore while typing/editing any text input context
    if app.renaming_state.is_some()
        || app.sidebar_renaming.is_some()
        || app.is_address_editing
        || app.global_search.active
        || ctx.wants_keyboard_input()
    {
        return false;
    }

    if !app
        .shortcuts
        .is_triggered(ShortcutAction::PreviewSelected, ctx)
    {
        return false;
    }

    if !app.should_consume_space_for_selected_preview_overlay_action() {
        return false;
    }

    let _ = app.trigger_selected_preview_overlay_action();
    true
}

fn create_new_tab(app: &mut ImageViewerApp) {
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

fn close_current_tab(app: &mut ImageViewerApp, ctx: &egui::Context) {
    if app.tab_manager.close_active_tab() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    } else {
        app.sync_from_tab();
        app.update_video_visibility();
    }
}

fn switch_tab(app: &mut ImageViewerApp, previous: bool) {
    app.sync_to_tab();
    if previous {
        app.tab_manager.prev_tab();
    } else {
        app.tab_manager.next_tab();
    }
    app.sync_from_tab();
    app.update_video_visibility();
}

fn handle_paste_shortcut(
    app: &mut ImageViewerApp,
    ctx: &egui::Context,
    text_input_active: bool,
    app_has_focus: bool,
) -> bool {
    let binding = app.shortcuts.get(ShortcutAction::Paste);
    if binding == ShortcutBinding::ctrl(egui::Key::V) {
        if !app_has_focus {
            app.paste_key_debounce = false;
            return false;
        }

        let ctrl_down = is_virtual_key_down(0x11);
        let v_down = is_virtual_key_down(0x56);

        if ctrl_down && v_down && !app.paste_key_debounce && !text_input_active {
            app.paste_key_debounce = true;
            return true;
        }

        if !v_down {
            app.paste_key_debounce = false;
        }
        return false;
    }

    app.paste_key_debounce = false;
    !text_input_active && app.shortcuts.is_triggered(ShortcutAction::Paste, ctx)
}

fn handle_delete_permanently_shortcut(
    app: &mut ImageViewerApp,
    ctx: &egui::Context,
    text_input_active: bool,
    app_has_focus: bool,
) -> bool {
    let binding = app.shortcuts.get(ShortcutAction::DeletePermanently);
    if binding == ShortcutBinding::shift(egui::Key::Delete) {
        if !app_has_focus {
            app.delete_key_debounce = false;
            return false;
        }

        let shift_down = is_virtual_key_down(0x10);
        let del_down = is_virtual_key_down(0x2E);

        if shift_down && del_down && !app.delete_key_debounce && !text_input_active {
            app.delete_key_debounce = true;
            return true;
        }

        if !del_down {
            app.delete_key_debounce = false;
        }
        return false;
    }

    app.delete_key_debounce = false;
    !text_input_active
        && app
            .shortcuts
            .is_triggered(ShortcutAction::DeletePermanently, ctx)
}

pub fn handle_input(app: &mut ImageViewerApp, ctx: &egui::Context) {
    let mut user_active = false;
    if app.renaming_state.is_none()
        && app.sidebar_renaming.is_none()
        && !app.is_address_editing
        && app.batch_rename_state.is_none()
        && app.pending_drag_move_confirmation.is_none()
    {
        if app.shortcut_editor.is_capturing() {
            user_active = ctx.input(|i| {
                i.pointer.any_pressed()
                    || i.pointer.any_click()
                    || i.raw_scroll_delta != egui::Vec2::ZERO
                    || !i.events.is_empty()
            });

            if user_active {
                app.last_user_activity = std::time::Instant::now();
                let _ = app
                    .file_operation_state
                    .idle_warmup_sender
                    .send(IdleWarmupMessage::UserActive);
            }
            return;
        }

        // Handle media hardware input first (overrides normal navigation when player focused)
        if handle_media_hardware_input(app, ctx) {
            return;
        }

        // While the global search modal is open, keep focus/input inside it.
        // Prevent routing shortcuts/quick-search to the main file views.
        if app.global_search.active {
            if app
                .shortcuts
                .is_triggered(ShortcutAction::GlobalSearch, ctx)
            {
                app.close_global_search();
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
                app.last_user_activity = std::time::Instant::now();
                let _ = app
                    .file_operation_state
                    .idle_warmup_sender
                    .send(IdleWarmupMessage::UserActive);
            }
            return;
        }
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
                if let egui::Event::Key { key, pressed, .. } = event {
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
                }
            }
        });

        let app_has_focus = ctx.input(|i| i.viewport().focused.unwrap_or(false));

        if app.shortcuts.is_triggered(ShortcutAction::NewTab, ctx) {
            create_new_tab(app);
            user_active = true;
        }

        if app.shortcuts.is_triggered(ShortcutAction::CloseTab, ctx) {
            close_current_tab(app, ctx);
            user_active = true;
        }

        if app.shortcuts.is_triggered(ShortcutAction::NextTab, ctx) {
            switch_tab(app, false);
            user_active = true;
        }

        if app.shortcuts.is_triggered(ShortcutAction::PreviousTab, ctx) {
            switch_tab(app, true);
            user_active = true;
        }

        if !text_input_active && app.shortcuts.is_triggered(ShortcutAction::SelectAll, ctx) {
            if app.select_all_current_items() {
                app.last_input = crate::app::state::LastInput::Keyboard;
            }
            user_active = true;
        }

        if !text_input_active && app.shortcuts.is_triggered(ShortcutAction::Copy, ctx) {
            app.command_copy(None);
            user_active = true;
        }

        if !text_input_active && app.shortcuts.is_triggered(ShortcutAction::Cut, ctx) {
            app.command_cut(None);
            user_active = true;
        }

        if handle_paste_shortcut(app, ctx, text_input_active, app_has_focus) {
            app.command_paste(None);
            user_active = true;
        }

        if !text_input_active && app.shortcuts.is_triggered(ShortcutAction::Rename, ctx) {
            if app.multi_selection.len() > 1 {
                app.begin_batch_rename();
                user_active = true;
            } else if let Some(idx) = app.selected_item {
                app.begin_rename_item(idx);
                user_active = true;
            }
        }

        if !text_input_active && app.shortcuts.is_triggered(ShortcutAction::Delete, ctx) {
            app.delete_with_shell_for_idx(None);
            user_active = true;
        }

        if handle_delete_permanently_shortcut(app, ctx, text_input_active, app_has_focus) {
            app.delete_permanently_for_idx(None);
            user_active = true;
        }

        if !text_input_active && app.shortcuts.is_triggered(ShortcutAction::Properties, ctx) {
            app.show_properties_for_idx(None);
            user_active = true;
        }

        // F5 = Refresh (always, even in dual panel mode)
        if app.shortcuts.is_triggered(ShortcutAction::Refresh, ctx) {
            app.trigger_manual_refresh();
            user_active = true;
        }

        if app
            .shortcuts
            .is_triggered(ShortcutAction::FocusAddressBar, ctx)
        {
            let display_override =
                app.tag_view_display_name_for_path(&app.navigation_state.current_path);
            app.navigation_state.path_input = address_bar::editable_path(
                &app.navigation_state.current_path,
                display_override.as_deref(),
            );
            app.is_address_editing = true;
            app.show_address_history_menu = false;
            app.address_bar_focus_request = true;
            user_active = true;
        }

        // Ctrl+Scroll: adjust thumbnail size (grid mode only)
        // eframe/winit converts Ctrl+Scroll into zoom_delta before smooth_scroll_delta
        // Read zoom_delta and reset the UI zoom factor back to 1.0
        let zoom_delta = ctx.input(|i| i.zoom_delta());
        if (zoom_delta - 1.0).abs() > 0.001 && app.view_mode == ViewMode::Grid {
            // zoom_delta > 1.0 = scroll up (increase), < 1.0 = decrease
            // Scale: each wheel notch produces delta ~0.1 -> 0.1 x 24 = ~2.4px per notch
            let change = (zoom_delta - 1.0) * 24.0;
            app.thumbnail_size =
                (app.thumbnail_size + change).clamp(crate::ui::theme::THUMBNAIL_MIN, 256.0);
            // Prevent egui from applying zoom to the UI itself
            ctx.set_zoom_factor(1.0);
            app.save_preferences();
            user_active = true;
        }

        if app
            .shortcuts
            .is_triggered(ShortcutAction::CreateFolder, ctx)
            && !app.navigation_state.is_computer_view
            && !app.navigation_state.is_recycle_bin_view
        {
            app.create_new_folder();
            user_active = true;
        }

        if app
            .shortcuts
            .is_triggered(ShortcutAction::GlobalSearch, ctx)
        {
            app.toggle_global_search();
            user_active = true;
        }

        // ── DUAL PANEL SHORTCUTS ──
        // Tab (no modifiers) = switch active panel when dual panel is enabled
        // Ctrl+Shift+D = toggle dual panel on/off
        if !text_input_active {
            let tab_pressed = ctx.input(|i| {
                i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::Key { key: egui::Key::Tab, pressed: true, modifiers, .. }
                        if !modifiers.ctrl && !modifiers.alt && !modifiers.shift
                    )
                })
            });
            if tab_pressed && app.dual_panel_enabled {
                app.dual_panel_switch_active();
                user_active = true;
            }

            let ctrl_shift_d = ctx.input(|i| {
                i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::Key { key: egui::Key::D, pressed: true, modifiers, .. }
                        if modifiers.ctrl && modifiers.shift && !modifiers.alt
                    )
                })
            });
            if ctrl_shift_d {
                app.dual_panel_toggle();
                user_active = true;
            }

            // F6 = move selected to other panel (dual panel only)
            if app.dual_panel_enabled {
                let f6_pressed =
                    ctx.input(|i| {
                        i.events.iter().any(|e| matches!(
                        e,
                        egui::Event::Key { key: egui::Key::F6, pressed: true, modifiers, .. }
                        if !modifiers.ctrl && !modifiers.alt && !modifiers.shift
                    ))
                    });
                if f6_pressed {
                    app.dual_panel_move_to_other();
                    user_active = true;
                }
            }
        }

        let consumed_preview_shortcut_action = handle_preview_shortcut_action(app, ctx);
        if consumed_preview_shortcut_action {
            user_active = true;
        }

        if !consumed_preview_shortcut_action {
            handle_quick_search(app, ctx);
        }
    } else {
        // During rename or batch-rename modal: ESC cancels the operation
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if app.batch_rename_state.is_some() {
                app.batch_rename_state = None;
            } else if app.pending_drag_move_confirmation.is_some() {
                app.cancel_pending_drag_move();
            } else {
                app.renaming_state = None;
                app.focus_rename = false;
            }
            user_active = true;
        }
    }
    if user_active {
        app.last_user_activity = std::time::Instant::now();
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
    // VK_SPACE = 0x20, VK_UP = 0x26, VK_DOWN = 0x28, VK_RIGHT = 0x27, VK_LEFT = 0x25
    // VK_CTRL = 0x11, VK_SHIFT = 0x10, VK_U = 0x55, VK_A = 0x41
    let ctrl_down = is_virtual_key_down(0x11);
    let shift_down = is_virtual_key_down(0x10);
    let u_down = is_virtual_key_down(0x55);
    let a_down = is_virtual_key_down(0x41);

    if ctrl_down && shift_down && u_down && preview.is_rtx_supported() {
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
    } else if is_virtual_key_down(0x20) {
        preview.toggle_play();
        consumed = true;
    } else if is_virtual_key_down(0x26) {
        let vol = preview.get_video_state().map(|s| s.volume).unwrap_or(1.0);
        let new_vol = (vol + 0.05).min(1.0);
        preview.set_volume(new_vol);
        new_session_vol = Some(new_vol);
        let msg = format!("Volume: {}%", (new_vol * 100.0).round() as i32);
        preview.show_osd(&msg, 2000);
        consumed = true;
    } else if is_virtual_key_down(0x28) {
        let vol = preview.get_video_state().map(|s| s.volume).unwrap_or(1.0);
        let new_vol = (vol - 0.05).max(0.0);
        preview.set_volume(new_vol);
        new_session_vol = Some(new_vol);
        let msg = format!("Volume: {}%", (new_vol * 100.0).round() as i32);
        preview.show_osd(&msg, 2000);
        consumed = true;
    } else if is_virtual_key_down(0x27) {
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
    } else if is_virtual_key_down(0x25) {
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

    // Never treat typing inside text fields (toolbar search, global search,
    // address edits, inline editors, etc.) as Explorer-style quick search.
    if ctx.wants_keyboard_input() {
        let active_tab = app.tab_manager.active_mut();
        if !active_tab.quick_search_buffer.is_empty() {
            active_tab.quick_search_buffer.clear();
            log::debug!(
                "[QUICK_SEARCH] Buffer cleared because a text input has focus (Tab {})",
                active_tab.id
            );
        }
        return;
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
