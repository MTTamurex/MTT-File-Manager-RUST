//! Virtual drive settings modal for configuring SSD/HDD optimization

use crate::infrastructure::virtual_drive_config::{DiskTypeOverride, get_all_overrides, set_drive_override, remove_drive_override};
use crate::infrastructure::windows::drives::get_all_drives;
use crate::infrastructure::io_priority;
use eframe::egui;

/// Info about a detected virtual drive
#[derive(Clone)]
struct VirtualDriveInfo {
    letter: char,
    label: String,
    file_system: String,
    current_override: Option<DiskTypeOverride>,
}

/// Render the virtual drive settings modal window
pub fn render_virtual_drive_settings(
    ctx: &egui::Context,
    show_modal: bool,
) -> bool {
    let mut keep_open = show_modal;
    
    let response = egui::Window::new("⚙ Configuração de Drives Virtuais")
        .collapsible(false)
        .resizable(true)
        .default_width(600.0)
        .default_height(400.0)
        .show(ctx, |ui| {
            ui.vertical(|ui| {
                // Header explanation
                ui.heading("Otimização de Drives Virtuais");
                ui.add_space(8.0);
                ui.label("Configure como drives virtuais (Cryptomator, etc.) devem ser otimizados:");
                ui.label("• SSD: Acesso aleatório rápido (padrão para drives desconhecidos)");
                ui.label("• HDD: Agrupamento por diretório para minimizar seeks");
                ui.add_space(16.0);

                // Detect virtual drives
                let virtual_drives = detect_virtual_drives();
                
                if virtual_drives.is_empty() {
                    ui.colored_label(
                        egui::Color32::from_rgb(200, 200, 0),
                        "⚠ Nenhum drive virtual detectado no sistema"
                    );
                    ui.add_space(8.0);
                    ui.label("Drives virtuais típicos incluem:");
                    ui.label("• Cryptomator (CryptoFS)");
                    ui.label("• Dokan");
                    ui.label("• WinFsp");
                } else {
                    // Table header
                    egui::Grid::new("virtual_drives_grid")
                        .striped(true)
                        .min_col_width(60.0)
                        .show(ui, |ui| {
                            ui.strong("Drive");
                            ui.strong("Label");
                            ui.strong("Sistema");
                            ui.strong("Otimização");
                            ui.strong("Ações");
                            ui.end_row();

                            for drive_info in &virtual_drives {
                                render_drive_row(ui, drive_info);
                            }
                        });
                }

                ui.add_space(16.0);

                ui.separator();
                
                // Info footer
                ui.horizontal(|ui| {
                    ui.label("💡");
                    ui.label("As configurações são salvas em virtual_drive_config.json e aplicadas imediatamente");
                });

                ui.add_space(8.0);

                // Close button
                ui.horizontal(|ui| {
                    if ui.button("Fechar").clicked() {
                        keep_open = false;
                    }
                });
            });
        });
    
    // Check if user closed via X button
    if let Some(resp) = response {
        if resp.response.hovered() {
            // Window still open
            keep_open
        } else {
            keep_open
        }
    } else {
        false // Window was closed
    }
}

/// Detect all virtual drives in the system
fn detect_virtual_drives() -> Vec<VirtualDriveInfo> {
    let mut virtual_drives = Vec::new();
    let overrides = get_all_overrides();

    // Get all available drives
    let drives = get_all_drives();

    for (path, label) in drives {
        if let Some(drive_letter) = path.chars().next() {
            let drive_letter = drive_letter.to_ascii_uppercase();
            
            // Check if it's a virtual drive by querying volume info
            if let Some((is_virtual, fs)) = check_if_virtual(drive_letter) {
                if is_virtual {
                    virtual_drives.push(VirtualDriveInfo {
                        letter: drive_letter,
                        label,
                        file_system: fs,
                        current_override: overrides.get(&drive_letter).copied(),
                    });
                }
            }
        }
    }

    virtual_drives
}

/// Check if a drive is virtual by examining its file system
fn check_if_virtual(drive_letter: char) -> Option<(bool, String)> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetVolumeInformationW;

    let root_path = format!("{}:\\", drive_letter);
    let wide_path: Vec<u16> = root_path.encode_utf16().chain(std::iter::once(0)).collect();

    let mut volume_name = [0u16; 261];
    let mut file_system_name = [0u16; 261];
    let mut serial_number: u32 = 0;
    let mut max_component_len: u32 = 0;
    let mut fs_flags: u32 = 0;

    let ok = unsafe {
        GetVolumeInformationW(
            PCWSTR(wide_path.as_ptr()),
            Some(&mut volume_name),
            Some(&mut serial_number),
            Some(&mut max_component_len),
            Some(&mut fs_flags),
            Some(&mut file_system_name),
        )
    };

    if !ok.is_ok() {
        return None;
    }

    let volume_len = volume_name
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(volume_name.len());
    let fs_len = file_system_name
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(file_system_name.len());

    let volume = String::from_utf16_lossy(&volume_name[..volume_len]).to_lowercase();
    let file_system = String::from_utf16_lossy(&file_system_name[..fs_len]);
    let fs_lower = file_system.to_lowercase();

    // Detect virtual drive indicators
    let is_virtual = volume.contains("cryptomator")
        || fs_lower.contains("cryptofs")
        || fs_lower.contains("dokan")
        || fs_lower.contains("winfsp")
        || fs_lower == "fuse";

    Some((is_virtual, file_system))
}

/// Render a single drive configuration row
fn render_drive_row(ui: &mut egui::Ui, drive_info: &VirtualDriveInfo) {
    // Drive letter
    ui.label(format!("{}:\\", drive_info.letter));

    // Label
    ui.label(&drive_info.label);

    // File system
    ui.label(&drive_info.file_system);

    // Current setting
    let current_type = drive_info.current_override.unwrap_or(DiskTypeOverride::SSD);
    let is_ssd = matches!(current_type, DiskTypeOverride::SSD);

    ui.horizontal(|ui| {
        if ui
            .selectable_label(is_ssd, "SSD")
            .on_hover_text("Acesso aleatório rápido")
            .clicked()
        {
            if let Err(e) = set_drive_override(drive_info.letter, DiskTypeOverride::SSD) {
                eprintln!("[Config] Failed to set SSD override: {}", e);
            } else {
                io_priority::invalidate_drive_cache(drive_info.letter);
            }
        }

        if ui
            .selectable_label(!is_ssd, "HDD")
            .on_hover_text("Agrupamento por diretório")
            .clicked()
        {
            if let Err(e) = set_drive_override(drive_info.letter, DiskTypeOverride::HDD) {
                eprintln!("[Config] Failed to set HDD override: {}", e);
            } else {
                io_priority::invalidate_drive_cache(drive_info.letter);
            }
        }
    });

    // Actions
    ui.horizontal(|ui| {
        if drive_info.current_override.is_some() {
            if ui.button("🗑").on_hover_text("Remover configuração").clicked() {
                if let Err(e) = remove_drive_override(drive_info.letter) {
                    eprintln!("[Config] Failed to remove override: {}", e);
                } else {
                    io_priority::invalidate_drive_cache(drive_info.letter);
                }
            }
        } else {
            ui.label("(padrão)");
        }
    });

    ui.end_row();
}
