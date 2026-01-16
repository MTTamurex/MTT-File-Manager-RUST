//! List view rendering
//! Follows .cursorrules: single responsibility, < 300 lines

use eframe::egui::{self, Color32, FontId, Pos2, Rect, RichText, Sense, Ui};
use std::path::PathBuf;

use crate::domain::file_entry::{FileEntry, SortMode, SyncStatus};
use crate::infrastructure::windows::{format_date, format_size};

/// Context for list view rendering
pub struct ListViewContext<'a> {
    pub items: &'a [FileEntry],
    pub selected_item: Option<usize>,
    pub selected_file: Option<&'a FileEntry>,
    pub sort_mode: SortMode,
    pub sort_descending: bool,
    pub renaming_state: Option<(usize, String)>,
    pub focus_rename: bool,
    pub scroll_to_selected: bool, // Scroll to selected item on keyboard navigation
    pub is_computer_view: bool,
    pub is_recycle_bin_view: bool,
    pub is_onedrive_folder: bool,
    pub texture_cache: &'a mut lru::LruCache<PathBuf, egui::TextureHandle>,
    pub loading_set: &'a mut std::collections::HashSet<PathBuf>,
    pub scanned_folders: &'a mut std::collections::HashSet<PathBuf>,
    pub folder_icon_texture: Option<&'a egui::TextureHandle>,
    pub computer_icon: Option<&'a egui::TextureHandle>,
    pub drive_icon_cache: &'a mut lru::LruCache<String, egui::TextureHandle>,
    pub item_icon_loader: &'a mut crate::ui::icon_loader::IconLoader,
    pub deletion_date_cache: Option<&'a mut lru::LruCache<String, String>>, // Cache para datas de exclusão (Path string -> Data)
}

/// Action returned by list view
pub enum ListViewAction {
    Click(usize),
    DoubleClick(usize),
    SecondaryClick(usize),
    SortChange(SortMode),
    EmptyAreaSecondaryClick,
}

/// Operations that can be performed from list view
pub trait ListViewOperations {
    fn navigate_to(&mut self, path: &str);
    fn open_with_shell(&mut self, path: &PathBuf);
    fn request_thumbnail_load(&mut self, path: PathBuf);
    fn request_folder_scan(&mut self, path: PathBuf);
    fn request_folder_preview_load(&mut self, path: PathBuf);
    fn rename_with_shell(&mut self, idx: usize);
}

