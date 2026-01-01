//! List view rendering
//! Follows .cursorrules: single responsibility, < 300 lines

use eframe::egui::{self, Color32, FontId, Pos2, Rect, RichText, Sense, Ui};
use std::path::PathBuf;

use crate::domain::file_entry::{FileEntry, SortMode};
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
    pub is_computer_view: bool,
    pub texture_cache: &'a mut lru::LruCache<PathBuf, egui::TextureHandle>,
    pub loading_set: &'a mut std::collections::HashSet<PathBuf>,
    pub scanned_folders: &'a mut std::collections::HashSet<PathBuf>,
    pub folder_icon_texture: Option<&'a egui::TextureHandle>,
    pub computer_icon: Option<&'a egui::TextureHandle>,
    pub drive_icon_cache: &'a mut lru::LruCache<String, egui::TextureHandle>,
    pub item_icon_loader: &'a mut crate::ui::icon_loader::IconLoader,
}

/// Action returned by list view
pub enum ListViewAction {
    Click(usize),
    DoubleClick(usize),
    SecondaryClick(usize),
    SortChange(SortMode),
}

/// Operations that can be performed from list view
pub trait ListViewOperations {
    fn navigate_to(&mut self, path: &str);
    fn open_with_shell(&mut self, path: &PathBuf);
    fn request_thumbnail_load(&mut self, path: PathBuf);
    fn request_folder_scan(&mut self, path: PathBuf);
    fn rename_with_shell(&mut self, idx: usize);
    fn get_or_load_icon(
        &mut self,
        ctx: &egui::Context,
        path: &std::path::Path,
    ) -> Option<egui::TextureHandle>;
}

