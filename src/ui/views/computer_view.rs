//! Computer view rendering (Este Computador)
//! Follows .cursorrules: single responsibility, < 300 lines

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Ui};

use crate::domain::file_entry::IconSize;
use crate::infrastructure::windows;

/// Context for computer view rendering
pub struct ComputerViewContext<'a> {
    pub disks: &'a [(String, String)],  // (path, label)
    pub selected_disk: Option<&'a str>,
    pub computer_icon: Option<&'a egui::TextureHandle>,
    pub drive_icon_cache: &'a mut lru::LruCache<String, egui::TextureHandle>,
}

/// Operations that can be performed from computer view
pub trait ComputerViewOperations {
    fn navigate_to(&mut self, path: &str);
    fn extract_drive_icon(&mut self, drive_path: &str, size: IconSize) -> Option<egui::TextureHandle>;
}

/// Renders the computer view (Este Computador)
pub fn render_computer_view(
    ui: &mut Ui,
    ctx: &mut ComputerViewContext,
    _ops: &mut dyn ComputerViewOperations,
) -> Option<String> {
    let mut clicked_disk = None;
    
    for (disk_path, disk_label) in ctx.disks {
        // Pré-carrega ícone do drive se não estiver no cache
        let drive_icon = if let Some(icon) = ctx.drive_icon_cache.get(disk_path) {
            Some(icon.clone())
        } else {
            // Tenta carregar ícone real do drive
            if let Ok((rgba_data, width, height)) = windows::extract_drive_icon(disk_path, IconSize::Small) {
                let texture = ui.ctx().load_texture(
                    format!("drive_{}", disk_path),
                    egui::ColorImage::from_rgba_unmultiplied(
                        [width as usize, height as usize],
                        &rgba_data,
                    ),
                    egui::TextureOptions::LINEAR,
                );
                let cloned = texture.clone();
                ctx.drive_icon_cache.put(disk_path.clone(), texture);
                Some(cloned)
            } else {
                None
            }
        };
        
        // Renderiza drive com ícone + label usando interact() para controle total do cursor
        let is_selected = ctx.selected_disk == Some(disk_path.as_str());
        
        // Desenha conteúdo no horizontal layout
        let (mut rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 24.0),
            Sense::click()  // Captura cliques, sem texto selecionável
        );
        
        // Expande rect para preencher toda a largura da sidebar (remove gaps)
        rect.min.x = ui.clip_rect().min.x;
        rect.max.x = ui.clip_rect().max.x;
        
        // Só desenha se visível
        if ui.is_rect_visible(rect) {
            // Background de seleção
            if is_selected {
                ui.painter().rect_filled(
                    rect,
                    0.0,  // Sem cantos arredondados para ficar flush com as bordas
                    Color32::from_rgb(200, 220, 240)
                );
            }
            
            // Hover effect
            if response.hovered() && !is_selected {
                ui.painter().rect_filled(
                    rect,
                    2.0,
                    Color32::from_rgba_unmultiplied(200, 220, 240, 50)
                );
            }
            
            // Desenha ícone e texto manualmente
            let mut cursor_x = rect.min.x + 5.0;
            
            // Ícone
            if let Some(icon) = drive_icon {
                let icon_rect = Rect::from_min_size(
                    Pos2::new(cursor_x, rect.center().y - 8.0),
                    egui::vec2(16.0, 16.0)
                );
                ui.painter().image(icon.id(), icon_rect, Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)), Color32::WHITE);
                cursor_x += 20.0;
            } else {
                ui.painter().text(
                    Pos2::new(cursor_x, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    "💽",
                    egui::FontId::proportional(14.0),
                    ui.visuals().text_color()
                );
                cursor_x += 20.0;
            }
            
            // Texto
            ui.painter().text(
                Pos2::new(cursor_x, rect.center().y),
                egui::Align2::LEFT_CENTER,
                disk_label,
                egui::FontId::proportional(14.0),
                if is_selected { 
                    Color32::from_rgb(0, 50, 100) 
                } else { 
                    ui.visuals().text_color() 
                }
            );
        }
        
        if response.clicked() {
            clicked_disk = Some(disk_path.clone());
        }
        
        ui.add_space(3.0);
    }
    
    clicked_disk
}
