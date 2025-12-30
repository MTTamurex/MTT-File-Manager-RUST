//! List view rendering
//! Follows .cursorrules: single responsibility, < 300 lines

use eframe::egui;

use crate::domain::file_entry::{FileEntry, SortMode};
use crate::ui::app::ImageViewerApp;
use super::common::{get_file_type_string, format_date, format_size, open_with_shell};

impl ImageViewerApp {
    /// Renders the list view.
    pub fn render_list_view(&mut self, ui: &mut egui::Ui) {
        let row_height = 24.0;
        let available_w = ui.available_width();
        
        // Column widths
        let w_name = (available_w - 410.0).max(200.0);
        let w_date = 170.0;
        let w_type = 120.0;
        let w_size = 100.0;
        
        // Table Header
        ui.horizontal(|ui| {
            ui.style_mut().spacing.item_spacing.x = 0.0;
            
            let mut draw_header = |ui: &mut egui::Ui, text: &str, width: f32, mode: SortMode| {
                let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 22.0), egui::Sense::click());
                let is_active = self.sort_mode == mode;
                
                if ui.is_rect_visible(rect) {
                    if is_active {
                        ui.painter().rect_filled(rect, 2.0, egui::Color32::from_gray(230));
                    }
                    let text_color = if is_active { egui::Color32::BLACK } else { egui::Color32::from_gray(100) };
                    ui.painter().text(
                        rect.min + egui::vec2(8.0, 4.0),
                        egui::Align2::LEFT_TOP,
                        text,
                        egui::FontId::proportional(12.0),
                        text_color,
                    );
                    if is_active {
                        let arrow = if self.sort_descending { "v" } else { "^" };
                        ui.painter().text(
                            rect.max - egui::vec2(15.0, 8.0),
                            egui::Align2::CENTER_CENTER,
                            arrow,
                            egui::FontId::proportional(10.0),
                            text_color,
                        );
                    }
                }
                
                if response.clicked() {
                    if self.sort_mode == mode {
                        self.sort_descending = !self.sort_descending;
                    } else {
                        self.sort_mode = mode;
                        self.sort_descending = false;
                    }
                    self.sort_items();
                }
            };

            draw_header(ui, "Nome", w_name, SortMode::Name);
            draw_header(ui, "Última modificação", w_date, SortMode::Date);
            draw_header(ui, "Tipo", w_type, SortMode::Name); // Type uses Name sort secondary
            draw_header(ui, "Tamanho", w_size, SortMode::Size);
        });
        
        ui.separator();

        // Virtualized List
        let total_rows = self.items.len();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(
            ui,
            row_height + 2.0,
            total_rows,
            |ui, row_range| {
                for i in row_range {
                    if i >= self.items.len() { break; }
                    let item = self.items[i].clone();
                    let is_selected = self.selected_item_index == Some(i);

                    ui.push_id(i, |ui| {
                        let (rect, response) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), row_height), 
                            egui::Sense::click()
                        );

                        // Selection and Action
                        if response.clicked() {
                            self.selected_item_index = Some(i);
                            self.selected_file = Some(item.clone());
                            
                            // Trigger thumbnail load for sidebar preview
                            if !item.is_dir {
                                if !self.texture_cache.contains(&item.path) && !self.loading_set.contains(&item.path) {
                                    self.request_thumbnail_load(item.path.clone());
                                }
                            }
                        }
                        if response.double_clicked() {
                            if item.is_dir {
                                self.navigate_to(&item.path.to_string_lossy());
                            } else {
                                open_with_shell(&item.path);
                            }
                        }
                        
                        // Right click: open context menu and select item
                        if response.secondary_clicked() {
                            // Select item visually
                            self.selected_item_index = Some(i);
                            self.selected_file = Some(item.clone());
                            
                            // Open context menu
                            self.context_menu.open(
                                response.interact_pointer_pos()
                                    .unwrap_or_else(|| ui.ctx().pointer_latest_pos().unwrap_or(egui::Pos2::ZERO)),
                                Some(i),
                                Some(item.path.clone()),
                                false,
                            );
                        }

                        // Background Selection
                        if is_selected {
                            ui.painter().rect_filled(rect, 0.0, egui::Color32::from_rgb(205, 232, 255));
                        } else if response.hovered() {
                            ui.painter().rect_filled(rect, 0.0, egui::Color32::from_gray(245));
                        }

                        // Tooltip at cursor
                        if response.hovered() {
                            egui::show_tooltip_at_pointer(ui.ctx(), ui.layer_id(), response.id, |ui: &mut egui::Ui| {
                                ui.set_max_width(300.0);
                                ui.vertical(|ui| {
                                    ui.label(egui::RichText::new(&item.name).strong());
                                    ui.separator();
                                    ui.label(format!("Tipo: {}", get_file_type_string(&item)));
                                    if !item.is_dir {
                                        ui.label(format!("Tamanho: {}", format_size(item.size)));
                                    }
                                    ui.label(format!("Última modificação: {}", format_date(item.modified)));
                                });
                            });
                        }

                        let text_color = egui::Color32::BLACK;
                        let secondary_color = egui::Color32::from_gray(100);
                        
                        // 1. Icon + Name
                        let icon_size_px = 16.0;
                        let icon_rect = egui::Rect::from_min_size(
                            rect.min + egui::vec2(4.0, 4.0),
                            egui::vec2(icon_size_px, icon_size_px)
                        );
                        
                        if item.is_dir {
                            // folder: native Windows icon
                            self.ensure_folder_icon(ui.ctx());
                            if let Some(folder_icon) = &self.folder_icon_texture {
                                ui.painter().image(
                                    folder_icon.id(),
                                    icon_rect,
                                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                    egui::Color32::WHITE
                                );
                            } else {
                                ui.painter().text(icon_rect.min, egui::Align2::LEFT_TOP, crate::ui::components::ICON_FOLDER, egui::FontId::new(14.0, egui::FontFamily::Name("icons".into())), egui::Color32::from_rgb(255, 193, 7));
                            }
                        } else {
                            // File: try to load native icon
                            if let Some(file_icon) = self.get_or_load_icon(ui.ctx(), &item.path) {
                                ui.painter().image(
                                    file_icon.id(),
                                    icon_rect,
                                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                    egui::Color32::WHITE
                                );
                            } else {
                                ui.painter().text(icon_rect.min, egui::Align2::LEFT_TOP, crate::ui::components::ICON_FILE, egui::FontId::new(14.0, egui::FontFamily::Name("icons".into())), egui::Color32::GRAY);
                            }
                        }

                        // RENAMING VISUAL LOGIC (LIST VIEW)
                        let is_renaming_this = self.renaming_state.as_ref().map_or(false, |state| state.item_index == i);
                        if is_renaming_this {
                            let mut text = self.renaming_state.as_mut().unwrap().new_name.clone();
                            let name_rect = egui::Rect::from_min_size(
                                rect.min + egui::vec2(24.0, 2.0),
                                egui::vec2(w_name - 30.0, row_height - 4.0)
                            );
                            
                            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(name_rect), |ui| {
                                let response = ui.add(egui::TextEdit::singleline(&mut text)
                                    .frame(true)
                                    .horizontal_align(egui::Align::Min)
                                    .id_source("rename_input_list"));
                                
                                self.renaming_state.as_mut().unwrap().new_name = text;

                                if self.renaming_state.as_ref().unwrap().focus_requested() {
                                    response.request_focus();
                                    self.renaming_state.as_mut().unwrap().mark_focus_handled();
                                }

                                if response.lost_focus() && ui.input(|i_in| i_in.key_pressed(egui::Key::Enter)) {
                                    self.rename_with_shell(i);
                                } else if ui.input(|i_in| i_in.key_pressed(egui::Key::Escape)) {
                                    self.renaming_state = None;
                                } else if response.clicked_elsewhere() {
                                    self.renaming_state = None;
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
                                egui::FontId::proportional(12.0),
                                text_color,
                            );
                        }

                        // 2. Date
                        ui.painter().text(
                            egui::pos2(rect.min.x + w_name, rect.min.y + 5.0),
                            egui::Align2::LEFT_TOP,
                            format_date(item.modified),
                            egui::FontId::proportional(12.0),
                            secondary_color,
                        );

                        // 3. Type (truncated)
                        let type_str = get_file_type_string(&item);
                        let max_type_chars = 14; // ~100px at 7px per char
                        let display_type: String = if type_str.chars().count() > max_type_chars {
                            type_str.chars().take(max_type_chars - 2).collect::<String>() + ".."
                        } else {
                            type_str
                        };
                        ui.painter().text(
                            egui::pos2(rect.min.x + w_name + w_date, rect.min.y + 5.0),
                            egui::Align2::LEFT_TOP,
                            display_type,
                            egui::FontId::proportional(12.0),
                            secondary_color,
                        );

                        // 4. Size
                        let size_str = if item.is_dir { "".to_string() } else { format_size(item.size) };
                        ui.painter().text(
                            egui::pos2(rect.min.x + w_name + w_date + w_type, rect.min.y + 5.0),
                            egui::Align2::LEFT_TOP,
                            size_str,
                            egui::FontId::proportional(12.0),
                            secondary_color,
                        );
                    });
                }
            }
        );
    }
}
