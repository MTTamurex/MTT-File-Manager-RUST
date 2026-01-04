use crate::infrastructure::windows::{detect_drive_type, DriveType};
use eframe::egui::{self, Color32, Pos2, Rect, Sense};

/// Context for sidebar rendering
pub struct SidebarContext<'a> {
    pub disks: &'a [(String, String)], // (path, label)
    pub current_path: &'a str,
    pub is_computer_view: bool,
    pub computer_icon: Option<&'a egui::TextureHandle>,
    pub is_renaming: bool, // Bloqueia navegação durante renomeação
    pub icon_loader: &'a mut crate::ui::icon_loader::IconLoader,
    pub onedrive_path: Option<&'a str>, // Caminho do OneDrive (se instalado)
    pub onedrive_icon: Option<&'a egui::TextureHandle>, // Ícone nativo do OneDrive
}

/// Ações que podem ser disparadas pela sidebar
pub enum SidebarAction {
    NavigateTo(String),
    NavigateToComputer,
}

/// Renders the sidebar with drives and computer view
pub fn render_sidebar(ui: &mut egui::Ui, ctx: &mut SidebarContext) -> Option<SidebarAction> {
    let mut action = None;
    ui.add_space(10.0);

    // Header "Este Computador" com ícone nativo
    let (header_rect, header_response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.0), egui::Sense::click());

    // Alinha header_rect com as bordas da sidebar
    let mut header_rect_full = header_rect;
    header_rect_full.min.x = ui.clip_rect().min.x;
    header_rect_full.max.x = ui.clip_rect().max.x;

    if ui.is_rect_visible(header_rect_full) {
        let is_selected = ctx.is_computer_view;

        // Background
        if is_selected {
            ui.painter()
                .rect_filled(header_rect_full, 0.0, Color32::from_rgb(200, 220, 240));
        } else if header_response.hovered() {
            ui.painter().rect_filled(
                header_rect_full,
                0.0,
                Color32::from_rgba_unmultiplied(200, 220, 240, 50),
            );
        }

        let mut cursor_x = header_rect_full.min.x + 8.0;

        // Ícone
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

        // Texto
        ui.painter().text(
            Pos2::new(cursor_x, header_rect_full.center().y),
            egui::Align2::LEFT_CENTER,
            "Este Computador",
            egui::FontId::proportional(14.0),
            if is_selected {
                Color32::from_rgb(0, 50, 100)
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

    // === QUICK ACCESS: OneDrive ===
    if let Some(onedrive_path) = ctx.onedrive_path {
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("Acesso Rápido")
                    .size(10.0)
                    .color(Color32::from_gray(120)),
            );
        });
        ui.add_space(4.0);

        let is_selected = !ctx.is_computer_view && ctx.current_path.starts_with(onedrive_path);

        let (mut rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), Sense::click());

        rect.min.x = ui.clip_rect().min.x;
        rect.max.x = ui.clip_rect().max.x;

        if ui.is_rect_visible(rect) {
            if is_selected {
                ui.painter()
                    .rect_filled(rect, 0.0, Color32::from_rgb(200, 220, 240));
            } else if response.hovered() {
                ui.painter().rect_filled(
                    rect,
                    0.0,
                    Color32::from_rgba_unmultiplied(200, 220, 240, 50),
                );
            }

            let mut cursor_x = rect.min.x + 12.0;

            // Ícone OneDrive (carrega ícone nativo do Windows via IconLoader)
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
                // Fallback: cloud emoji
                ui.painter().text(
                    Pos2::new(cursor_x + 8.0, rect.center().y),
                    egui::Align2::CENTER_CENTER,
                    "☁",
                    egui::FontId::proportional(14.0),
                    Color32::from_rgb(0, 120, 215),
                );
                cursor_x += 24.0;
            }

            ui.painter().text(
                Pos2::new(cursor_x, rect.center().y),
                egui::Align2::LEFT_CENTER,
                "OneDrive",
                egui::FontId::proportional(13.0),
                if is_selected {
                    Color32::from_rgb(0, 50, 100)
                } else {
                    ui.visuals().text_color()
                },
            );
        }

        if response.clicked() && !ctx.is_renaming {
            action = Some(SidebarAction::NavigateTo(onedrive_path.to_string()));
        }

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);
    }

    let mut local_drives = Vec::new();
    let mut network_drives = Vec::new();

    for (disk_path, disk_label) in ctx.disks.iter() {
        let drive_type = detect_drive_type(disk_path);
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
            let is_selected = !ctx.is_computer_view && ctx.current_path.starts_with(disk_path);

            let (mut rect, response) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), Sense::click());

            rect.min.x = ui.clip_rect().min.x;
            rect.max.x = ui.clip_rect().max.x;

            if ui.is_rect_visible(rect) {
                if is_selected {
                    ui.painter()
                        .rect_filled(rect, 0.0, Color32::from_rgb(200, 220, 240));
                } else if response.hovered() {
                    ui.painter().rect_filled(
                        rect,
                        0.0,
                        Color32::from_rgba_unmultiplied(200, 220, 240, 50),
                    );
                }

                let mut cursor_x = rect.min.x + 12.0; // Identação para discos

                // Tenta carregar ícone real do drive (via IconLoader)
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
                    ui.painter().text(
                        Pos2::new(cursor_x, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        "💽",
                        egui::FontId::proportional(14.0),
                        ui.visuals().text_color(),
                    );
                    cursor_x += 20.0;
                }

                ui.painter().text(
                    Pos2::new(cursor_x, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    disk_label,
                    egui::FontId::proportional(13.0),
                    if is_selected {
                        Color32::from_rgb(0, 50, 100)
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

    action
}
