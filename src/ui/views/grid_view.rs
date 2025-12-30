//! Grid view rendering
//! Follows .cursorrules: single responsibility, < 300 lines

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Ui};
use std::path::PathBuf;

use crate::domain::file_entry::FileEntry;

/// Context for grid view rendering
pub struct GridViewContext<'a> {
    pub items: &'a [FileEntry],
    pub selected_item: Option<usize>,
    pub selected_file: Option<&'a FileEntry>,
    pub thumbnail_size: f32,
    pub last_grid_cols: usize,
    pub renaming_state: Option<(usize, String)>,
    pub focus_rename: bool,
    pub texture_cache: &'a mut lru::LruCache<PathBuf, egui::TextureHandle>,
    pub loading_set: &'a mut std::collections::HashSet<PathBuf>,
    pub scanned_folders: &'a mut std::collections::HashSet<PathBuf>,
    pub folder_icon_texture: Option<&'a egui::TextureHandle>,
    pub computer_icon: Option<&'a egui::TextureHandle>,
    pub drive_icon_cache: &'a mut lru::LruCache<String, egui::TextureHandle>,
    pub item_icon_loader: &'a mut crate::ui::icon_loader::IconLoader,
}

/// Operations that can be performed from grid view
pub trait GridViewOperations {
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

/// Renders the grid view
pub fn render_grid_view(
    ui: &mut Ui,
    ctx: &mut GridViewContext,
    ops: &mut dyn GridViewOperations,
) -> Option<usize> {
    let padding = 8.0;
    let item_w = ctx.thumbnail_size;
    let item_h = ctx.thumbnail_size + 20.0;  // Height: thumb + text
    let available_w = ui.available_width();
    let cols = ((available_w - padding) / (item_w + padding)).floor().max(1.0) as usize;
    ctx.last_grid_cols = cols;
    
    // Keyboard navigation (handled by caller)
    
    // Virtualized grid
    let count = ctx.items.len();
    let rows = (count as f32 / cols as f32).ceil() as usize;
    let total_height = rows as f32 * (item_h + padding) + padding;
    
    let mut clicked_item = None;
    let mut double_clicked_item = None;
    let mut secondary_clicked_item = None;
    let mut navigated = false;
    
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let content_min = ui.min_rect().min;
        ui.allocate_rect(Rect::from_min_size(content_min, egui::vec2(available_w, total_height)), Sense::hover());
        
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
                // Check bounds against current items length
                if index >= ctx.items.len() { break; }
                
                let x_pos = col as f32 * (item_w + padding) + padding;
                let y_pos = row as f32 * (item_h + padding) + padding;
                let rect = Rect::from_min_size(content_min + egui::vec2(x_pos, y_pos), egui::vec2(item_w, item_h));
                
                if ui.is_rect_visible(rect) {
                    // Clone item for safe use in this iteration
                    let item = &ctx.items[index];
                    
                    let response = ui.interact(rect, ui.id().with(index), Sense::click());
                    if response.clicked() {
                        clicked_item = Some(index);
                    }
                    
                    if response.double_clicked() {
                        double_clicked_item = Some(index);
                    }
                    
                    if response.secondary_clicked() {
                        secondary_clicked_item = Some(index);
                    }

                    if ctx.selected_item == Some(index) {
                        ui.painter().rect_stroke(rect, 2.0, egui::Stroke::new(2.0, Color32::from_rgb(0, 120, 215)), egui::StrokeKind::Inside);
                        ui.painter().rect_filled(rect, 4.0, Color32::from_rgba_unmultiplied(0, 120, 215, 30));
                    }

                    // Tooltip at cursor
                    let item_tooltip = item.clone();
                    if response.hovered() {
                        egui::show_tooltip_at_pointer(ui.ctx(), ui.layer_id(), response.id, |ui: &mut Ui| {
                            ui.set_max_width(300.0);
                            ui.vertical(|ui| {
                                ui.label(egui::RichText::new(&item_tooltip.name).strong());
                                ui.separator();
                                ui.label(format!("Tipo: {}", get_file_type_string(&item_tooltip)));
                                if !item_tooltip.is_dir {
                                    ui.label(format!("Tamanho: {}", crate::infrastructure::windows::format_size(item_tooltip.size)));
                                }
                                ui.label(format!("Última modificação: {}", crate::infrastructure::windows::format_date(item_tooltip.modified)));
                            });
                        });
                    }
                    
                    // Content area with margin for selection border visibility
                    let content_margin = 3.0;
                    let inner_rect = rect.shrink(content_margin);
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner_rect), |ui| {
                        render_item_slot_for_grid(ui, index, item, ctx, ops);
                    });