/// Renders the list view
pub fn render_list_view(
    ui: &mut Ui,
    ctx: &mut ListViewContext,
    ops: &mut dyn ListViewOperations,
) -> Option<ListViewAction> {
    let row_height = 24.0;
    let available_w = ui.available_width();

    // Column widths - add status column when in OneDrive folder
    let w_status = if ctx.is_onedrive_folder && !ctx.is_computer_view {
        120.0
    } else {
        0.0
    };
    let base_cols = 410.0 + w_status;
    let w_name = (available_w - base_cols).max(200.0);
    let w_date = 170.0;
    let w_type = 120.0;
    let w_size = 100.0;

    // Table header - capture sort mode change
    let mut sort_action: Option<SortMode> = None;

    ui.horizontal(|ui| {
        ui.style_mut().spacing.item_spacing.x = 0.0;

        let draw_header = |ui: &mut Ui, text: &str, width: f32, mode: SortMode| {
            let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 22.0), Sense::click());
            let is_active = ctx.sort_mode == mode;

            if ui.is_rect_visible(rect) {
                if is_active {
                    ui.painter().rect_filled(rect, 2.0, Color32::from_gray(230));
                }
                let text_color = if is_active {
                    Color32::BLACK
                } else {
                    Color32::from_gray(100)
                };
                ui.painter().text(
                    rect.min + egui::vec2(8.0, 4.0),
                    egui::Align2::LEFT_TOP,
                    text,
                    FontId::proportional(12.0),
                    text_color,
                );
                if is_active {
                    let arrow = if ctx.sort_descending { "v" } else { "^" };
                    ui.painter().text(
                        rect.max - egui::vec2(15.0, 8.0),
                        egui::Align2::CENTER_CENTER,
                        arrow,
                        FontId::proportional(10.0),
                        text_color,
                    );
                }
            }

            (response.clicked(), mode)
        };

        let (clicked_name, _) = draw_header(ui, "Nome", w_name, SortMode::Name);
        if clicked_name {
            return Some(SortMode::Name);
        }

        if ctx.is_computer_view {
            let (clicked_type, _) = draw_header(ui, "Tipo", w_type, SortMode::Type);
            if clicked_type {
                return Some(SortMode::Type);
            }

            let (clicked_total, _) = draw_header(ui, "Espaço Total", w_date, SortMode::Size);
            if clicked_total {
                return Some(SortMode::Size);
            }

            let (clicked_free, _) = draw_header(ui, "Espaço Livre", w_size, SortMode::Size);
            if clicked_free {
                return Some(SortMode::Size);
            }
        } else {
            let date_label = if ctx.is_recycle_bin_view {
                "Data de Exclusão"
            } else {
                "Última modificação"
            };
            let (clicked_date, _) = draw_header(ui, date_label, w_date, SortMode::Date);
            if clicked_date {
                return Some(SortMode::Date);
            }

            let (clicked_type, _) = draw_header(ui, "Tipo", w_type, SortMode::Type);
            if clicked_type {
                return Some(SortMode::Type);
            }

            let (clicked_size, _) = draw_header(ui, "Tamanho", w_size, SortMode::Size);
            if clicked_size {
                return Some(SortMode::Size);
            }

            // Status column (OneDrive only)
            if ctx.is_onedrive_folder {
                let (rect, _response) =
                    ui.allocate_exact_size(egui::vec2(w_status, 22.0), Sense::hover());
                if ui.is_rect_visible(rect) {
                    ui.painter().text(
                        rect.min + egui::vec2(8.0, 4.0),
                        egui::Align2::LEFT_TOP,
                        "Status",
                        FontId::proportional(12.0),
                        Color32::from_gray(100),
                    );
                }
            }
        }

        None
    })
    .inner
    .map(|mode| sort_action = Some(mode));

    ui.separator();

    let total_rows = ctx.items.len();
    // Virtualized list or Grouped list for Computer View
    let mut clicked_item = None;
    let mut double_clicked_item = None;
    let mut secondary_clicked_item = None;
    let mut empty_area_secondary_click = false;

    let scroll_area = egui::ScrollArea::vertical().auto_shrink([false, false]);
    let available_rect = ui.available_rect_before_wrap();

    if ctx.is_computer_view {
        // Grouped view for "Este Computador"
        scroll_area.show(ui, |ui| {
            let mut local = Vec::new();
            let mut network = Vec::new();

            for (i, item) in ctx.items.iter().enumerate() {
                let is_remote = item.drive_info.as_ref().map_or(false, |di| {
                    di.drive_type == crate::infrastructure::windows::DriveType::Remote
                });
                if is_remote {
                    network.push((i, item));
                } else {
                    local.push((i, item));
                }
            }

            let mut render_item = |ui: &mut Ui, i: usize, item: &FileEntry| {
                // GATILHO LAZY LOAD PARA PASTAS: Descobre capa se ainda não tem
                if item.is_dir
                    && !ctx.is_computer_view
                    && !ctx.is_recycle_bin_view
                    && item.folder_cover.is_none()
                    && !ctx.scanned_folders.contains(&item.path)
                {
                    ctx.scanned_folders.insert(item.path.clone());
                    ops.request_folder_scan(item.path.clone());
                }

                // GATILHO LAZY LOAD PARA ARQUIVOS DE MÍDIA: Carrega thumbnail proativamente
                if !item.is_dir && !ctx.is_recycle_bin_view {
                    let is_media_file = item
                        .path
                        .extension()
                        .map(|ext| {
                            crate::infrastructure::windows::is_media_extension(
                                &ext.to_string_lossy(),
                            )
                        })
                        .unwrap_or(false);

                    if is_media_file
                        && !ctx.texture_cache.contains(&item.path)
                        && !ctx.loading_set.contains(&item.path)
                        && ctx.loading_set.len() < 50
                    {
                        ctx.loading_set.insert(item.path.clone());
                        ops.request_thumbnail_load(item.path.clone());
                    }
                }

                let is_selected = ctx.selected_item == Some(i);
                let is_recycle_bin = ctx.is_recycle_bin_view;

                ui.push_id(i, |ui| {
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), row_height),
                        Sense::click(),
                    );

                    // Selection and Action
                    if response.clicked() {
                        clicked_item = Some(i);
                    }

                    if response.double_clicked() {
                        double_clicked_item = Some(i);
                    }

                    if response.secondary_clicked() {
                        secondary_clicked_item = Some(i);
                    }

                    // Background Selection
                    if is_selected {
                        // Scroll to selected item if requested (keyboard navigation)
                        if ctx.scroll_to_selected {
                            ui.scroll_to_rect(rect, Some(egui::Align::Center));
                        }
                        ui.painter()
                            .rect_filled(rect, 0.0, crate::ui::theme::COLOR_SELECTION);
                    } else if response.hovered() {
                        ui.painter()
                            .rect_filled(rect, 0.0, crate::ui::theme::color_selection_hover());
                    }

                    // Tooltip at cursor
                    if response.hovered() {
                        let right_bound = available_rect.right();
                        let mouse_pos =
                            ui.input(|i| i.pointer.hover_pos()).unwrap_or_default();

                        // SMART TOOLTIP: Inverte se estiver perto da borda direita (área do player)
                        let tooltip_pos = if mouse_pos.x + 320.0 > right_bound {
                            mouse_pos - egui::vec2(320.0, 0.0)
                        } else {
                            mouse_pos
                        };

                        egui::show_tooltip_at(
                            ui.ctx(),
                            ui.layer_id(),
                            response.id,
                            tooltip_pos,
                            |ui: &mut Ui| {
                                ui.set_max_width(300.0);
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(&item.name).strong());
                                    ui.separator();
                                    ui.label(format!("Tipo: {}", get_file_type_string(item)));
                                    if !item.is_dir {
                                        ui.label(format!("Tamanho: {}", format_size(item.size)));
                                    }
                                    let date_lbl = if is_recycle_bin {
                                        "Data de Exclusão"
                                    } else {
                                        "Última modificação"
                                    };
                                    let date_val = if is_recycle_bin {
                                        item.deletion_date
                                            .clone()
                                            .unwrap_or_else(|| "-".to_string())
                                    } else {
                                        format_date(item.modified)
                                    };
                                    ui.label(format!("{}: {}", date_lbl, date_val));
                                });
                            },
                        );
                    }

                    let text_color = if is_selected {
                        crate::ui::theme::COLOR_SELECTION_TEXT
                    } else {
                        Color32::BLACK
                    };
                    let secondary_color = if is_selected {
                        crate::ui::theme::COLOR_SELECTION_TEXT
                    } else {
                        Color32::from_gray(100)
                    };

                    // 1. Icon + Name
                    let icon_size_px = 16.0;
                    let icon_rect = Rect::from_min_size(
                        rect.min + egui::vec2(4.0, 4.0),
                        egui::vec2(icon_size_px, icon_size_px),
                    );

                    if let Some(_) = &item.drive_info {
                        // Drive: use specialized drive icon loader
                        if let Some(drive_icon) = ctx
                            .item_icon_loader
                            .get_or_load_drive_icon(ui.ctx(), &item.path.to_string_lossy())
                        {
                            ui.painter().image(
                                drive_icon.id(),
                                icon_rect,
                                Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        } else {
                            ui.painter().text(
                                icon_rect.min,
                                egui::Align2::LEFT_TOP,
                                "💽",
                                FontId::proportional(14.0),
                                Color32::GRAY,
                            );
                        }
                    } else if item.is_dir {
                        // folder: Windows native icon
                        if let Some(folder_icon) = ctx.folder_icon_texture {
                            ui.painter().image(
                                folder_icon.id(),
                                icon_rect,
                                Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        } else {
                            ui.painter().text(
                                icon_rect.min,
                                egui::Align2::LEFT_TOP,
                                "\u{ED9F}", // ICON_FOLDER
                                FontId::new(14.0, egui::FontFamily::Name("icons".into())),
                                Color32::from_rgb(255, 193, 7),
                            );
                        }
                    } else {
                        // File: load native Windows icon using IconLoader (same as grid view)
                        if ctx.is_recycle_bin_view {
                            ui.painter().text(
                                icon_rect.min,
                                egui::Align2::LEFT_TOP,
                                "📄",
                                FontId::proportional(14.0),
                                Color32::GRAY,
                            );
                        } else if let Some(file_icon) =
                            ctx.item_icon_loader.get_or_load_icon(ui.ctx(), &item.path)
                        {
                            ui.painter().image(
                                file_icon.id(),
                                icon_rect,
                                Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        } else {
                            ui.painter().text(
                                icon_rect.min,
                                egui::Align2::LEFT_TOP,
                                "\u{ECD3}", // ICON_FILE
                                FontId::new(14.0, egui::FontFamily::Name("icons".into())),
                                Color32::GRAY,
                            );
                        }
                    }

                    // RENAMING LOGIC (LIST VIEW)
                    let is_renaming_this = ctx
                        .renaming_state
                        .as_ref()
                        .map_or(false, |(idx, _)| *idx == i);
                    if is_renaming_this {
                        let mut text = ctx.renaming_state.as_ref().unwrap().1.clone();
                        let name_rect = Rect::from_min_size(
                            rect.min + egui::vec2(24.0, 2.0),
                            egui::vec2(w_name - 30.0, row_height - 4.0),
                        );

                        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(name_rect), |ui| {
                            let response = ui.add(
                                egui::TextEdit::singleline(&mut text)
                                    .frame(true)
                                    .horizontal_align(egui::Align::Min)
                                    .id_source("rename_input_list"),
                            );

                            if ctx.focus_rename {
                                response.request_focus();
                            }

                            // Confirma renomeação com Enter (enquanto tem foco)
                            if response.has_focus()
                                && ui.input(|i_in| i_in.key_pressed(egui::Key::Enter))
                            {
                                ops.rename_with_shell(i);
                            } else if ui.input(|i_in| i_in.key_pressed(egui::Key::Escape)) {
                                // Cancel renaming
                            } else if response.clicked_elsewhere() {
                                // Cancel renaming
                            }
                        });
                    } else {
                        // Name (truncated to fit column - safe UTF-8)
                        let max_name_chars = ((w_name - 30.0) / 7.0) as usize;
                        let display_name: String =
                            if item.name.chars().count() > max_name_chars && max_name_chars > 3 {
                                let truncated: String = item
                                    .name
                                    .chars()
                                    .take(max_name_chars.saturating_sub(3))
                                    .collect();
                                format!("{}...", truncated)
                            } else {
                                item.name.clone()
                            };
                        ui.painter().text(
                            rect.min + egui::vec2(24.0, 5.0),
                            egui::Align2::LEFT_TOP,
                            display_name,
                            FontId::proportional(12.0),
                            text_color,
                        );
                    }

                    if ctx.is_computer_view {
                        // 2. Type
                        let drive_type = if let Some(di) = &item.drive_info {
                            di.drive_type.label().to_string()
                        } else {
                            "Unidade".to_string()
                        };

                        ui.painter().text(
                            Pos2::new(rect.min.x + w_name, rect.min.y + 5.0),
                            egui::Align2::LEFT_TOP,
                            drive_type,
                            FontId::proportional(12.0),
                            secondary_color,
                        );

                        // 3. Total Size
                        let total_str = if let Some(di) = &item.drive_info {
                            format_size(di.total_space)
                        } else {
                            "-".to_string()
                        };
                        ui.painter().text(
                            Pos2::new(rect.min.x + w_name + w_type, rect.min.y + 5.0),
                            egui::Align2::LEFT_TOP,
                            total_str,
                            FontId::proportional(12.0),
                            secondary_color,
                        );

                        // 4. Free Space
                        let free_str = if let Some(di) = &item.drive_info {
                            format_size(di.free_space)
                        } else {
                            "-".to_string()
                        };
                        ui.painter().text(
                            Pos2::new(rect.min.x + w_name + w_type + w_date, rect.min.y + 5.0),
                            egui::Align2::LEFT_TOP,
                            free_str,
                            FontId::proportional(12.0),
                            secondary_color,
                        );
                    } else {
                        // 2. Date
                        let date_str = if ctx.is_recycle_bin_view {
                            item.deletion_date
                                .clone()
                                .unwrap_or_else(|| "-".to_string())
                        } else {
                            crate::infrastructure::windows::formatting::format_date(item.modified)
                        };

                        ui.painter().text(
                            Pos2::new(rect.min.x + w_name, rect.min.y + 5.0),
                            egui::Align2::LEFT_TOP,
                            date_str,
                            FontId::proportional(12.0),
                            secondary_color,
                        );

                        // 3. Type (truncated)
                        let type_str = get_file_type_string(item);
                        let max_type_chars = 14; // ~100px at 7px per char
                        let display_type: String = if type_str.chars().count() > max_type_chars {
                            type_str
                                .chars()
                                .take(max_type_chars - 2)
                                .collect::<String>()
                                + ".."
                        } else {
                            type_str
                        };
                        ui.painter().text(
                            Pos2::new(rect.min.x + w_name + w_date, rect.min.y + 5.0),
                            egui::Align2::LEFT_TOP,
                            display_type,
                            FontId::proportional(12.0),
                            secondary_color,
                        );

                        // 4. Size
                        let size_str = if item.is_dir {
                            "".to_string()
                        } else {
                            format_size(item.size)
                        };
                        ui.painter().text(
                            Pos2::new(rect.min.x + w_name + w_date + w_type, rect.min.y + 5.0),
                            egui::Align2::LEFT_TOP,
                            size_str,
                            FontId::proportional(12.0),
                            secondary_color,
                        );

                        // 5. OneDrive Status (if in OneDrive folder)
                        if ctx.is_onedrive_folder {
                            render_status_badge(
                                ui,
                                Pos2::new(
                                    rect.min.x + w_name + w_date + w_type + w_size + 8.0,
                                    rect.min.y + 4.0,
                                ),
                                item.sync_status,
                            );
                        }
                    }
                });
            };

            if !local.is_empty() {
                render_section_header(ui, "Discos locais");
                for (i, item) in local {
                    render_item(ui, i, item);
                }
                ui.add_space(10.0);
            }

            if !network.is_empty() {
                render_section_header(ui, "Unidades de rede");
                for (i, item) in network {
                    render_item(ui, i, item);
                }
                ui.add_space(10.0);
            }
        });

        if ui.input(|i| i.pointer.secondary_clicked()) {
            if let Some(pos) = ui.ctx().pointer_latest_pos() {
                if available_rect.contains(pos) {
                    empty_area_secondary_click = true;
                }
            }
        }
    } else {
        // Regular virtualized list
        let _scroll_res = scroll_area.show_rows(ui, row_height + 2.0, total_rows, |ui, row_range| {
            let mut render_item = |ui: &mut Ui, i: usize, item: &FileEntry| {
                // GATILHO LAZY LOAD PARA PASTAS: Descobre capa se ainda não tem
                if item.is_dir
                    && !ctx.is_computer_view
                    && item.folder_cover.is_none()
                    && !ctx.scanned_folders.contains(&item.path)
                {
                    ctx.scanned_folders.insert(item.path.clone());
                    ops.request_folder_scan(item.path.clone());
                }

                // GATILHO LAZY LOAD PARA ARQUIVOS DE MÍDIA: Carrega thumbnail proativamente
                if !item.is_dir {
                    let is_media_file = item
                        .path
                        .extension()
                        .map(|ext| {
                            crate::infrastructure::windows::is_media_extension(
                                &ext.to_string_lossy(),
                            )
                        })
                        .unwrap_or(false);

                    if is_media_file
                        && !ctx.texture_cache.contains(&item.path)
                        && !ctx.loading_set.contains(&item.path)
                        && ctx.loading_set.len() < 50
                    {
                        ctx.loading_set.insert(item.path.clone());
                        ops.request_thumbnail_load(item.path.clone());
                    }
                }

                let is_selected = ctx.selected_item == Some(i);
                let is_recycle_bin_virt = ctx.is_recycle_bin_view;

                ui.push_id(i, |ui| {
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), row_height),
                        Sense::click(),
                    );

                    // Selection and Action
                    if response.clicked() {
                        clicked_item = Some(i);
                    }

                    if response.double_clicked() {
                        double_clicked_item = Some(i);
                    }

                    if response.secondary_clicked() {
                        secondary_clicked_item = Some(i);
                    }

                    // Background Selection
                    if is_selected {
                        // Scroll to selected item if requested (keyboard navigation)
                        if ctx.scroll_to_selected {
                            ui.scroll_to_rect(rect, Some(egui::Align::Center));
                        }
                        ui.painter()
                            .rect_filled(rect, 0.0, crate::ui::theme::COLOR_SELECTION);
                    } else if response.hovered() {
                        ui.painter()
                            .rect_filled(rect, 0.0, crate::ui::theme::color_selection_hover());
                    }

                    // Tooltip at cursor
                    if response.hovered() {
                        let right_bound = available_rect.right();
                        let mouse_pos =
                            ui.input(|i| i.pointer.hover_pos()).unwrap_or_default();

                        // SMART TOOLTIP: Inverte se estiver perto da borda direita (área do player)
                        let tooltip_pos = if mouse_pos.x + 320.0 > right_bound {
                            mouse_pos - egui::vec2(320.0, 0.0)
                        } else {
                            mouse_pos
                        };

                        egui::show_tooltip_at(
                            ui.ctx(),
                            ui.layer_id(),
                            response.id,
                            tooltip_pos,
                            |ui: &mut Ui| {
                                ui.set_max_width(300.0);
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(&item.name).strong());
                                    ui.separator();
                                    ui.label(format!("Tipo: {}", get_file_type_string(item)));
                                    if !item.is_dir {
                                        ui.label(format!("Tamanho: {}", format_size(item.size)));
                                    }
                                    let date_lbl = if is_recycle_bin_virt {
                                        "Data de Exclusão"
                                    } else {
                                        "Última modificação"
                                    };
                                    let date_val = if is_recycle_bin_virt {
                                        item.deletion_date
                                            .clone()
                                            .unwrap_or_else(|| "-".to_string())
                                    } else {
                                        format_date(item.modified)
                                    };
                                    ui.label(format!("{}: {}", date_lbl, date_val));
                                });
                            },
                        );
                    }

                    let text_color = if is_selected {
                        crate::ui::theme::COLOR_SELECTION_TEXT
                    } else {
                        Color32::BLACK
                    };
                    let secondary_color = if is_selected {
                        crate::ui::theme::COLOR_SELECTION_TEXT
                    } else {
                        Color32::from_gray(100)
                    };

                    // 1. Icon + Name
                    let icon_size_px = 16.0;
                    let icon_rect = Rect::from_min_size(
                        rect.min + egui::vec2(4.0, 4.0),
                        egui::vec2(icon_size_px, icon_size_px),
                    );

                    if let Some(_) = &item.drive_info {
                        // Drive: use specialized drive icon loader
                        if let Some(drive_icon) = ctx
                            .item_icon_loader
                            .get_or_load_drive_icon(ui.ctx(), &item.path.to_string_lossy())
                        {
                            ui.painter().image(
                                drive_icon.id(),
                                icon_rect,
                                Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        } else {
                            ui.painter().text(
                                icon_rect.min,
                                egui::Align2::LEFT_TOP,
                                "💽",
                                FontId::proportional(14.0),
                                Color32::GRAY,
                            );
                        }
                    } else if item.is_dir {
                        // folder: Windows native icon
                        if let Some(folder_icon) = ctx.folder_icon_texture {
                            ui.painter().image(
                                folder_icon.id(),
                                icon_rect,
                                Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        } else {
                            ui.painter().text(
                                icon_rect.min,
                                egui::Align2::LEFT_TOP,
                                "\u{ED9F}", // ICON_FOLDER
                                FontId::new(14.0, egui::FontFamily::Name("icons".into())),
                                Color32::from_rgb(255, 193, 7),
                            );
                        }
                    } else {
                        // File: load native Windows icon using IconLoader (same as grid view)
                        if let Some(file_icon) =
                            ctx.item_icon_loader.get_or_load_icon(ui.ctx(), &item.path)
                        {
                            ui.painter().image(
                                file_icon.id(),
                                icon_rect,
                                Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        } else {
                            ui.painter().text(
                                icon_rect.min,
                                egui::Align2::LEFT_TOP,
                                "\u{ECD3}", // ICON_FILE
                                FontId::new(14.0, egui::FontFamily::Name("icons".into())),
                                Color32::GRAY,
                            );
                        }
                    }

                    // RENAMING LOGIC (LIST VIEW)
                    let is_renaming_this = ctx
                        .renaming_state
                        .as_ref()
                        .map_or(false, |(idx, _)| *idx == i);
                    if is_renaming_this {
                        let mut text = ctx.renaming_state.as_ref().unwrap().1.clone();
                        let name_rect = Rect::from_min_size(
                            rect.min + egui::vec2(24.0, 2.0),
                            egui::vec2(w_name - 30.0, row_height - 4.0),
                        );

                        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(name_rect), |ui| {
                            let response = ui.add(
                                egui::TextEdit::singleline(&mut text)
                                    .frame(true)
                                    .horizontal_align(egui::Align::Min)
                                    .id_source("rename_input_list"),
                            );

                            if ctx.focus_rename {
                                response.request_focus();
                            }

                            // Confirma renomeação com Enter (enquanto tem foco)
                            if response.has_focus()
                                && ui.input(|i_in| i_in.key_pressed(egui::Key::Enter))
                            {
                                ops.rename_with_shell(i);
                            } else if ui.input(|i_in| i_in.key_pressed(egui::Key::Escape)) {
                                // Cancel renaming
                            } else if response.clicked_elsewhere() {
                                // Cancel renaming
                            }
                        });
                    } else {
                        // Name (truncated to fit column - safe UTF-8)
                        let max_name_chars = ((w_name - 30.0) / 7.0) as usize;
                        let display_name: String =
                            if item.name.chars().count() > max_name_chars && max_name_chars > 3 {
                                let truncated: String = item
                                    .name
                                    .chars()
                                    .take(max_name_chars.saturating_sub(3))
                                    .collect();
                                format!("{}...", truncated)
                            } else {
                                item.name.clone()
                            };
                        ui.painter().text(
                            rect.min + egui::vec2(24.0, 5.0),
                            egui::Align2::LEFT_TOP,
                            display_name,
                            FontId::proportional(12.0),
                            text_color,
                        );
                    }

                    if ctx.is_computer_view {
                        // 2. Type
                        let drive_type = if let Some(di) = &item.drive_info {
                            di.drive_type.label().to_string()
                        } else {
                            "Unidade".to_string()
                        };

                        ui.painter().text(
                            Pos2::new(rect.min.x + w_name, rect.min.y + 5.0),
                            egui::Align2::LEFT_TOP,
                            drive_type,
                            FontId::proportional(12.0),
                            secondary_color,
                        );

                        // 3. Total Size
                        let total_str = if let Some(di) = &item.drive_info {
                            format_size(di.total_space)
                        } else {
                            "-".to_string()
                        };
                        ui.painter().text(
                            Pos2::new(rect.min.x + w_name + w_type, rect.min.y + 5.0),
                            egui::Align2::LEFT_TOP,
                            total_str,
                            FontId::proportional(12.0),
                            secondary_color,
                        );

                        // 4. Free Space
                        let free_str = if let Some(di) = &item.drive_info {
                            format_size(di.free_space)
                        } else {
                            "-".to_string()
                        };
                        ui.painter().text(
                            Pos2::new(rect.min.x + w_name + w_type + w_date, rect.min.y + 5.0),
                            egui::Align2::LEFT_TOP,
                            free_str,
                            FontId::proportional(12.0),
                            secondary_color,
                        );
                    } else {
                        // 2. Date
                        let date_str = if ctx.is_recycle_bin_view {
                            item.deletion_date
                                .clone()
                                .unwrap_or_else(|| "-".to_string())
                        } else {
                            format_date(item.modified)
                        };
                        ui.painter().text(
                            Pos2::new(rect.min.x + w_name, rect.min.y + 5.0),
                            egui::Align2::LEFT_TOP,
                            date_str,
                            FontId::proportional(12.0),
                            secondary_color,
                        );

                        // 3. Type (truncated)
                        let type_str = get_file_type_string(item);
                        let max_type_chars = 14; // ~100px at 7px per char
                        let display_type: String = if type_str.chars().count() > max_type_chars {
                            type_str
                                .chars()
                                .take(max_type_chars - 2)
                                .collect::<String>()
                                + ".."
                        } else {
                            type_str
                        };
                        ui.painter().text(
                            Pos2::new(rect.min.x + w_name + w_date, rect.min.y + 5.0),
                            egui::Align2::LEFT_TOP,
                            display_type,
                            FontId::proportional(12.0),
                            secondary_color,
                        );

                        // 4. Size
                        let size_str = if item.is_dir {
                            "".to_string()
                        } else {
                            format_size(item.size)
                        };
                        ui.painter().text(
                            Pos2::new(rect.min.x + w_name + w_date + w_type, rect.min.y + 5.0),
                            egui::Align2::LEFT_TOP,
                            size_str,
                            FontId::proportional(12.0),
                            secondary_color,
                        );

                        // 5. OneDrive Status (if in OneDrive folder)
                        if ctx.is_onedrive_folder {
                            render_status_badge(
                                ui,
                                Pos2::new(
                                    rect.min.x + w_name + w_date + w_type + w_size + 8.0,
                                    rect.min.y + 4.0,
                                ),
                                item.sync_status,
                            );
                        }
                    }
                });
            };
            for i in row_range {
                if i >= ctx.items.len() {
                    break;
                }
                render_item(ui, i, &ctx.items[i]);
            }
        });

        if ui.input(|i| i.pointer.secondary_clicked()) {
            if let Some(pos) = ui.ctx().pointer_latest_pos() {
                if available_rect.contains(pos) {
                    empty_area_secondary_click = true;
                }
            }
        }
    }

    // Capture secondary click on the scroll area if no item was clicked
    if empty_area_secondary_click && secondary_clicked_item.is_none() {
        return Some(ListViewAction::EmptyAreaSecondaryClick);
    }

    // Header helper
    fn render_section_header(ui: &mut Ui, title: &str) {
        ui.add_space(8.0);
        ui.label(
            RichText::new(title)
                .size(11.0)
                .color(Color32::from_gray(120))
                .strong(),
        );
        ui.add_space(4.0);
    }

    // Handle actions after rendering - ORDER MATTERS!
    // Sort header clicks take priority
    if let Some(mode) = sort_action {
        return Some(ListViewAction::SortChange(mode));
    }

    // double_clicked and secondary_clicked must be checked BEFORE clicked
    // because clicked() also returns true on double-click
    if let Some(idx) = double_clicked_item {
        return Some(ListViewAction::DoubleClick(idx));
    }

    if let Some(idx) = secondary_clicked_item {
        return Some(ListViewAction::SecondaryClick(idx));
    }

    if let Some(idx) = clicked_item {
        return Some(ListViewAction::Click(idx));
    }

    None
}

