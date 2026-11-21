use crate::domain::file_entry::FileEntry;
use crate::ui::svg_icons::SvgIconManager;
use eframe::egui;
#[cfg(test)]
use std::path::Path;

enum ContentKind {
    Markdown,
    Text,
    Pdf,
    Svg,
}

fn content_kind(file: &FileEntry, ctx: &eframe::egui::Context) -> Option<ContentKind> {
    if file.is_dir || crate::domain::file_entry::is_path_inside_archive(&file.path) {
        return None;
    }

    let extension = file
        .path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("");
    if extension.eq_ignore_ascii_case("pdf") {
        Some(ContentKind::Pdf)
    } else if extension.eq_ignore_ascii_case("svg") {
        Some(ContentKind::Svg)
    } else if crate::ui::preview_panel::markdown_preview::is_markdown(&file.path) {
        Some(ContentKind::Markdown)
    } else if crate::ui::preview_panel::text_preview::is_previewable(&file.path, ctx) {
        Some(ContentKind::Text)
    } else {
        None
    }
}

pub(super) fn render(
    ui: &mut egui::Ui,
    file: &FileEntry,
    svg_manager: &mut SvgIconManager,
) -> bool {
    let Some(kind) = content_kind(file, ui.ctx()) else {
        return false;
    };

    match kind {
        ContentKind::Markdown => super::markdown_preview::render_markdown_preview(ui, file),
        ContentKind::Text => super::text_preview::render_text_preview(ui, file),
        ContentKind::Pdf => super::pdf_preview::render_pdf_preview(ui, file, svg_manager),
        ContentKind::Svg => super::svg_preview::render_svg_preview(ui, file, svg_manager),
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, is_dir: bool) -> FileEntry {
        FileEntry {
            path: Path::new(path).to_path_buf(),
            name: Path::new(path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            is_dir,
            size: 0,
            modified: 0,
            created: None,
            folder_cover: None,
            drive_info: None,
            sync_status: crate::domain::file_entry::SyncStatus::None,
            is_hidden: false,
            recycle_bin: None,
        }
    }

    #[test]
    fn classifies_supported_content_and_excludes_folders() {
        let ctx = eframe::egui::Context::default();
        assert!(matches!(
            content_kind(&entry("document.pdf", false), &ctx),
            Some(ContentKind::Pdf)
        ));
        assert!(matches!(
            content_kind(&entry("notes.md", false), &ctx),
            Some(ContentKind::Markdown)
        ));
        assert!(matches!(
            content_kind(&entry("source.rs", false), &ctx),
            Some(ContentKind::Text)
        ));
        assert!(matches!(
            content_kind(&entry("drawing.svg", false), &ctx),
            Some(ContentKind::Svg)
        ));
        assert!(content_kind(&entry("notes.md", true), &ctx).is_none());
    }

    #[test]
    fn archive_entries_do_not_start_content_previews() {
        assert!(content_kind(
            &entry("archive.zip\\folder\\notes.md", false),
            &eframe::egui::Context::default()
        )
        .is_none());
    }
}