/// Renders the list view
pub fn render_list_view(
    ui: &mut Ui,
    ctx: &mut ListViewContext,
    ops: &mut dyn ListViewOperations,
) -> Option<ListViewAction> {
    let row_height = 24.0;
    let available_w = ui.available_width();
    
    // Column widths
    let w_name = (available_w - 410.0).max(200.0);
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
                let text_color = if is_active { Color32::BLACK } else { Color32::from_gray(100) };
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
        if clicked_name { return Some(SortMode::Name); }
        
        if ctx.is_computer_view {
            let (clicked_type, _) = draw_header(ui, "Tipo", w_type, SortMode::Type);
            if clicked_type { return Some(SortMode::Type); }
            
            let (clicked_total, _) = draw_header(ui, "Espaço Total", w_date, SortMode::Size);
            if clicked_total { return Some(SortMode::Size); }
            
            let (clicked_free, _) = draw_header(ui, "Espaço Livre", w_size, SortMode::Size);
            if clicked_free { return Some(SortMode::Size); }
        } else {
            let (clicked_date, _) = draw_header(ui, "Última modificação", w_date, SortMode::Date);
            if clicked_date { return Some(SortMode::Date); }
            
            let (clicked_type, _) = draw_header(ui, "Tipo", w_type, SortMode::Type);
            if clicked_type { return Some(SortMode::Type); }
            
            let (clicked_size, _) = draw_header(ui, "Tamanho", w_size, SortMode::Size);
            if clicked_size { return Some(SortMode::Size); }
        }
        
        None
    }).inner.map(|mode| sort_action = Some(mode));
    
    ui.separator();

    // Virtualized list
    let total_rows = ctx.items.len();
    let mut clicked_item = None;
    let mut double_clicked_item = None;
    let mut secondary_clicked_item = None;
    
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(
        ui,
        row_height + 2.0,
        total_rows,
        |ui, row_range| {
            for i in row_range {
                if i >= ctx.items.len() { break; }
                let item = &ctx.items[i];
                let is_selected = ctx.selected_item == Some(i);

                ui.push_id(i, |ui| {
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), row_height), 
                        Sense::click()
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
                        ui.painter().rect_filled(rect, 0.0, Color32::from_rgb(205, 232, 255));
                    } else if response.hovered() {
                        ui.painter().rect_filled(rect, 0.0, Color32::from_gray(245));
                    }

                    // Tooltip at cursor
                    if response.hovered() {
                        egui::show_tooltip_at_pointer(ui.ctx(), ui.layer_id(), response.id, |ui: &mut Ui| {
                            ui.set_max_width(300.0);
                            ui.vertical(|ui| {
                                ui.label(RichText::new(&item.name).strong());
                                ui.separator();
                                ui.label(format!("Tipo: {}", get_file_type_string(item)));
                                if !item.is_dir {
                                    ui.label(format!("Tamanho: {}", format_size(item.size)));
                                }
                                ui.label(format!("Última modificação: {}", format_date(item.modified)));
                            });
                        });
                    }

                    let text_color = Color32::BLACK;
                    let secondary_color = Color32::from_gray(100);
                    
                    // 1. Icon + Name
                    let icon_size_px = 16.0;
                    let icon_rect = Rect::from_min_size(
                        rect.min + egui::vec2(4.0, 4.0),
                        egui::vec2(icon_size_px, icon_size_px)
                    );
                    
                    if let Some(_) = &item.drive_info {
                        // Drive: use specialized drive icon loader
                        if let Some(drive_icon) = ctx.item_icon_loader.get_or_load_drive_icon(ui.ctx(), &item.path.to_string_lossy()) {
                            ui.painter().image(
                                drive_icon.id(),
                                icon_rect,
                                Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                                Color32::WHITE
                            );
                        } else {
                            ui.painter().text(
                                icon_rect.min,
                                egui::Align2::LEFT_TOP,
                                "💽",
                                FontId::proportional(14.0),
                                Color32::GRAY
                            );
                        }
                    } else if item.is_dir {
                        // folder: Windows native icon
                        if let Some(folder_icon) = ctx.folder_icon_texture {
                            ui.painter().image(
                                folder_icon.id(),
                                icon_rect,
                                Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                                Color32::WHITE
                            );
                        } else {
                            ui.painter().text(
                                icon_rect.min,
                                egui::Align2::LEFT_TOP,
                                "\u{ED9F}", // ICON_FOLDER
                                FontId::new(14.0, egui::FontFamily::Name("icons".into())),
                                Color32::from_rgb(255, 193, 7)
                            );
                        }
                    } else {
                        // File: try to load native icon
                        if let Some(file_icon) = ops.get_or_load_icon(ui.ctx(), &item.path) {
                            ui.painter().image(
                                file_icon.id(),
                                icon_rect,
                                Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                                Color32::WHITE
                            );
                        } else {
                            ui.painter().text(
                                icon_rect.min,
                                egui::Align2::LEFT_TOP,
                                "\u{ECD3}", // ICON_FILE
                                FontId::new(14.0, egui::FontFamily::Name("icons".into())),
                                Color32::GRAY
                            );
                        }
                    }

                    // RENAMING LOGIC (LIST VIEW)
                    let is_renaming_this = ctx.renaming_state.as_ref().map_or(false, |(idx, _)| *idx == i);
                    if is_renaming_this {
                        let mut text = ctx.renaming_state.as_ref().unwrap().1.clone();
                        let name_rect = Rect::from_min_size(
                            rect.min + egui::vec2(24.0, 2.0),
                            egui::vec2(w_name - 30.0, row_height - 4.0)
                        );
                        
                        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(name_rect), |ui| {
                            let response = ui.add(egui::TextEdit::singleline(&mut text)
                                .frame(true)
                                .horizontal_align(egui::Align::Min)
                                .id_source("rename_input_list"));
                            
                            if ctx.focus_rename {
                                response.request_focus();
                            }

                            // Confirma renomeação com Enter (enquanto tem foco)
                            if response.has_focus() && ui.input(|i_in| i_in.key_pressed(egui::Key::Enter)) {
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
                        let display_name: String = if item.name.chars().count() > max_name_chars && max_name_chars > 3 {
                            let truncated: String = item.name.chars().take(max_name_chars.saturating_sub(3)).collect();
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
                            if !di.file_system.is_empty() {
                                format!("Disco Local ({})", di.file_system)
                            } else {
                                "Disco Local".to_string()
                            }
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
                        ui.painter().text(
                            Pos2::new(rect.min.x + w_name, rect.min.y + 5.0),
                            egui::Align2::LEFT_TOP,
                            format_date(item.modified),
                            FontId::proportional(12.0),
                            secondary_color,
                        );

                        // 3. Type (truncated)
                        let type_str = get_file_type_string(item);
                        let max_type_chars = 14; // ~100px at 7px per char
                        let display_type: String = if type_str.chars().count() > max_type_chars {
                            type_str.chars().take(max_type_chars - 2).collect::<String>() + ".."
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
                        let size_str = if item.is_dir { "".to_string() } else { format_size(item.size) };
                        ui.painter().text(
                            Pos2::new(rect.min.x + w_name + w_date + w_type, rect.min.y + 5.0),
                            egui::Align2::LEFT_TOP,
                            size_str,
                            FontId::proportional(12.0),
                            secondary_color,
                        );
                    }
                });
            }
        }
    );
    
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
