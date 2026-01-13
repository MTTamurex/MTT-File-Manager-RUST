//! Grid view rendering
//! Follows .cursorrules: single responsibility, < 300 lines

use eframe::egui::{self, Color32, Rect, Sense, Ui};
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
    pub scroll_to_selected: bool, // Scroll to selected item on keyboard navigation
    pub is_computer_view: bool,
    pub is_recycle_bin_view: bool,
    pub texture_cache: &'a mut lru::LruCache<PathBuf, egui::TextureHandle>,
    pub loading_set: &'a mut std::collections::HashSet<PathBuf>,
    pub scanned_folders: &'a mut std::collections::HashSet<PathBuf>,
    pub folder_icon_texture: Option<&'a egui::TextureHandle>,
    pub computer_icon: Option<&'a egui::TextureHandle>,
    pub drive_icon_cache: &'a mut lru::LruCache<String, egui::TextureHandle>,
    pub item_icon_loader: &'a mut crate::ui::icon_loader::IconLoader,
    pub folder_preview_cache: &'a mut lru::LruCache<PathBuf, egui::TextureHandle>,
    pub folder_preview_loading: &'a mut std::collections::HashSet<PathBuf>,
}

/// Operations that can be performed from grid view
pub trait GridViewOperations {
    fn navigate_to(&mut self, path: &str);
    fn open_with_shell(&mut self, path: &PathBuf);
    fn request_thumbnail_load(&mut self, path: PathBuf);
    fn request_folder_scan(&mut self, path: PathBuf);
    fn request_folder_preview_load(&mut self, path: PathBuf);
    fn rename_with_shell(&mut self, idx: usize);
}

/// Action returned by grid view
pub enum GridViewAction {
    Click(usize),
    DoubleClick(usize),
    SecondaryClick(usize),
}

