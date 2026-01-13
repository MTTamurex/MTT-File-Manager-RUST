impl eframe::App for ImageViewerApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Validate selected_file on first frame - clear if path no longer exists
        if self.startup_tick == 0 {
            if let Some(ref file) = self.selected_file {
                if !file.path.exists() {
                    self.selected_file = None;
                    self.selected_thumbnail = None;
                    self.selected_metadata = None;
                }
            }
        }
        
        // --- 3-STAGE STARTUP SEQUENCE ---
        // Stage 1 (frame 1): Apply saved geometry (maximize OR size) while hidden
        // Stage 2 (frames 2-5): Wait for layouts to stabilize  
        // Stage 3 (frame 5): Reveal window
        if self.startup_tick < 5 {
            self.startup_tick += 1;
            
            if self.startup_tick == 1 {
                // Frame 1: Apply saved geometry while window is hidden
                if self.saved_is_maximized {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
                } else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(
                        egui::Vec2::new(self.saved_window_width, self.saved_window_height)
                    ));
                }
            }
            
            if self.startup_tick == 5 {
                // Frame 5: Reveal the window
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);

                // FINAL INITIALIZATION: Agora que a UI estÃ¡ pronta, garante que a aba inicial estÃ¡ populada
                if self.is_computer_view {
                    self.setup_computer_view();
                } else {
                    self.load_folder(false);
                }
                self.sync_to_tab();
            }
            
            // Keep the loop running fast during startup
            ctx.request_repaint();
        }
        // --- END STARTUP SEQUENCE ---

        // Track current window state for saving on exit
        let (size_changed, maximized_changed) = ctx.input(|i| {
            let mut size_changed = false;
            let mut maximized_changed = false;
            
            if let Some(rect) = i.viewport().inner_rect {
                // Only save size when NOT maximized
                if !i.viewport().maximized.unwrap_or(false) {
                    if (self.saved_window_width - rect.width()).abs() > 1.0 || 
                       (self.saved_window_height - rect.height()).abs() > 1.0 {
                        size_changed = true;
                    }
                    self.saved_window_width = rect.width();
                    self.saved_window_height = rect.height();
                }
            }
            
            let new_maximized = i.viewport().maximized.unwrap_or(false);
            if new_maximized != self.saved_is_maximized {
                maximized_changed = true;
            }
            self.saved_is_maximized = new_maximized;
            
            (size_changed, maximized_changed)
        });
        
        // Save preferences when window state changes
        if size_changed || maximized_changed {
            self.save_preferences();
        }
        // --- END STARTUP SEQUENCE ---

        self.ensure_window_handle(frame);

        // --- DETECÇÃO DE COMANDOS DE SISTEMA (Clipboard) ---
        // Usa detecção via eventos RAW de teclas.
        // Só bloqueia durante renomeação ou edição de endereço.

        // DEBUG: Log todos os frames para verificar se o código está rodando
        // eprintln!("[DEBUG] Frame update - renaming={:?} address_editing={}", self.renaming_state.is_some(), self.is_address_editing);

        if self.renaming_state.is_none() && !self.is_address_editing {
            // Detectar teclas através dos eventos (Key events)
            let mut do_copy = false;
            let mut do_cut = false;
            let mut do_paste = false;

            ctx.input(|i| {
                // Log all key events to see what's arriving
                for event in &i.events {
                    match event {
                        egui::Event::Key {
                            key,
                            pressed,
                            modifiers,
                            ..
                        } => {
                            if *pressed && modifiers.ctrl {
                                eprintln!("[DEBUG] Key event: {:?} Ctrl+pressed", key);
                                match key {
                                    egui::Key::C => do_copy = true,
                                    egui::Key::X => do_cut = true,
                                    egui::Key::V => do_paste = true,
                                    // TAB MANAGEMENT SHORTCUTS
                                    egui::Key::T => {
                                        // Ctrl+T = New tab
                                        self.sync_to_tab();
                                        self.tab_manager.new_tab();
                                        self.sync_from_tab();
                                        self.setup_computer_view();
                                        self.sync_to_tab();
                                    }
                                    egui::Key::W => {
                                        // Ctrl+W = Close current tab
                                        if self.tab_manager.close_active_tab() {
                                            // Last tab - quit app
                                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                                        } else {
                                            self.sync_from_tab();
                                        }
                                    }
                                    egui::Key::Tab => {
                                        // Ctrl+Tab = Next tab, Ctrl+Shift+Tab = Previous tab
                                        self.sync_to_tab();
                                        if modifiers.shift {
                                            self.tab_manager.prev_tab();
                                        } else {
                                            self.tab_manager.next_tab();
                                        }
                                        self.sync_from_tab();
                                    }
                                    _ => {}
                                }
                            }
                        }
                        egui::Event::Copy => {
                            do_copy = true;
                        }
                        egui::Event::Cut => {
                            do_cut = true;
                        }
                        egui::Event::Paste(_) => {
                            do_paste = true;
                        }
                        _ => {}
                    }
                }
            });

            // Fallback: use Windows GetAsyncKeyState for hardware-level detection
            // (Windows consumes Ctrl+V key events when clipboard has files)
            // VK_CONTROL = 0x11, VK_V = 0x56
            let ctrl_down = unsafe { GetAsyncKeyState(0x11) < 0 };
            let v_down = unsafe { GetAsyncKeyState(0x56) < 0 };

            // Debounced paste detection (only fire once per key press)
            if ctrl_down && v_down && !self.paste_key_debounce {
                do_paste = true;
                self.paste_key_debounce = true;
            } else if !v_down {
                self.paste_key_debounce = false;
            }

            // Executar ações de clipboard
            if do_copy {
                self.command_copy(None);
            }
            if do_cut {
                self.command_cut(None);
            }
            if do_paste {
                self.command_paste(None);
            }

            // Delete: Excluir
            if ctx.input(|i| i.key_pressed(egui::Key::Delete)) {
                self.delete_with_shell_for_idx(None);
            }

            // Ctrl + Shift + N: Nova Pasta
            if ctx.input(|i| i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(egui::Key::N)) {
                self.create_new_folder();
            }
        } else {
            // Durante renomeação: ESC cancela a operação
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.renaming_state = None;
                self.focus_rename = false;
            }
        }

        self.process_incoming_messages(ctx);
        self.refresh_drives_if_needed();
        self.ensure_folder_icon(ctx);
        self.ensure_computer_icon(ctx);

        // Status Bar (Footer) - Definido primeiro para ocupar toda a largura
        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(24.0)
            .show(ctx, |ui| {
                use mtt_file_manager::ui::status_bar::{render_status_bar, StatusBarAction};
                let action = render_status_bar(
                    ui,
                    &mut self.is_loading_folder,
                    self.total_items,
                    &mut self.view_mode,
                    &mut self.thumbnail_size,
                    &mut self.sort_mode,
                    &mut self.sort_descending,
                    &mut self.folders_position,
                    &self.cache_manager.texture_cache,
                );
                match action {
                    StatusBarAction::SortChanged => {
                        self.sort_items();
                        self.save_preferences();
                    }
                    StatusBarAction::ViewModeChanged => {
                        // View mode changed - nothing extra to do
                    }
                    StatusBarAction::None => {}
                }
            });

        // Windows 11 style sidebar
        // Left Sidebar moved to after TopPanels for correct layout

        // TAB BAR (custom title bar with tabs and window controls)
        egui::TopBottomPanel::top("tab_bar_panel")
            .show_separator_line(false)
            .exact_height(36.0)
            .frame(egui::Frame {
                fill: if ctx.style().visuals.dark_mode {
                    egui::Color32::from_rgb(32, 32, 32)
                } else {
                    egui::Color32::from_rgb(243, 243, 243)
                },
                ..Default::default()
            })
            .show(ctx, |ui| {
                use mtt_file_manager::ui::tab_bar::{render_tab_bar, TabBarAction};
                let action = render_tab_bar(
                    ui,
                    &self.tab_manager,
                    &mut self.svg_icon_manager,
                    frame,
                    self.cache_manager.computer_icon.as_ref(), // Pass native computer icon
                    &mut self.item_icon_loader,               // Pass icon loader for dynamic icons
                );
                
                match action {
                    TabBarAction::SwitchTab(idx) => {
                        self.sync_to_tab();
                        self.tab_manager.switch_to(idx);
                        self.sync_from_tab();
                    }
                    TabBarAction::NewTab => {
                        self.sync_to_tab();
                        self.tab_manager.new_tab();
                        self.sync_from_tab();
                        self.setup_computer_view(); // Popula os drives na nova aba
                        self.sync_to_tab(); // Salva estado populado
                    }
                    TabBarAction::CloseTab(idx) => {
                        if self.tab_manager.close_tab(idx) {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        } else {
                            self.sync_from_tab();
                        }
                    }
                    TabBarAction::CloseApp => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    TabBarAction::ToggleMaximize => {
                        let is_maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!is_maximized));
                    }
                    TabBarAction::Minimize => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    }
                    TabBarAction::None => {}
                }
            });

        // Top navigation bar
        egui::TopBottomPanel::top("nav_bar")
            .show_separator_line(true)
            .frame(egui::Frame {
                fill: if ctx.style().visuals.dark_mode {
                    egui::Color32::from_rgb(45, 45, 45) // Matches tab_bar.rs active_bg
                } else {
                    egui::Color32::from_rgb(255, 255, 255) // Matches tab_bar.rs active_bg
                },
                ..Default::default()
            })
            .show(ctx, |ui| {
                use mtt_file_manager::ui::toolbar::{render_toolbar, ToolbarAction};
                // Make sure ViewMode is imported or available via crate::domain::file_entry
                use mtt_file_manager::domain::file_entry::{SortMode, ViewMode};

                let action = render_toolbar(
                    ui,
                    &self.current_path,
                    &mut self.path_input,
                    &mut self.is_address_editing,
                    &mut self.search_query,
                    &self.navigation,
                    self.view_mode,
                    self.sort_mode,
                    self.sort_descending,
                    &mut self.thumbnail_size,
                    self.show_preview_panel,
                    self.renaming_state.is_some(),
                    self.cache_manager.computer_icon.as_ref(),
                    &mut self.svg_icon_manager,
                );

                if let Some(act) = action {
                    match act {
                        ToolbarAction::GoBack => self.go_back(),
                        ToolbarAction::GoForward => self.go_forward(),
                        ToolbarAction::GoUp => self.go_up_one_level(),
                        ToolbarAction::Refresh => self.trigger_manual_refresh(),
                        ToolbarAction::CreateFolder => self.create_new_folder(),
                        ToolbarAction::NavigateToComputer => self.navigate_to_computer(),
                        ToolbarAction::NavigateToRecycleBin => self.navigate_to_recycle_bin(),
                        ToolbarAction::ToggleViewMode => {
                            if self.view_mode == ViewMode::List {
                                self.view_mode = ViewMode::Grid;
                            } else {
                                self.view_mode = ViewMode::List;
                            }
                        },
                        ToolbarAction::TogglePreviewPanel => self.show_preview_panel = !self.show_preview_panel,
                        ToolbarAction::ChangeSortMode(mode) => {
                            self.sort_mode = mode;
                            self.sort_items();
                            self.save_preferences();
                        },
                        ToolbarAction::ToggleSortDescending => {
                            self.sort_descending = !self.sort_descending;
                            self.sort_items();
                            self.save_preferences();
                        },
                        ToolbarAction::Search(_query) => {
                            self.filter_items();
                        },
                        ToolbarAction::Navigate(path) => self.navigate_to(&path),
                        ToolbarAction::StartAddressEdit => {
                            self.path_input = self.current_path.clone();
                            self.is_address_editing = true;
                        },
                        ToolbarAction::CommitPathInput(path) => {
                            if std::path::Path::new(&path).exists() {
                                self.navigate_to(&path);
                                self.is_address_editing = false;
                            } else {
                                self.path_input = self.current_path.clone();
                                self.is_address_editing = false;
                            }
                        },
                         ToolbarAction::CancelPathInput => {
                             self.is_address_editing = false;
                             self.path_input = self.current_path.clone();
                         },
                         ToolbarAction::UpdatePathInput(_) => {
                             // Handled by text edit binding
                         },
                         _ => {}
                    }
                }
            });

        // Windows 11 style sidebar (Restored)
        
        let sidebar_response = egui::SidePanel::left("sidebar")
            .min_width(150.0)
            .default_width(self.sidebar_left_width.max(150.0)) // Garante que nunca seja 0
            .resizable(true)
            .show(ctx, |ui| {
                use mtt_file_manager::ui::sidebar::{render_sidebar, SidebarContext};

                // Clonar dados necessários para evitar problemas de borrow
                let disks = self.disks.clone();
                let current_path = self.current_path.clone();
                let is_computer_view = self.is_computer_view;
                let computer_icon = self.cache_manager.computer_icon.clone();

                // Criar contexto para sidebar
                let mut ctx = SidebarContext {
                    disks: &disks,
                    current_path: &current_path,
                    is_computer_view,
                    is_recycle_bin_view: self.is_recycle_bin_view,
                    computer_icon: computer_icon.as_ref(),
                    is_renaming: self.renaming_state.is_some(),
                    icon_loader: &mut self.item_icon_loader,
                    onedrive_path: self.onedrive_path.as_deref(),
                    onedrive_icon: self.onedrive_icon.as_ref(),
                };

                render_sidebar(ui, &mut ctx)
            });
        
        // Captura a largura REAL do painel (não a disponível dentro dele)
        // IMPORTANTE: Não atualiza se janela está minimizada (rect fica inválido)
        let is_minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
        let actual_panel_width = sidebar_response.response.rect.width();
        if !is_minimized && actual_panel_width > 100.0 && (self.sidebar_left_width - actual_panel_width).abs() > 2.0 {
            self.sidebar_left_width = actual_panel_width;
        }
        
        let sidebar_action = sidebar_response.inner;

        // Processar ação da sidebar (após ctx ser dropado e self liberado)
        if let Some(action) = sidebar_action {
            use mtt_file_manager::ui::sidebar::SidebarAction;
            match action {
                SidebarAction::NavigateTo(path) => self.navigate_to(&path),
                SidebarAction::NavigateToComputer => self.navigate_to_computer(),
                SidebarAction::NavigateToRecycleBin => self.navigate_to_recycle_bin(),
            }
        }

        // Preview Pane (Windows Explorer style) - ANTES do CentralPanel
        if self.show_preview_panel {
            self.refresh_selected_metadata();
            
            let right_panel_response = egui::SidePanel::right("preview_panel")
                .resizable(true)
                .default_width(self.sidebar_right_width.max(250.0)) // Garante que nunca seja 0
                .min_width(250.0)
                .max_width(500.0)
                .show(ctx, |ui| {
                    use mtt_file_manager::ui::preview_panel::{render_preview_panel, PreviewPanelAction};

                    egui::ScrollArea::vertical()
                        .id_source("preview_scroll")
                        .show(ui, |ui| {
                            ui.set_max_width(ui.available_width());
                            
                            // 1. Calculate effective_file (Logic from main.rs)
                            let effective_file = if let Some(file) = self.selected_file.clone() {
                                if self.is_recycle_bin_view || file.path.exists() {
                                    Some(file)
                                } else {
                                    None
                                }
                            } else if self.is_recycle_bin_view {
                                // Logic for Lixeira root...
                                Some(FileEntry {
                                    path: PathBuf::from("Lixeira"),
                                    name: "Lixeira".to_string(),
                                    is_dir: true,
                                    size: 0,
                                    modified: 0,
                                    folder_cover: None,
                                    drive_info: None,
                                    sync_status: SyncStatus::None,
                                    deletion_date: None,
                                })
                            } else if !self.is_computer_view {
                                // Fallback logic
                                let path = std::path::PathBuf::from(&self.current_path);
                                let mut entry = FileEntry::from_path(path.clone(), true);
                                if path.to_string_lossy().len() <= 3 && path.to_string_lossy().contains(':') {
                                     use mtt_file_manager::infrastructure::windows::get_volume_info;
                                     let vol = get_volume_info(&self.current_path);
                                     let drive_type = windows_infra::detect_drive_type(&self.current_path);
                                     let label = self.disks.iter()
                                         .find(|(p, _)| p.starts_with(&self.current_path) || self.current_path.starts_with(p))
                                         .map(|(_, l)| l.clone())
                                         .unwrap_or_else(|| self.current_path.clone());
                                     entry.name = label;
                                     entry.drive_info = Some(mtt_file_manager::domain::file_entry::DriveInfo {
                                         file_system: vol.file_system,
                                         total_space: vol.total_space,
                                         free_space: vol.free_space,
                                         drive_type,
                                     });
                                } else {
                                     entry.name = path.file_name()
                                         .map(|n| n.to_string_lossy().to_string())
                                         .unwrap_or_else(|| self.current_path.clone());
                                }
                                Some(entry)
                            } else {
                                None
                            };

                            if let Some(file) = effective_file {
                                // 2. Metadata
                                let selected_metadata = self.selected_metadata.as_ref().and_then(|(p, meta)| {
                                    if p == &file.path { Some(meta) } else { None }
                                });
                                
                                // 3. Folder Size
                                let folder_size = if file.is_dir {
                                    self.folder_size_cache.get(&file.path).copied()
                                } else { None };
                                let is_folder_size_loading = self.folder_size_loading.contains(&file.path);

                                // 4. Render Panel
                                let action = render_preview_panel(
                                    ui,
                                    &file,
                                    self.selected_thumbnail.as_ref(), // Passed from main
                                    selected_metadata,
                                    self.cache_manager.texture_cache.peek(&file.path).cloned(),
                                    self.cache_manager.folder_preview_cache.get(&file.path).cloned(),
                                    self.cache_manager.folder_preview_loading.contains(&file.path),
                                    self.metadata_loading.contains(&file.path),
                                    folder_size,
                                    is_folder_size_loading,
                                    self.is_recycle_bin_view,
                                    &mut self.item_icon_loader,
                                    &mut self.svg_icon_manager,
                                );

                                if let Some(act) = action {
                                     match act {
                                         PreviewPanelAction::RefreshThumbnail(path) => {
                                              self.disk_cache.remove_cache_for_path(&path);
                                              self.cache_manager.texture_cache.pop(&path);
                                              self.cache_manager.loading_set.remove(&path);
                                              let _ = self.thumbnail_req_sender.send((path, self.generation));
                                              self.notifications.push(mtt_file_manager::application::AppNotification::info("Recarregando thumbnail...".to_string()));
                                         },
                                         PreviewPanelAction::LoadFolderPreview(path) => {
                                              if self.cache_manager.folder_preview_loading.len() < 30 {
                                                  self.cache_manager.folder_preview_loading.insert(path.clone());
                                                  let _ = self.folder_preview_sender.send(path);
                                              }
                                         },
                                         PreviewPanelAction::CalculateFolderSize(path) => {
                                              self.folder_size_loading.insert(path.clone());
                                              let _ = self.folder_size_req_sender.send(path);
                                         }
                                     }
                                }
                            } else {
                                ui.vertical_centered(|ui| {
                                    ui.add_space(100.0);
                                    ui.label("Nenhum item selecionado");
                                    ui.label("Selecione algo para ver detalhes");
                                });
                            }
                        });
                });
            
            // Captura a largura REAL do painel direito
            // IMPORTANTE: Não atualiza se janela está minimizada (rect fica inválido)
            let is_minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
            let actual_panel_width = right_panel_response.response.rect.width();
            if !is_minimized && actual_panel_width > 200.0 && (self.sidebar_right_width - actual_panel_width).abs() > 2.0 {
                self.sidebar_right_width = actual_panel_width;
            }
        }

        // Central Panel
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.is_loading_folder && self.items.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.spinner();
                    ui.label("Carregando...");
                });
            } else if self.items.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label("Pasta vazia");
                });
            } else {
                match self.view_mode {
                    ViewMode::Grid => self.render_grid_view(ui),
                    ViewMode::List => self.render_list_view(ui),
                }

                // F2 -> INICIAR RENOMEAÇÃO (Global no CentralPanel)
                if ui.input(|i| i.key_pressed(egui::Key::F2)) {
                    if let Some(idx) = self.selected_item {
                        if let Some(item) = self.items.get(idx) {
                            self.renaming_state = Some((idx, item.name.clone()));
                            self.focus_rename = true;
                        }
                    }
                }

                // Spinner pequeno no canto se ainda carregando
                if self.is_loading_folder {
                    let rect = ui.max_rect();
                    let spinner_rect = egui::Rect::from_min_size(
                        rect.right_bottom() - egui::vec2(24.0, 24.0),
                        egui::vec2(16.0, 16.0),
                    );
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(spinner_rect), |ui| {
                        ui.spinner();
                    });
                }
            }

            // Detecção de clique direito na área vazia (fora dos itens)
            // Só abre menu de contexto se não houver item selecionado pelo clique direito
            if !self.context_menu.is_open
                && ui.input(|i| i.pointer.secondary_clicked())
            {
                // Verifica se o clique foi em um item
                let pointer_pos = ui.ctx().pointer_latest_pos();
                let mut clicked_on_item = false;

                // Verifica se o clique foi em algum item (grid ou lista)
                if let Some(pos) = pointer_pos {
                    // Para grid view
                    if self.view_mode == ViewMode::Grid && !self.items.is_empty() {
                        let padding = 8.0;
                        let item_w = self.thumbnail_size;
                        let item_h = self.thumbnail_size + 20.0;
                        let available_w = ui.available_width();
                        let cols = ((available_w - padding) / (item_w + padding))
                            .floor()
                            .max(1.0) as usize;

                        // Calcula qual célula foi clicada
                        let content_min = ui.min_rect().min;
                        let relative_x = pos.x - content_min.x;
                        let relative_y = pos.y - content_min.y;

                        let col = (relative_x / (item_w + padding)).floor() as usize;
                        let row = (relative_y / (item_h + padding)).floor() as usize;
                        let index = row * cols + col;

                        if index < self.items.len() {
                            clicked_on_item = true;
                        }
                    }
                    // Para list view (mais simples - verifica se está na área dos itens)
                    else if self.view_mode == ViewMode::List && !self.items.is_empty() {
                        let row_height = 24.0;
                        let total_rows = self.items.len();
                        let scroll_area_top = ui.min_rect().top();
                        let relative_y = pos.y - scroll_area_top;

                        let row = (relative_y / row_height).floor() as usize;
                        if row < total_rows {
                            clicked_on_item = true;
                        }
                    }
                }

                // Se não clicou em item, abre menu de contexto estilizado para a pasta atual (área vazia)
                if !clicked_on_item {
                    let path = PathBuf::from(&self.current_path);
                    let pointer_pos = ui.ctx().pointer_latest_pos().unwrap_or(egui::Pos2::ZERO);
                    self.populate_context_menu(ui.ctx(), &path, true, None);
                    self.context_menu.open(
                        pointer_pos,
                        None,
                        Some(path),
                        true,
                    );
                }
            }
        });

        // Exibe o menu de contexto (se aberto)
        let mut context_menu = std::mem::replace(&mut self.context_menu, mtt_file_manager::application::context_menu::ContextMenuState::default());
        let _ = mtt_file_manager::ui::context_menu::render_context_menu(ctx, &mut context_menu, &mut self.svg_icon_manager);
        
        // Handle selected command before putting state back
        if let Some(id) = context_menu.selected_command_id.take() {
            if id > 0 {
                // Shell command
                if let Some(native_ctx) = &context_menu.native_context {
                    if let Some(shell_ctx) = native_ctx.downcast_ref::<mtt_file_manager::infrastructure::windows::native_menu::ShellMenuContext>() {
                        let _ = mtt_file_manager::infrastructure::windows::native_menu::invoke_menu_command(
                            self.native_hwnd.unwrap_or_default(),
                            &shell_ctx.context_menu,
                            id as u32,
                            context_menu.position.x as i32,
                            context_menu.position.y as i32,
                        );
                    }
                }
            } else {
                // Internal command handled via trait
                let item_idx = context_menu.item_index;
                eprintln!("[DEBUG] Internal command id: {}, item_idx: {:?}", id, item_idx);
                match id {
                    -1 => self.create_new_folder(),
                    -2 | -31 => self.command_copy(item_idx),
                    -3 | -30 => self.command_cut(item_idx),
                    -4 | -32 => self.command_paste(item_idx),
                    -5 | -33 => {
                        if let Some(idx) = item_idx.or(self.selected_item) {
                            if let Some(item) = self.items.get(idx) {
                                self.renaming_state = Some((idx, item.name.clone()));
                                self.focus_rename = true;
                            }
                        }
                    }
                    -6 | -34 => self.delete_with_shell_for_idx(item_idx),
                    -20 => {
                        // Abrir: Navigate into folder or open file with shell
                        if let Some(path) = self.context_target_path(item_idx) {
                            if path.is_dir() {
                                self.navigate_to(&path.to_string_lossy());
                            } else {
                                open_with_shell(&path);
                            }
                        }
                    }
                    -21 => {
                        if let Some(path) = self.context_target_path(item_idx) {
                            let target = if path.is_dir() {
                                path
                            } else {
                                path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from(&self.current_path))
                            };

                            self.sync_to_tab();
                            self.tab_manager.new_tab_at(&target.to_string_lossy());
                            self.sync_from_tab();

                            if self.is_computer_view {
                                self.setup_computer_view();
                            } else {
                                self.watch_current_folder();
                                self.load_folder(false);
                            }
                        }
                    }
                    -24 => {
                        if let Some(path) = self.context_target_path(item_idx) {
                            self.copy_path_to_clipboard(&path);
                        }
                    }
                    -26 => {
                        if let Some(path) = self.context_target_path(item_idx) {
                            match self.create_shell_shortcut(&path) {
                                Ok(created) => {
                                    // Refresh to show the new shortcut in the view
                                    self.load_folder(false);
                                    self.notifications.push(
                                        mtt_file_manager::application::AppNotification::info(
                                            format!("Atalho criado: {}", created.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()),
                                        ),
                                    );
                                }
                                Err(e) => {
                                    self.notifications.push(
                                        mtt_file_manager::application::AppNotification::error(
                                            format!("Falha ao criar atalho: {e}"),
                                        ),
                                    );
                                }
                            }
                        }
                    }
                    -28 => self.show_properties_for_idx(item_idx),
                    // Recycle Bin actions
                    -50 | -52 => {
                        // Restaurar
                        if let Some(idx) = item_idx.or(self.selected_item) {
                            if let Some(item) = self.items.get(idx) {
                                let path = item.path.clone();
                                self.restore_from_recycle_bin(&path);
                            }
                        }
                    }
                    -51 | -53 => {
                        // Excluir permanentemente
                        if let Some(idx) = item_idx.or(self.selected_item) {
                            if let Some(item) = self.items.get(idx) {
                                let path = item.path.clone();
                                self.delete_permanently(&path);
                            }
                        }
                    }
                    -54 => {
                        // Esvaziar Lixeira
                        self.empty_recycle_bin();
                    }
                    _ => {}
                }
            }
            context_menu.close();
        }
        
        self.context_menu = context_menu;

        // === RESIZE GRIP (bottom-right corner) ===
        let is_not_maximized = !ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        if is_not_maximized {
            let screen_rect = ctx.screen_rect();
            
            // === BORDAS INVISÍVEIS PARA RESIZE (8px de largura) ===
            let border_width = 12.0;  // mais fácil de clicar
            
            // Borda ESQUERDA
            let left_border = egui::Rect::from_min_max(
                screen_rect.min,
                egui::pos2(screen_rect.min.x + border_width, screen_rect.max.y)
            );
            egui::Area::new(egui::Id::new("resize_border_left"))
                .fixed_pos(left_border.min)
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    let left_response = ui.interact(left_border, egui::Id::new("resize_left"), egui::Sense::click_and_drag());
                    if left_response.hovered() {
                        ctx.set_cursor_icon(egui::CursorIcon::ResizeWest);
                    }
                    if left_response.drag_started() {
                        // Usa egui BeginResize - funciona mas tem efeito sanfona no lado esquerdo
                        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(egui::ResizeDirection::West));
                    }
                });
            
            // Borda DIREITA
            let right_border = egui::Rect::from_min_max(
                egui::pos2(screen_rect.max.x - border_width, screen_rect.min.y),
                screen_rect.max
            );
            egui::Area::new(egui::Id::new("resize_border_right"))
                .fixed_pos(right_border.min)
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    let right_response = ui.interact(right_border, egui::Id::new("resize_right"), egui::Sense::click_and_drag());
                    if right_response.hovered() {
                        ctx.set_cursor_icon(egui::CursorIcon::ResizeEast);
                    }
                    if right_response.drag_started() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(egui::ResizeDirection::East));
                    }
                });
            
            // Borda INFERIOR
            let bottom_border = egui::Rect::from_min_max(
                egui::pos2(screen_rect.min.x, screen_rect.max.y - border_width),
                screen_rect.max
            );
            egui::Area::new(egui::Id::new("resize_border_bottom"))
                .fixed_pos(bottom_border.min)
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    let bottom_response = ui.interact(bottom_border, egui::Id::new("resize_bottom"), egui::Sense::click_and_drag());
                    if bottom_response.hovered() {
                        ctx.set_cursor_icon(egui::CursorIcon::ResizeSouth);
                    }
                    if bottom_response.drag_started() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(egui::ResizeDirection::South));
                    }
                });
            
            // === GRIP VISUAL (canto inferior direito - 50x50px) ===
            let grip_size = 50.0;  // MUITO maior para ser facilmente clicável
            let grip_pos = egui::pos2(
                screen_rect.max.x - grip_size,
                screen_rect.max.y - grip_size,
            );
            let grip_rect = egui::Rect::from_min_size(grip_pos, egui::vec2(grip_size, grip_size));
            
            egui::Area::new(egui::Id::new("resize_grip"))
                .fixed_pos(grip_pos)
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    let (_rect, response) = ui.allocate_exact_size(
                        egui::vec2(grip_size, grip_size),
                        egui::Sense::click_and_drag(),
                    );
                    
                    // SEM VISUAL - apenas área interativa (sem listras aparecendo por cima)
                    
                    // Handle resize drag - só dispara no início do drag
                    if response.drag_started() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(egui::ResizeDirection::SouthEast));
                    }
                    
                    // Change cursor on hover
                    if response.hovered() {
                        ctx.set_cursor_icon(egui::CursorIcon::ResizeNwSe);
                    }
                });
        }
        
        // === TOAST NOTIFICATIONS ===
        self.notifications.cleanup(); // Remove expired notifications

        if !self.notifications.is_empty() {
            let toast_width = 300.0;
            let toast_height = 40.0;
            let padding = 10.0;
            let margin = 20.0;

            let screen = ctx.screen_rect();
            let base_x = screen.max.x - toast_width - margin;

            for (i, notification) in self.notifications.active().iter().enumerate() {
                let base_y = screen.max.y - margin - ((i + 1) as f32 * (toast_height + padding));
                let fade = notification.remaining_fraction();

                let mut bg_color = notification.level.color();
                bg_color = egui::Color32::from_rgba_unmultiplied(
                    bg_color.r(),
                    bg_color.g(),
                    bg_color.b(),
                    (fade * 230.0) as u8,
                );

                egui::Area::new(egui::Id::new(format!("toast_{}", i)))
                    .fixed_pos(egui::pos2(base_x, base_y))
                    .order(egui::Order::Foreground)
                    .show(ctx, |ui| {
                        let rect = egui::Rect::from_min_size(
                            ui.cursor().min,
                            egui::vec2(toast_width, toast_height),
                        );

                        ui.painter().rect_filled(rect, 6.0, bg_color);

                        // Icon
                        ui.painter().text(
                            rect.min + egui::vec2(12.0, 12.0),
                            egui::Align2::LEFT_TOP,
                            notification.level.icon(),
                            egui::FontId::proportional(14.0),
                            egui::Color32::WHITE.gamma_multiply(fade),
                        );

                        // Message
                        ui.painter().text(
                            rect.min + egui::vec2(32.0, 12.0),
                            egui::Align2::LEFT_TOP,
                            &notification.message,
                            egui::FontId::proportional(13.0),
                            egui::Color32::WHITE.gamma_multiply(fade),
                        );
                    });
            }
            ctx.request_repaint(); // Keep animating
        }

    }

    /// Called when the app is exiting - save all preferences
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Force save sidebar widths before exit
        self.save_preferences();
        eprintln!("[EXIT] Saved sidebar widths: L={}, R={}", self.sidebar_left_width, self.sidebar_right_width);
    }
}

/// Load application icon from PNG file
fn load_app_icon() -> Option<egui::IconData> {
    let icon_path = std::path::PathBuf::from("appicon.png");
    
    if !icon_path.exists() {
        eprintln!("Warning: appicon.png not found - using default icon");
        return None;
    }
    
    // Load PNG using image crate
    match image::open(&icon_path) {
        Ok(img) => {
            // Resize to 256x256 for optimal display (Windows icon standard)
            let resized = img.resize_exact(256, 256, image::imageops::FilterType::Lanczos3);
            let rgba_image = resized.to_rgba8();
            let pixels = rgba_image.into_raw();
            
            Some(egui::IconData {
                rgba: pixels,
                width: 256,
                height: 256,
            })
        }
        Err(e) => {
            eprintln!("Warning: Failed to load appicon.png: {}", e);
            None
        }
    }
}
