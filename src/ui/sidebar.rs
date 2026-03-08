use crate::domain::pinned_folder::PinnedFolder;
use crate::infrastructure::windows::{detect_drive_type, DriveType};
use eframe::egui::{self, Color32, Pos2, Rect, Sense};
use rust_i18n::t;
use std::collections::HashMap;
use std::sync::Mutex;

/// Cached drive type results. Drive types don't change at runtime
/// (a drive letter is always local or always network), so we cache
/// indefinitely. The cache is cleared when the drive list changes.
static DRIVE_TYPE_CACHE: std::sync::LazyLock<Mutex<HashMap<String, DriveType>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// Get drive type from cache, only calling GetDriveTypeW on cache miss.
fn get_cached_drive_type(disk_path: &str) -> DriveType {
    let mut cache = DRIVE_TYPE_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(&dt) = cache.get(disk_path) {
        return dt;
    }
    let dt = detect_drive_type(disk_path);
    cache.insert(disk_path.to_string(), dt);
    dt
}

/// Clear the drive type cache (call when drive list changes).
pub fn invalidate_drive_type_cache() {
    if let Ok(mut cache) = DRIVE_TYPE_CACHE.lock() {
        cache.clear();
    }
}

/// Context for sidebar rendering
pub struct SidebarContext<'a> {
    pub disks: &'a [(String, String)], // (path, label)
    pub current_path: &'a str,
    pub is_computer_view: bool,
    pub is_recycle_bin_view: bool,
    pub computer_icon: Option<&'a egui::TextureHandle>,
    pub is_renaming: bool, // Blocks navigation during renaming
    pub icon_loader: &'a mut crate::ui::icon_loader::IconLoader,
    pub onedrive_path: Option<&'a str>, // OneDrive path (if installed)
    pub onedrive_icon: Option<&'a egui::TextureHandle>, // Native OneDrive icon
    pub pinned_folders: &'a [PinnedFolder],
    pub is_item_dragging: bool,   // ANY item (file or folder) is being dragged
    pub is_folder_dragging: bool,  // A folder is being dragged from the main content area
    pub dragging_path: Option<&'a str>, // Path of the folder being dragged
}

/// Actions that can be triggered by the sidebar
pub enum SidebarAction {
    NavigateTo(String),
    NavigateToComputer,
    NavigateToRecycleBin,
    PinFolder(String),
    UnpinFolder(String),
    ReorderPinnedFolder { from: usize, to: usize },
}