                    if navigated { break 'row_loop; }
                }
            }
        }
    });
    
    // Handle actions after rendering
    if let Some(idx) = clicked_item {
        return Some(idx);
    }
    
    if let Some(idx) = double_clicked_item {
        let item = &ctx.items[idx];
        if item.is_dir {
            ops.navigate_to(&item.path.to_string_lossy());
            navigated = true;
        } else {
            ops.open_with_shell(&item.path);
        }
    }
    
    if let Some(idx) = secondary_clicked_item {
        // This would trigger context menu - handled by caller
        return Some(idx);
    }
    
    None
}

/// Renders an individual item slot for grid view
fn render_item_slot_for_grid(
    ui: &mut Ui,
    idx: usize,
    item: &FileEntry,
    ctx: &mut GridViewContext,
    ops: &mut dyn GridViewOperations,
) {
    use crate::ui::components::item_slot::{render_item_slot, ItemSlotContext};
    
    let is_renaming = ctx.renaming_state.as_ref().map_or(false, |(i, _)| *i == idx);
    
    // Para evitar conflitos de borrow, coletamos as operações pendentes
    // e executamos depois de renderizar
    let mut pending_thumbnail_loads: Vec<std::path::PathBuf> = Vec::new();
    let mut pending_folder_scans: Vec<std::path::PathBuf> = Vec::new();
    let mut pending_rename: Option<usize> = None;
    
    // Texto de renomeação precisa ser tratado separadamente
    let mut renaming_text_clone = if is_renaming {
        ctx.renaming_state.as_ref().map(|(_, s)| s.clone())
    } else {
        None
    };
    
    // Create context with mutable reference to the clone
    {
        let renaming_text = renaming_text_clone.as_mut();
        
        let mut item_slot_ctx = ItemSlotContext {
            item,
            idx,
            thumbnail_size: ctx.thumbnail_size,
            is_renaming,
            renaming_text,
            focus_rename: ctx.focus_rename,
            texture_cache: ctx.texture_cache,
            icon_loader: ctx.item_icon_loader,
            scanned_folders: ctx.scanned_folders,
            loading_set: ctx.loading_set,
        };
        
        // Create simple ops struct that collects operations
        struct SimpleOps<'a> {
            thumbnail_loads: &'a mut Vec<std::path::PathBuf>,
            folder_scans: &'a mut Vec<std::path::PathBuf>,
            pending_rename: &'a mut Option<usize>,
            grid_ops: &'a mut dyn GridViewOperations,
        }
        
        impl<'a> crate::ui::components::item_slot::ItemSlotOperations for SimpleOps<'a> {
            fn request_thumbnail_load(&mut self, path: std::path::PathBuf) {
                self.thumbnail_loads.push(path);
            }
            
            fn request_folder_scan(&mut self, path: std::path::PathBuf) {
                self.folder_scans.push(path);
            }
            
            fn rename_item(&mut self, idx: usize) {
                *self.pending_rename = Some(idx);
            }
        }
        
        let mut simple_ops = SimpleOps {
            thumbnail_loads: &mut pending_thumbnail_loads,
            folder_scans: &mut pending_folder_scans,
            pending_rename: &mut pending_rename,
            grid_ops: ops,
        };
        
        render_item_slot(ui, &mut item_slot_ctx, &mut simple_ops);
    }
    
    // Apply changes after render
    if let Some(new_text) = renaming_text_clone {
        if is_renaming {
            if let Some((_, ref mut text)) = ctx.renaming_state {
                *text = new_text;
            }
        }
    }
    
    // Execute pending operations
    for path in pending_thumbnail_loads {
        ops.request_thumbnail_load(path);
    }
    
    for path in pending_folder_scans {
        ops.request_folder_scan(path);
    }
    
    if let Some(rename_idx) = pending_rename {
        ops.rename_with_shell(rename_idx);
    }
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
