use eframe::egui;
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use crate::app::ImageViewerApp;

pub fn handle_input(app: &mut ImageViewerApp, ctx: &egui::Context) {
    if app.renaming_state.is_none() && !app.is_address_editing {
        // Detectar teclas através dos eventos (Key events)
        let mut do_copy = false;
        let mut do_cut = false;
        let mut do_paste = false;

        ctx.input(|i| {
            for event in &i.events {
                match event {
                    egui::Event::Key {
                        key,
                        pressed,
                        modifiers,
                        ..
                    } => {
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
                                }
                                egui::Key::W => {
                                    if app.tab_manager.close_active_tab() {
                                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                                    } else {
                                        app.sync_from_tab();
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

pub fn handle_resize_borders(app: &mut ImageViewerApp, ctx: &egui::Context) {
    let is_not_maximized = !ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    if is_not_maximized {
        let screen_rect = ctx.screen_rect();
        let border_width = 12.0;

        // Borda ESQUERDA
        let left_border = egui::Rect::from_min_max(
            screen_rect.min,
            egui::pos2(screen_rect.min.x + border_width, screen_rect.max.y),
        );
        egui::Area::new(egui::Id::new("resize_border_left"))
            .fixed_pos(left_border.min)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let left_response = ui.interact(
                    left_border,
                    egui::Id::new("resize_left"),
                    egui::Sense::click_and_drag(),
                );
                if left_response.hovered() { ctx.set_cursor_icon(egui::CursorIcon::ResizeWest); }
                if left_response.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(egui::ResizeDirection::West));
                }
            });

        // Borda DIREITA
        let right_border = egui::Rect::from_min_max(
            egui::pos2(screen_rect.max.x - border_width, screen_rect.min.y),
            screen_rect.max,
        );
        egui::Area::new(egui::Id::new("resize_border_right"))
            .fixed_pos(right_border.min)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let right_response = ui.interact(
                    right_border,
                    egui::Id::new("resize_right"),
                    egui::Sense::click_and_drag(),
                );
                if right_response.hovered() { ctx.set_cursor_icon(egui::CursorIcon::ResizeEast); }
                if right_response.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(egui::ResizeDirection::East));
                }
            });

        // Borda INFERIOR
        let bottom_border = egui::Rect::from_min_max(
            egui::pos2(screen_rect.min.x, screen_rect.max.y - border_width),
            screen_rect.max,
        );
        egui::Area::new(egui::Id::new("resize_border_bottom"))
            .fixed_pos(bottom_border.min)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let bottom_response = ui.interact(
                    bottom_border,
                    egui::Id::new("resize_bottom"),
                    egui::Sense::click_and_drag(),
                );
                if bottom_response.hovered() { ctx.set_cursor_icon(egui::CursorIcon::ResizeSouth); }
                if bottom_response.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(egui::ResizeDirection::South));
                }
            });

        // GRIP VISUAL
        let grip_size = 50.0;
        let grip_pos = egui::pos2(screen_rect.max.x - grip_size, screen_rect.max.y - grip_size);

        egui::Area::new(egui::Id::new("resize_grip"))
            .fixed_pos(grip_pos)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let (_rect, response) = ui.allocate_exact_size(
                    egui::vec2(grip_size, grip_size),
                    egui::Sense::click_and_drag(),
                );
                if response.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(egui::ResizeDirection::SouthEast));
                }
                if response.hovered() { ctx.set_cursor_icon(egui::CursorIcon::ResizeNwSe); }
            });
    }
}
