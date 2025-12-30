//! Computer view rendering (drives)
//! Follows .cursorrules: single responsibility, < 300 lines

use eframe::egui;

use crate::ui::app::ImageViewerApp;

impl ImageViewerApp {
    /// Renders the "Este Computador" view with drives.
    pub fn render_computer_view(&mut self, ui: &mut egui::Ui) {
        let padding = 8.0;
        let item_w = self.thumbnail_size;
        let item_h = self.thumbnail_size + 20.0;  // Height: thumb + text
        let available_w = ui.available_width();
        let cols = ((available_w - padding) / (item_w + padding)).floor().max(1.0) as usize;
        self.last_grid_cols = cols;
        
        // Virtualized Grid for drives
        let count = self.disks.len();
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
                    if index >= self.disks.len() { break; }
                    
                    let (disk_path, disk_label) = self.disks[index].clone();
                    let x_pos = col as f32 * (item_w + padding) + padding;
                    let y_pos = row as f32 * (item_h + padding) + padding;
                    let rect = egui::Rect::from_min_size(content_min + egui::vec2(x_pos, y_pos), egui::vec2(item_w, item_h));
                    
                    if ui.is_rect_visible(rect) {
                        let response = ui.interact(rect, ui.id().with(index), egui::Sense::click());
                        if response.clicked() {
                            self.selected_item_index = Some(index);
                            self.selected_file = None;  // Not a FileEntry
                        }
                        
                        if response.double_clicked() {
                            self.navigate_to(&disk_path);
                            break 'row_loop;
                        }
                        
                        // Right click: open context menu
                        if response.secondary_clicked() {
                            self.selected_item_index = Some(index);
                            self.selected_file = None;
                            
                            self.context_menu.open(
                                response.interact_pointer_pos()
                                    .unwrap_or_else(|| ui.ctx().pointer_latest_pos().unwrap_or(egui::Pos2::ZERO)),
                                Some(index),
                                Some(std::path::PathBuf::from(&disk_path)),
                                false,
                            );
                        }

                        if self.selected_item_index == Some(index) {
                            ui.painter().rect_stroke(rect, 2.0, egui::Stroke::new(2.0, egui::Color32::from_rgb(0, 120, 215)), egui::StrokeKind::Inside);
                            ui.painter().rect_filled(rect, 4.0, egui::Color32::from_rgba_unmultiplied(0, 120, 215, 30));
                        }

                        // Tooltip
                        if response.hovered() {
                            egui::show_tooltip_at_pointer(ui.ctx(), ui.layer_id(), response.id, |ui: &mut egui::Ui| {
                                ui.set_max_width(300.0);
                                ui.vertical(|ui| {
                                    ui.label(egui::RichText::new(&disk_label).strong());
                                    ui.separator();
                                    ui.label(format!("Caminho: {}", disk_path));
                                    ui.label("Tipo: Drive");
                                });
                            });
                        }
                        
                        // Content area
                        let content_margin = 3.0;
                        let inner_rect = rect.shrink(content_margin);
                        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner_rect), |ui| {
                            self.render_drive_slot(ui, &disk_path, &disk_label);
                        });
                    }
                }
            }
        });
    }
}