/// Renders the sidebar with drives and computer view
pub fn render_sidebar(ui: &mut egui::Ui, ctx: &mut SidebarContext) -> Option<SidebarAction> {
    let t_start = std::time::Instant::now();
    let mut action = None;
    ui.add_space(10.0);

    // "This PC" header with native icon
    let (header_rect, header_response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.0), egui::Sense::click());

    // Align header_rect with sidebar edges
    let mut header_rect_full = header_rect;
    header_rect_full.min.x = ui.clip_rect().min.x;
    header_rect_full.max.x = ui.clip_rect().max.x;

    if ui.is_rect_visible(header_rect_full) {
        let is_selected = ctx.is_computer_view;

        // Background
        if is_selected {
            ui.painter()
                .rect_filled(header_rect_full, 0.0, crate::ui::theme::COLOR_SELECTION);
        } else if header_response.hovered() && !ctx.is_item_dragging {
            ui.painter().rect_filled(
                header_rect_full,
                0.0,
                crate::ui::theme::color_selection_hover(),
            );
        }

        let mut cursor_x = header_rect_full.min.x + 8.0;

        // Icon
        if let Some(icon) = ctx.computer_icon {
            let icon_rect = Rect::from_center_size(
                Pos2::new(cursor_x + 8.0, header_rect_full.center().y),
                egui::vec2(18.0, 18.0),
            );
            ui.painter().image(
                icon.id(),
                icon_rect,
                Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
            cursor_x += 24.0;
        }

        // Text
        ui.painter().text(
            Pos2::new(cursor_x, header_rect_full.center().y),
            egui::Align2::LEFT_CENTER,
            t!("nav.computer"),
            egui::FontId::proportional(12.0),
            if is_selected {
                crate::ui::theme::COLOR_SELECTION_TEXT
            } else {
                ui.visuals().text_color()
            },
        );
    }

    if header_response.clicked() && !ctx.is_renaming {
        action = Some(SidebarAction::NavigateToComputer);
    }

    ui.add_space(4.0);
    ui.separator();
    ui.add_space(8.0);

    // === QUICK ACCESS ===
    let t_quick_access = std::time::Instant::now();
    // Track the start Y for drag-to-pin zone detection
    let qa_section_start_y = ui.cursor().top();

    // Section header — pure label, not interactive.
    // Using Sense::hover() so we can detect pointer position for drag-to-pin
    // highlight, but the label itself is never a clickable or selectable item.
    let (qa_label_rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 16.0), Sense::hover());
    if ui.is_rect_visible(qa_label_rect) {
        // Defer highlight drawing until after we know the full QA zone (see below).
        ui.painter().text(
            Pos2::new(qa_label_rect.min.x + 8.0, qa_label_rect.center().y),
            egui::Align2::LEFT_CENTER,
            t!("sidebar.quick_access"),
            egui::FontId::proportional(10.0),
            Color32::from_gray(120),
        );
    }
    ui.add_space(4.0);

    // 1. OneDrive (if available)
    if let Some(onedrive_path) = ctx.onedrive_path {
        let is_selected = !ctx.is_computer_view && ctx.current_path.starts_with(onedrive_path);

        let (mut rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), Sense::click());

        rect.min.x = ui.clip_rect().min.x;
        rect.max.x = ui.clip_rect().max.x;

        if ui.is_rect_visible(rect) {
            if is_selected {
                ui.painter()
                    .rect_filled(rect, 0.0, crate::ui::theme::COLOR_SELECTION);
            } else if response.hovered() {
                ui.painter()
                    .rect_filled(rect, 0.0, crate::ui::theme::color_selection_hover());
            }

            let mut cursor_x = rect.min.x + 12.0;

            // OneDrive icon
            let onedrive_icon = ctx
                .icon_loader
                .get_or_load_folder_path_icon(ui.ctx(), onedrive_path);
            if let Some(icon) = onedrive_icon {
                let icon_rect = Rect::from_center_size(
                    Pos2::new(cursor_x + 8.0, rect.center().y),
                    egui::vec2(16.0, 16.0),
                );
                ui.painter().image(
                    icon.id(),
                    icon_rect,
                    Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
                cursor_x += 24.0;
            } else {
                ui.painter().text(
                    Pos2::new(cursor_x + 8.0, rect.center().y),
                    egui::Align2::CENTER_CENTER,
                    "☁",
                    egui::FontId::proportional(12.0),
                    Color32::from_rgb(0, 120, 215),
                );
                cursor_x += 24.0;
            }

            ui.painter().text(
                Pos2::new(cursor_x, rect.center().y),
                egui::Align2::LEFT_CENTER,
                "OneDrive",
                egui::FontId::proportional(11.5),
                if is_selected {
                    crate::ui::theme::COLOR_SELECTION_TEXT
                } else {
                    ui.visuals().text_color()
                },
            );
        }

        if response.clicked() && !ctx.is_renaming {
            action = Some(SidebarAction::NavigateTo(onedrive_path.to_string()));
        }
    }

    // 2. LIXEIRA (RECYCLE BIN)
    {
        let is_selected = ctx.is_recycle_bin_view;
        let (mut rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), Sense::click());

        rect.min.x = ui.clip_rect().min.x;
        rect.max.x = ui.clip_rect().max.x;

        if ui.is_rect_visible(rect) {
            if is_selected {
                ui.painter()
                    .rect_filled(rect, 0.0, crate::ui::theme::COLOR_SELECTION);
            } else if response.hovered() && !ctx.is_item_dragging {
                ui.painter()
                    .rect_filled(rect, 0.0, crate::ui::theme::color_selection_hover());
            }

            let mut cursor_x = rect.min.x + 12.0;

            // Recycle Bin icon
            let recycle_bin_path = "shell:RecycleBinFolder";
            let recycle_icon = ctx
                .icon_loader
                .get_or_load_folder_path_icon(ui.ctx(), recycle_bin_path);

            if let Some(icon) = recycle_icon {
                let icon_rect = Rect::from_center_size(
                    Pos2::new(cursor_x + 8.0, rect.center().y),
                    egui::vec2(16.0, 16.0),
                );
                ui.painter().image(
                    icon.id(),
                    icon_rect,
                    Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
                cursor_x += 24.0;
            } else {
                cursor_x += 24.0;
            }

            ui.painter().text(
                Pos2::new(cursor_x, rect.center().y),
                egui::Align2::LEFT_CENTER,
                t!("nav.recycle_bin"),
                egui::FontId::proportional(11.5),
                if is_selected {
                    crate::ui::theme::COLOR_SELECTION_TEXT
                } else {
                    ui.visuals().text_color()
                },
            );
        }

        if response.clicked() && !ctx.is_renaming {
            action = Some(SidebarAction::NavigateToRecycleBin);
        }
    }

    // 3. USER-PINNED FOLDERS
    render_pinned_folders(ui, ctx, &mut action);

    // === Drag-to-pin drop zone ===
    if ctx.is_folder_dragging {
        let qa_section_end_y = ui.cursor().top();
        let qa_zone = egui::Rect::from_min_max(
            egui::pos2(ui.clip_rect().min.x, qa_section_start_y),
            egui::pos2(ui.clip_rect().max.x, qa_section_end_y),
        );

        let pointer_in_zone = ui
            .ctx()
            .input(|inp| inp.pointer.hover_pos())
            .map(|p| qa_zone.contains(p))
            .unwrap_or(false);

        // No visual highlight on the QA zone — pinned folders already show
        // their own hover feedback individually. The zone is only used for
        // the functional "pin folder on drop" logic below.

        let released = ui.ctx().input(|inp| inp.pointer.primary_released());
        if released && pointer_in_zone {
            if let Some(path) = ctx.dragging_path {
                let already_pinned = ctx.pinned_folders.iter().any(|pf| pf.path == path);
                if !already_pinned && action.is_none() {
                    action = Some(SidebarAction::PinFolder(path.to_string()));
                }
            }
        }
    }

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(8.0);

    let t_drives = std::time::Instant::now();
    let mut local_drives = Vec::new();
    let mut network_drives = Vec::new();

    for (disk_path, disk_label) in ctx.disks.iter() {
        let drive_type = get_cached_drive_type(disk_path);
        if drive_type == DriveType::Remote {
            network_drives.push((disk_path, disk_label));
        } else {
            local_drives.push((disk_path, disk_label));
        }
    }

    let mut render_drive_group = |title: &str, drives: Vec<(&String, &String)>| {
        if drives.is_empty() {
            return;
        }

        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(title)
                    .size(10.0)
                    .color(Color32::from_gray(120)),
            );
        });
        ui.add_space(4.0);

        for (disk_path, disk_label) in drives {
            // Don't select drive if we're in OneDrive (OneDrive has priority)
            let in_onedrive = ctx
                .onedrive_path
                .map(|od| ctx.current_path.starts_with(od))
                .unwrap_or(false);
            let is_selected =
                !ctx.is_computer_view && !in_onedrive && ctx.current_path.starts_with(disk_path);

            let (mut rect, response) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), Sense::click());

            rect.min.x = ui.clip_rect().min.x;
            rect.max.x = ui.clip_rect().max.x;

            if ui.is_rect_visible(rect) {
                if is_selected {
                    ui.painter()
                        .rect_filled(rect, 0.0, crate::ui::theme::COLOR_SELECTION);
                } else if response.hovered() {
                    ui.painter()
                        .rect_filled(rect, 0.0, crate::ui::theme::color_selection_hover());
                }

                let mut cursor_x = rect.min.x + 12.0; // Indentation for drives

                // Try to load real drive icon (via IconLoader)
                let drive_icon = ctx.icon_loader.get_or_load_drive_icon(ui.ctx(), disk_path);

                if let Some(icon) = drive_icon {
                    let icon_rect = Rect::from_center_size(
                        Pos2::new(cursor_x + 8.0, rect.center().y),
                        egui::vec2(16.0, 16.0),
                    );
                    ui.painter().image(
                        icon.id(),
                        icon_rect,
                        Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                    cursor_x += 24.0;
                } else {
                    cursor_x += 24.0;
                }

                ui.painter().text(
                    Pos2::new(cursor_x, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    disk_label,
                    egui::FontId::proportional(11.5),
                    if is_selected {
                        crate::ui::theme::COLOR_SELECTION_TEXT
                    } else {
                        ui.visuals().text_color()
                    },
                );
            }

            if response.clicked() && !ctx.is_renaming {
                action = Some(SidebarAction::NavigateTo(disk_path.to_string()));
            }
            ui.add_space(2.0);
        }

        ui.add_space(6.0);
    };

    render_drive_group("Discos locais", local_drives);
    render_drive_group("Unidades de rede", network_drives);

    let total_ms = t_start.elapsed().as_millis();
    if total_ms > 50 {
        log::warn!(
            "[PERF-SIDEBAR] total={}ms header={}ms quick_access={}ms drives={}ms | disks={} pinned={}",
            total_ms,
            t_quick_access.duration_since(t_start).as_millis(),
            t_drives.duration_since(t_quick_access).as_millis(),
            t_start.elapsed().as_millis().saturating_sub(t_drives.duration_since(t_start).as_millis()),
            ctx.disks.len(),
            ctx.pinned_folders.len(),
        );
    }

    action
}

