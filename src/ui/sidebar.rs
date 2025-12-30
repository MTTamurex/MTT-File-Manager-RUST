//! Sidebar rendering for drives and computer view
//! Follows .cursorrules: single responsibility, < 300 lines

use eframe::egui::{self, Color32, Pos2, Rect, Sense};

use crate::infrastructure::windows::extract_drive_icon;
use crate::domain::file_entry::IconSize;

/// Context for sidebar rendering
pub struct SidebarContext<'a> {
    pub disks: &'a [(String, String)],  // (path, label)
    pub current_path: &'a str,
    pub is_computer_view: bool,
    pub computer_icon: Option<&'a egui::TextureHandle>,
}

/// Operations that can be performed from sidebar
pub trait SidebarOperations {
    fn navigate_to(&mut self, path: &str);
    fn navigate_to_computer(&mut self);
}

/// Renders the sidebar with drives and computer view
pub fn render_sidebar(
    ui: &mut egui::Ui,
    ctx: &mut SidebarContext,
    ops: &mut dyn SidebarOperations,
) {
    ui.add_space(10.0);
    
    // Header "Este Computador" com ícone nativo - CLICÁVEL
    let (header_rect, header_response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 30.0),
        egui::Sense::click()
    );
    
    if ui.is_rect_visible(header_rect) {
        // Background de hover/seleção
        let is_selected = ctx.is_computer_view;
        if is_selected {
            ui.painter().rect_filled(
                header_rect,
                0.0,
                Color32::from_rgb(200, 220, 240)
            );
        } else if header_response.hovered() {
            ui.painter().rect_filled(
                header_rect,
                0.0,
                Color32::from_rgba_unmultiplied(200, 220, 240, 50)
            );
        }
        
        // Desenha ícone e texto manualmente
        let mut cursor_x = header_rect.min.x + 5.0;
        
        // Ícone
        if let Some(icon) = ctx.computer_icon {
            let icon_rect = Rect::from_min_size(
                Pos2::new(cursor_x, header_rect.center().y - 8.0),
                egui::vec2(16.0, 16.0)
            );
            ui.painter().image(
                icon.id(), 
                icon_rect, 
                Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)), 
                Color32::WHITE
            );
            cursor_x += 20.0;
        }
        
        // Texto
        ui.painter().text(
            Pos2::new(cursor_x, header_rect.center().y),
            egui::Align2::LEFT_CENTER,
            "Este Computador",
            egui::FontId::proportional(16.0),
            if is_selected {
                Color32::from_rgb(0, 50, 100)
            } else {
                ui.visuals().text_color()
            }
        );
    }
    
    // CLICK ACTION: Navega para "Este Computador"
    if header_response.clicked() {
        ops.navigate_to_computer();
    }
    
    ui.separator();
    
    ui.add_space(5.0);
    
    for (disk_path, disk_label) in ctx.disks {
        // Tenta carregar ícone real do drive (sem cache por enquanto)
        let drive_icon = match extract_drive_icon(disk_path, IconSize::Small) {
            Ok((rgba_data, width, height)) => {
                let texture = ui.ctx().load_texture(
                    format!("drive_{}", disk_path),
                    egui::ColorImage::from_rgba_unmultiplied(
                        [width as usize, height as usize],
                        &rgba_data,
                    ),
                    egui::TextureOptions::LINEAR,
                );
                Some(texture)
            }
            Err(_) => None,
        };
        
        
        // Renderiza drive com ícone + label usando interact() para controle total do cursor
        let is_selected = ctx.current_path.starts_with(disk_path);
        
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
                    "💾",
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
            ops.navigate_to(disk_path);
        }
        
        
        ui.add_space(3.0);
    }
}