/// Renders the grid view
pub fn render_grid_view(
    ui: &mut Ui,
    ctx: &mut GridViewContext,
    ops: &mut dyn GridViewOperations,
) -> Option<GridViewAction> {
    let padding = 8.0;
    let item_w = ctx.thumbnail_size;
    let item_h = ctx.thumbnail_size + 20.0; // Height: thumb + text
    let available_w = ui.available_width();
    let cols = ((available_w - padding) / (item_w + padding))
        .floor()
        .max(1.0) as usize;
    ctx.last_grid_cols = cols;

    // Keyboard navigation (handled by caller)

    let count = ctx.items.len();
    // Virtualized grid or Grouped grid
    let mut clicked_item = None;
    let mut double_clicked_item = None;
    let mut secondary_clicked_item = None;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if ctx.is_computer_view {
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

                let mut render_grid_section =
                    |ui: &mut Ui, items_to_render: Vec<(usize, &FileEntry)>| {
                        if items_to_render.is_empty() {
                            return;
                        }

                        let count = items_to_render.len();
                        let rows = (count as f32 / cols as f32).ceil() as usize;
                        let section_height = rows as f32 * (item_h + padding) + padding;

                        let content_min = ui.cursor().min;
                        let (_rect, _) = ui.allocate_exact_size(
                            egui::vec2(available_w, section_height),
                            Sense::hover(),
                        );

                        for (i, (index, item)) in items_to_render.into_iter().enumerate() {
                            let row = i / cols;
                            let col = i % cols;

                            let x_pos = col as f32 * (item_w + padding) + padding;
                            let y_pos = row as f32 * (item_h + padding) + padding;
                            let item_rect = Rect::from_min_size(
                                content_min + egui::vec2(x_pos, y_pos),
                                egui::vec2(item_w, item_h),
                            );

                            if ui.is_rect_visible(item_rect) {
                                let response =
                                    ui.interact(item_rect, ui.id().with(index), Sense::click());
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
                                    if ctx.scroll_to_selected {
                                        ui.scroll_to_rect(item_rect, Some(egui::Align::Center));
                                    }
                                    ui.painter().rect_stroke(
                                        item_rect,
                                        2.0,
                                        egui::Stroke::new(2.0, Color32::from_rgb(0, 120, 215)),
                                        egui::StrokeKind::Inside,
                                    );
                                    ui.painter().rect_filled(
                                        item_rect,
                                        4.0,
                                        Color32::from_rgba_unmultiplied(0, 120, 215, 30),
                                    );
                                }

                                // Tooltip
                                if response.hovered() {
                                    let item_tooltip = item.clone();
                                    let is_recycle = ctx.is_recycle_bin_view;
                                    egui::show_tooltip_at_pointer(
                                        ui.ctx(),
                                        ui.layer_id(),
                                        response.id,
                                        |ui: &mut Ui| {
                                            ui.set_max_width(300.0);
                                            ui.vertical(|ui| {
                                                ui.label(
                                                    egui::RichText::new(&item_tooltip.name)
                                                        .strong(),
                                                );
                                                ui.separator();
                                                ui.label(format!(
                                                    "Tipo: {}",
                                                    get_file_type_string(&item_tooltip)
                                                ));
                                                if !item_tooltip.is_dir {
                                                    ui.label(format!(
                                                        "Tamanho: {}",
                                                        crate::infrastructure::windows::format_size(
                                                            item_tooltip.size
                                                        )
                                                    ));
                                                }
                                                let date_lbl = if is_recycle { "Data de Exclusão" } else { "Última modificação" };
                                                let date_val = if is_recycle {
                                                    item_tooltip.deletion_date.clone().unwrap_or_else(|| "-".to_string())
                                                } else {
                                                    crate::infrastructure::windows::format_date(item_tooltip.modified)
                                                };
                                                ui.label(format!("{}: {}", date_lbl, date_val));
                                            });
                                        },
                                    );
                                }

                                let inner_rect = item_rect.shrink(3.0);
                                ui.allocate_new_ui(
                                    egui::UiBuilder::new().max_rect(inner_rect),
                                    |ui| {
                                        render_item_slot_for_grid(ui, index, item, ctx, ops);
                                    },
                                );
                            }
                        }
                    };

                if !local.is_empty() {
                    render_section_header(ui, "Discos locais");
                    render_grid_section(ui, local);
                    ui.add_space(10.0);
                }

                if !network.is_empty() {
                    render_section_header(ui, "Unidades de rede");
                    render_grid_section(ui, network);
                    ui.add_space(10.0);
                }
            } else {
                // Regular virtualized grid
                let rows = (count as f32 / cols as f32).ceil() as usize;
                let total_height = rows as f32 * (item_h + padding) + padding;
                let content_min = ui.min_rect().min;

                ui.allocate_rect(
                    Rect::from_min_size(content_min, egui::vec2(available_w, total_height)),
                    Sense::hover(),
                );

                let clip_rect = ui.clip_rect();
                let start_y = (clip_rect.top() - content_min.y).max(0.0);
                let end_y = start_y + clip_rect.height();

                let visible_min_row = (start_y / (item_h + padding)).floor() as usize;
                let visible_max_row = ((end_y / (item_h + padding)).ceil() as usize + 1).min(rows);

                let loop_min_row = visible_min_row.saturating_sub(2);
                let loop_max_row = (visible_max_row + 2).min(rows);

                for row in loop_min_row..loop_max_row {
                    for col in 0..cols {
                        let index = row * cols + col;
                        if index >= ctx.items.len() {
                            break;
                        }

                        let x_pos = col as f32 * (item_w + padding) + padding;
                        let y_pos = row as f32 * (item_h + padding) + padding;
                        let rect = Rect::from_min_size(
                            content_min + egui::vec2(x_pos, y_pos),
                            egui::vec2(item_w, item_h),
                        );

                        if ui.is_rect_visible(rect) {
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
                                if ctx.scroll_to_selected {
                                    ui.scroll_to_rect(rect, Some(egui::Align::Center));
                                }
                                ui.painter().rect_stroke(
                                    rect,
                                    2.0,
                                    egui::Stroke::new(2.0, Color32::from_rgb(0, 120, 215)),
                                    egui::StrokeKind::Inside,
                                );
                                ui.painter().rect_filled(
                                    rect,
                                    4.0,
                                    Color32::from_rgba_unmultiplied(0, 120, 215, 30),
                                );
                            }

                            if response.hovered() {
                                let item_tooltip = item.clone();
                                let is_recycle = ctx.is_recycle_bin_view;
                                egui::show_tooltip_at_pointer(
                                    ui.ctx(),
                                    ui.layer_id(),
                                    response.id,
                                    |ui: &mut Ui| {
                                        ui.set_max_width(300.0);
                                        ui.vertical(|ui| {
                                            ui.label(
                                                egui::RichText::new(&item_tooltip.name).strong(),
                                            );
                                            ui.separator();
                                            ui.label(format!(
                                                "Tipo: {}",
                                                get_file_type_string(&item_tooltip)
                                            ));
                                            if !item_tooltip.is_dir {
                                                ui.label(format!(
                                                    "Tamanho: {}",
                                                    crate::infrastructure::windows::format_size(
                                                        item_tooltip.size
                                                    )
                                                ));
                                            }
                                            let date_lbl = if is_recycle { "Data de Exclusão" } else { "Última modificação" };
                                            let date_val = if is_recycle {
                                                item_tooltip.deletion_date.clone().unwrap_or_else(|| "-".to_string())
                                            } else {
                                                crate::infrastructure::windows::format_date(item_tooltip.modified)
                                            };
                                            ui.label(format!("{}: {}", date_lbl, date_val));
                                        });
                                    },
                                );
                            }

                            let inner_rect = rect.shrink(3.0);
                            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner_rect), |ui| {
                                render_item_slot_for_grid(ui, index, item, ctx, ops);
                            });
                        }
                    }
                }
            }
        });

    // Header helper
    fn render_section_header(ui: &mut Ui, title: &str) {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(title)
                .size(13.0)
                .color(Color32::from_gray(120))
                .strong(),
        );
        ui.add_space(4.0);
    }

    // Handle actions after rendering - ORDER MATTERS!
    // double_clicked and secondary_clicked must be checked BEFORE clicked
    // because clicked() also returns true on double-click
    if let Some(idx) = double_clicked_item {
        return Some(GridViewAction::DoubleClick(idx));
    }

    if let Some(idx) = secondary_clicked_item {
        return Some(GridViewAction::SecondaryClick(idx));
    }

    if let Some(idx) = clicked_item {
        return Some(GridViewAction::Click(idx));
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

    let is_renaming = ctx
        .renaming_state
        .as_ref()
        .map_or(false, |(i, _)| *i == idx);

    // Para evitar conflitos de borrow, coletamos as operações pendentes
    // e executamos depois de renderizar
    let mut pending_thumbnail_loads: Vec<std::path::PathBuf> = Vec::new();
    let mut pending_folder_scans: Vec<std::path::PathBuf> = Vec::new();
    let mut pending_folder_preview_loads: Vec<std::path::PathBuf> = Vec::new();
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
            is_recycle_bin_view: ctx.is_recycle_bin_view,
            texture_cache: ctx.texture_cache,
            icon_loader: ctx.item_icon_loader,
            scanned_folders: ctx.scanned_folders,
            loading_set: ctx.loading_set,
            folder_preview_cache: ctx.folder_preview_cache,
            folder_preview_loading: ctx.folder_preview_loading,
        };

        // Create simple ops struct that collects operations
        struct SimpleOps<'a> {
            thumbnail_loads: &'a mut Vec<std::path::PathBuf>,
            folder_scans: &'a mut Vec<std::path::PathBuf>,
            folder_preview_loads: &'a mut Vec<std::path::PathBuf>,
            pending_rename: &'a mut Option<usize>,
        }

        impl<'a> crate::ui::components::item_slot::ItemSlotOperations for SimpleOps<'a> {
            fn request_thumbnail_load(&mut self, path: std::path::PathBuf) {
                self.thumbnail_loads.push(path);
            }

            fn request_folder_scan(&mut self, path: std::path::PathBuf) {
                self.folder_scans.push(path);
            }
            fn request_folder_preview_load(&mut self, path: std::path::PathBuf) {
                self.folder_preview_loads.push(path);
            }

            fn rename_item(&mut self, idx: usize) {
                *self.pending_rename = Some(idx);
            }
        }

        let mut simple_ops = SimpleOps {
            thumbnail_loads: &mut pending_thumbnail_loads,
            folder_scans: &mut pending_folder_scans,
            folder_preview_loads: &mut pending_folder_preview_loads,
            pending_rename: &mut pending_rename,
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
    for path in pending_folder_preview_loads {
        ops.request_folder_preview_load(path);
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
