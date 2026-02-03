//! Helper functions for list view: file type strings, status badges, section headers

use eframe::egui::{self, Color32, FontId, Pos2, RichText, Ui};

use crate::domain::file_entry::{FileEntry, SyncStatus};

/// Renders a section header for grouped views (Computer View)
pub(super) fn render_section_header(ui: &mut Ui, title: &str) {
    ui.add_space(8.0);
    ui.label(
        RichText::new(title)
            .size(11.0)
            .color(Color32::from_gray(120))
            .strong(),
    );
    ui.add_space(4.0);
}

/// PERFORMANCE: Returns Cow<str> — static &str for common extensions, allocated only for rare ones.
pub(super) fn get_file_type_string(item: &FileEntry) -> std::borrow::Cow<'static, str> {
    use std::borrow::Cow;

    // Check for ZIP manually because is_dir might be true (ASCII byte check, no allocation)
    if item.name.len() >= 4 {
        let bytes = item.name.as_bytes();
        let last4 = &bytes[bytes.len() - 4..];
        if last4[0] == b'.'
            && last4[1].to_ascii_lowercase() == b'z'
            && last4[2].to_ascii_lowercase() == b'i'
            && last4[3].to_ascii_lowercase() == b'p'
        {
            return Cow::Borrowed("Arquivo ZIP");
        }
    }
    if item.is_dir {
        return Cow::Borrowed("Pasta");
    }

    if let Some(ext) = item.path.extension() {
        let ext_lower = ext.to_ascii_lowercase();
        let ext_str = ext_lower.to_string_lossy();

        // Static strings for common file types (zero allocation)
        match ext_str.as_ref() {
            "txt" => return Cow::Borrowed("Arquivo TXT"),
            "pdf" => return Cow::Borrowed("Arquivo PDF"),
            "doc" | "docx" => return Cow::Borrowed("Arquivo Word"),
            "xls" | "xlsx" => return Cow::Borrowed("Arquivo Excel"),
            "ppt" | "pptx" => return Cow::Borrowed("Arquivo PowerPoint"),
            "jpg" | "jpeg" => return Cow::Borrowed("Arquivo JPEG"),
            "png" => return Cow::Borrowed("Arquivo PNG"),
            "gif" => return Cow::Borrowed("Arquivo GIF"),
            "bmp" => return Cow::Borrowed("Arquivo BMP"),
            "webp" => return Cow::Borrowed("Arquivo WebP"),
            "mp4" => return Cow::Borrowed("Arquivo MP4"),
            "mkv" => return Cow::Borrowed("Arquivo MKV"),
            "avi" => return Cow::Borrowed("Arquivo AVI"),
            "mov" => return Cow::Borrowed("Arquivo MOV"),
            "wmv" => return Cow::Borrowed("Arquivo WMV"),
            "mp3" => return Cow::Borrowed("Arquivo MP3"),
            "wav" => return Cow::Borrowed("Arquivo WAV"),
            "flac" => return Cow::Borrowed("Arquivo FLAC"),
            "exe" => return Cow::Borrowed("Arquivo Executável"),
            "dll" => return Cow::Borrowed("Biblioteca DLL"),
            "html" | "htm" => return Cow::Borrowed("Arquivo HTML"),
            "css" => return Cow::Borrowed("Arquivo CSS"),
            "js" => return Cow::Borrowed("Arquivo JavaScript"),
            "json" => return Cow::Borrowed("Arquivo JSON"),
            "xml" => return Cow::Borrowed("Arquivo XML"),
            "rs" => return Cow::Borrowed("Arquivo Rust"),
            "py" => return Cow::Borrowed("Arquivo Python"),
            "java" => return Cow::Borrowed("Arquivo Java"),
            "c" | "cpp" | "h" | "hpp" => return Cow::Borrowed("Arquivo C/C++"),
            "lnk" => return Cow::Borrowed("Atalho"),
            "iso" => return Cow::Borrowed("Imagem de Disco"),
            _ => {
                return Cow::Owned(format!(
                    "Arquivo {}",
                    ext.to_string_lossy().to_uppercase()
                ));
            }
        }
    }

    Cow::Borrowed("Arquivo")
}

/// Renders a sync status badge (OneDrive) in the status column
pub(super) fn render_status_badge(ui: &mut egui::Ui, pos: Pos2, status: SyncStatus) {
    if status == SyncStatus::None {
        return; // No badge for normal files
    }

    let badge_size = 16.0;
    let badge_center = pos + egui::vec2(badge_size / 2.0, badge_size / 2.0);
    let badge_radius = badge_size / 2.0;

    let painter = ui.painter();

    match status {
        SyncStatus::CloudOnly => {
            // Blue cloud icon - file needs download
            painter.circle_filled(badge_center, badge_radius, Color32::from_rgb(0, 120, 215));
            painter.text(
                badge_center,
                egui::Align2::CENTER_CENTER,
                "☁",
                FontId::proportional(11.0),
                Color32::WHITE,
            );
        }
        SyncStatus::Syncing => {
            // Blue circular arrows - file is being synced
            painter.circle_filled(badge_center, badge_radius, Color32::from_rgb(0, 120, 215));
            painter.text(
                badge_center,
                egui::Align2::CENTER_CENTER,
                "⟳",
                FontId::proportional(12.0),
                Color32::WHITE,
            );
        }
        SyncStatus::Pinned => {
            // Green solid circle with check - always keep on device
            painter.circle_filled(badge_center, badge_radius, Color32::from_rgb(0, 150, 0));
            painter.text(
                badge_center,
                egui::Align2::CENTER_CENTER,
                "✓",
                FontId::proportional(10.0),
                Color32::WHITE,
            );
        }
        SyncStatus::LocallyAvailable => {
            // White circle with green outline/check - downloaded on demand
            painter.circle_filled(badge_center, badge_radius, Color32::WHITE);
            painter.circle_stroke(
                badge_center,
                badge_radius - 1.0,
                egui::Stroke::new(2.0, Color32::from_rgb(0, 150, 0)),
            );
            painter.text(
                badge_center,
                egui::Align2::CENTER_CENTER,
                "✓",
                FontId::proportional(10.0),
                Color32::from_rgb(0, 150, 0),
            );
        }
        SyncStatus::None => {} // Already handled above
    }
}