/// Helper function to get file type string
fn get_file_type_string(item: &FileEntry) -> String {
    if item.is_dir {
        return "Pasta".to_string();
    }
    if let Some(ext) = item.path.extension() {
        return format!("Arquivo {}", ext.to_string_lossy().to_uppercase());
    }
    "Arquivo".to_string()
}

/// Renders a sync status badge (OneDrive) in the status column
fn render_status_badge(ui: &mut egui::Ui, pos: Pos2, status: SyncStatus) {
    if status == SyncStatus::None {
        return; // No badge for normal files
    }

    let badge_size = 16.0;
    let badge_center = pos + egui::vec2(badge_size / 2.0, badge_size / 2.0);
    let badge_radius = badge_size / 2.0;

    let painter = ui.painter();

    match status {
        SyncStatus::CloudOnly => {
            // Blue cloud icon - file needs download
            painter.circle_filled(badge_center, badge_radius, Color32::from_rgb(0, 120, 215));
            painter.text(
                badge_center,
                egui::Align2::CENTER_CENTER,
                "☁",
                FontId::proportional(11.0),
                Color32::WHITE,
            );
        }
        SyncStatus::Syncing => {
            // Blue circular arrows - file is being synced
            painter.circle_filled(badge_center, badge_radius, Color32::from_rgb(0, 120, 215));
            painter.text(
                badge_center,
                egui::Align2::CENTER_CENTER,
                "⟳",
                FontId::proportional(12.0),
                Color32::WHITE,
            );
        }
        SyncStatus::Pinned => {
            // Green solid circle with check - always keep on device
            painter.circle_filled(badge_center, badge_radius, Color32::from_rgb(0, 150, 0));
            painter.text(
                badge_center,
                egui::Align2::CENTER_CENTER,
                "✓",
                FontId::proportional(10.0),
                Color32::WHITE,
            );
        }
        SyncStatus::LocallyAvailable => {
            // White circle with green outline/check - downloaded on demand
            painter.circle_filled(badge_center, badge_radius, Color32::WHITE);
            painter.circle_stroke(
                badge_center,
                badge_radius - 1.0,
                egui::Stroke::new(2.0, Color32::from_rgb(0, 150, 0)),
            );
            painter.text(
                badge_center,
                egui::Align2::CENTER_CENTER,
                "✓",
                FontId::proportional(10.0),
                Color32::from_rgb(0, 150, 0),
            );
        }
        SyncStatus::None => {} // Already handled above
    }
}