/// Renders the user-pinned folders list with drag-to-reorder support.
fn render_pinned_folders(
    ui: &mut egui::Ui,
    ctx: &mut SidebarContext,
    action: &mut Option<SidebarAction>,
) {
    if ctx.pinned_folders.is_empty() {
        return;
    }

    let drag_id = egui::Id::new("qa_reorder_drag");
    let drag_src: Option<usize> = ui.ctx().data(|d| d.get_temp(drag_id));
    let mut hover_drop_idx: Option<usize> = None;

    for (i, pinned) in ctx.pinned_folders.iter().enumerate() {
        let is_selected = !ctx.is_computer_view
            && !ctx.is_recycle_bin_view
            && ctx.current_path == pinned.path;


        // Drop indicator line above this item
        if let Some(src) = drag_src {
            if src != i {
                let cursor_y = ui.cursor().min.y;
                let pointer_y = ui
                    .ctx()
                    .input(|inp| inp.pointer.hover_pos())
                    .map(|p| p.y)
                    .unwrap_or(f32::MAX);
                if pointer_y < cursor_y + 14.0 && pointer_y >= cursor_y - 14.0 {
                    hover_drop_idx = Some(i);
                    ui.painter().hline(
                        ui.clip_rect().min.x..=ui.clip_rect().max.x,
                        cursor_y,
                        egui::Stroke::new(2.0, Color32::from_rgb(0, 120, 215)),
                    );
                }
            }
        }

        let (mut rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 28.0),
            Sense::click_and_drag(),
        );
        rect.min.x = ui.clip_rect().min.x;
        rect.max.x = ui.clip_rect().max.x;

        // Start internal drag for reordering
        if response.drag_started() {
            ui.ctx().data_mut(|d| d.insert_temp(drag_id, i));
        }

        // Pin rect is computed outside the visibility check so it can be used for click detection
        let pin_size = 18.0;
        let pin_rect = Rect::from_center_size(
            Pos2::new(rect.max.x - pin_size / 2.0 - 3.0, rect.center().y),
            egui::vec2(pin_size, pin_size),
        );

        if ui.is_rect_visible(rect) {
            let is_being_dragged = drag_src == Some(i);

            if is_being_dragged {
                ui.painter().rect_filled(
                    rect,
                    0.0,
                    Color32::from_rgba_premultiplied(100, 120, 215, 60),
                );
            } else if is_selected {
                ui.painter()
                    .rect_filled(rect, 0.0, crate::ui::theme::COLOR_SELECTION);
            } else if response.hovered() || response.dragged() {
                ui.painter()
                    .rect_filled(rect, 0.0, crate::ui::theme::color_selection_hover());
            }

            let mut cursor_x = rect.min.x + 12.0;

            // Native folder icon
            let folder_icon = ctx
                .icon_loader
                .get_or_load_folder_path_icon(ui.ctx(), &pinned.path);
            if let Some(icon) = folder_icon {
                let icon_rect = Rect::from_center_size(
                    Pos2::new(cursor_x + 8.0, rect.center().y),
                    egui::vec2(16.0, 16.0),
                );
                ui.painter().image(
                    icon.id(),
                    icon_rect,
                    Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
                cursor_x += 24.0;
            } else {
                cursor_x += 24.0;
            }

            let text_color = if is_selected {
                crate::ui::theme::COLOR_SELECTION_TEXT
            } else {
                ui.visuals().text_color()
            };
            ui.painter().text(
                Pos2::new(cursor_x, rect.center().y),
                egui::Align2::LEFT_CENTER,
                &pinned.display_name,
                egui::FontId::proportional(11.5),
                text_color,
            );

            // Pin icon — always visible; click removes the shortcut
            if !is_being_dragged {
                let pointer_pos = ui.input(|inp| inp.pointer.hover_pos());
                let pin_hovered = pointer_pos.map(|p| pin_rect.contains(p)).unwrap_or(false);
                let pin_color = if pin_hovered {
                    Color32::from_rgb(220, 60, 60)
                } else {
                    Color32::from_gray(140)
                };
                ui.painter().text(
                    pin_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "📌",
                    egui::FontId::proportional(10.0),
                    pin_color,
                );
            }
        }

        // Handle click: check if the click landed on the pin icon or the rest of the row
        if response.clicked() && !ctx.is_renaming && action.is_none() {
            let click_pos = ui.input(|inp| inp.pointer.interact_pos());
            let clicked_pin = click_pos.map(|p| pin_rect.contains(p)).unwrap_or(false);
            if clicked_pin {
                *action = Some(SidebarAction::UnpinFolder(pinned.path.clone()));
            } else {
                *action = Some(SidebarAction::NavigateTo(pinned.path.clone()));
            }
        }

        ui.add_space(2.0);
    }

    // Drop indicator line at end of list
    if let Some(src) = drag_src {
        let end_y = ui.cursor().min.y;
        let pointer_y = ui
            .ctx()
            .input(|inp| inp.pointer.hover_pos())
            .map(|p| p.y)
            .unwrap_or(f32::MAX);
        if pointer_y >= end_y - 14.0 && hover_drop_idx.is_none() && src < ctx.pinned_folders.len() {
            hover_drop_idx = Some(ctx.pinned_folders.len());
            ui.painter().hline(
                ui.clip_rect().min.x..=ui.clip_rect().max.x,
                end_y,
                egui::Stroke::new(2.0, Color32::from_rgb(0, 120, 215)),
            );
        }
    }

    // Handle drop (release while reordering)
    if let Some(src) = drag_src {
        let released = ui.ctx().input(|inp| inp.pointer.primary_released());
        if released {
            ui.ctx().data_mut(|d| d.remove::<usize>(drag_id));
            if let Some(dst) = hover_drop_idx {
                if dst != src && action.is_none() {
                    *action = Some(SidebarAction::ReorderPinnedFolder { from: src, to: dst });
                }
            }
        }
    }
}
