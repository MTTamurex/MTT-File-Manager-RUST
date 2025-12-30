//! Grid view rendering
//! Follows .cursorrules: single responsibility, < 300 lines

use eframe::egui;

use crate::domain::file_entry::FileEntry;
use crate::ui::app::ImageViewerApp;
use super::common::{get_file_type_string, format_date, format_size, open_with_shell};

impl ImageViewerApp {
    /// Renders the grid view.
    pub fn render_grid_view(&mut self, ui: &mut egui::Ui) {
        // If we're in "Este Computador" view, show drives
        if self.is_computer_view {
            self.render_computer_view(ui);
            return;
        }
        
        let padding = 8.0;
        let item_w = self.thumbnail_size;
        let item_h = self.thumbnail_size + 20.0;  // Height: thumb + text
        let available_w = ui.available_width();
        let cols = ((available_w - padding) / (item_w + padding)).floor().max(1.0) as usize;
        self.last_grid_cols = cols;
        
        // Keyboard Navigation
        if ui.input(|i| i.focused) {
            let current_index = self.items.iter().position(|x| self.selected_file.as_ref().map_or(false, |f| f.path == x.path));
            // Keyboard navigation (ONLY IF NOT RENAMING)
            if self.renaming_state.is_none() {
                let mut new_index = None;
                if ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) { 
                    new_index = current_index.map(|idx| idx + 1); 
                }
                else if ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) { 
                    new_index = current_index.map(|idx| idx.saturating_sub(1)); 
                }
                else if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) { 
                    new_index = current_index.map(|idx| idx + cols).or(Some(0)); 
                }
                else if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) { 
                    new_index = current_index.map(|idx| idx.saturating_sub(cols)); 
                }

                if let Some(idx) = new_index {
                    let clamped = idx.min(self.items.len().saturating_sub(1));
                    if let Some(item) = self.items.get(clamped) {
                        self.selected_file = Some(item.clone());
                        self.selected_item_index = Some(clamped);
                    }
                }
            }
            
            // Enter to open (only if not renaming)
            if self.renaming_state.is_none() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                if let Some(selected) = &self.selected_file.clone() {
                    if selected.is_dir {
                        self.navigate_to(&selected.path.to_string_lossy());
                    } else {
                        open_with_shell(&selected.path);
                    }
                }
            }
        }

        // Virtualized Grid
        let count = self.items.len();
        let rows = (count as f32 / cols as f32).ceil() as usize;
        let total_height = rows as f32 * (item_h + padding) + padding;
        
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let content_min = ui.min_rect().min;
            ui.allocate_rect(egui::Rect::from_min_size(content_min, egui::vec2(available_w, total_height)), egui::Sense::hover());
            
            let clip_rect = ui.clip_rect();
            let start_y = (clip_rect.top() - content_min.y).max(0.0);
            let end_y = start_y + clip_rect.height();
            
            let visible_min_row = (start_y / (item_h + padding)).floor() as usize;
            let visible_max_row = ((end_y / (item_h + padding)).ceil() as usize + 1).min(rows);
            
            let loop_min_row = visible_min_row.saturating_sub(2);
            let loop_max_row = (visible_max_row + 2).min(rows);
            
            'row_loop: for row in loop_min_row..loop_max_row {
                for col in 0..cols {
                    let index = row * cols + col;
                    // Check bounds against current items length (prevents crash if navigate_to was called)
                    if index >= self.items.len() { break; }
                    
                    let x_pos = col as f32 * (item_w + padding) + padding;
                    let y_pos = row as f32 * (item_h + padding) + padding;
                    let rect = egui::Rect::from_min_size(content_min + egui::vec2(x_pos, y_pos), egui::vec2(item_w, item_h));
                    
                    if ui.is_rect_visible(rect) {
                        // Clone item for safe use in this iteration
                        let item = self.items[index].clone();
                        
                        let response = ui.interact(rect, ui.id().with(index), egui::Sense::click());
                        if response.clicked() {
                            self.selected_file = Some(item.clone());
                            self.selected_item_index = Some(index);
                        }
                        
                        let mut navigated = false;
                        if response.double_clicked() {
                            if item.is_dir { 
                                self.navigate_to(&item.path.to_string_lossy()); 
                                navigated = true;
                            }
                            else { open_with_shell(&item.path); }
                        }
                        
                        // Right click: open context menu and select item
                        if response.secondary_clicked() {
                            // Select item visually
                            self.selected_file = Some(item.clone());
                            self.selected_item_index = Some(index);
                            
                            // Open context menu
                            self.context_menu.open(
                                response.interact_pointer_pos()
                                    .unwrap_or_else(|| ui.ctx().pointer_latest_pos().unwrap_or(egui::Pos2::ZERO)),
                                Some(index),
                                Some(item.path.clone()),
                                false,
                            );
                        }

                        if self.selected_item_index == Some(index) {
                            ui.painter().rect_stroke(rect, 2.0, egui::Stroke::new(2.0, egui::Color32::from_rgb(0, 120, 215)), egui::StrokeKind::Inside);
                            ui.painter().rect_filled(rect, 4.0, egui::Color32::from_rgba_unmultiplied(0, 120, 215, 30));
                        }

                        // Tooltip at cursor
                        let item_tooltip = item.clone();
                        if response.hovered() {
                            egui::show_tooltip_at_pointer(ui.ctx(), ui.layer_id(), response.id, |ui: &mut egui::Ui| {
                                ui.set_max_width(300.0);
                                ui.vertical(|ui| {
                                    ui.label(egui::RichText::new(&item_tooltip.name).strong());
                                    ui.separator();
                                    ui.label(format!("Tipo: {}", get_file_type_string(&item_tooltip)));
                                    if !item_tooltip.is_dir {
                                        ui.label(format!("Tamanho: {}", format_size(item_tooltip.size)));
                                    }
                                    ui.label(format!("Última modificação: {}", format_date(item_tooltip.modified)));
                                });
                            });
                        }
                        
                        // Content area with margin for selection border visibility
                        let content_margin = 3.0;
                        let inner_rect = rect.shrink(content_margin);
                        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner_rect), |ui| {
                            self.render_item_slot(ui, index);
                        });

                        if navigated { break 'row_loop; } // Escape loop if context changed
                    }
                }
            }
        });
    }
}
