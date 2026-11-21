use crate::domain::file_entry::FileEntry;
use eframe::egui::{self, OutputCommand};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct MarkdownCacheState {
    path: std::path::PathBuf,
    modified: u64,
    size: u64,
    source: Arc<String>,
    cache: Arc<Mutex<CommonMarkCache>>,
}

pub(crate) fn is_markdown(path: &Path) -> bool {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("");
    if matches!(
        extension.to_ascii_lowercase().as_str(),
        "md" | "markdown" | "mdown" | "mkd"
    ) {
        return true;
    }
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("readme"))
}

fn cache_for(ui: &egui::Ui, file: &FileEntry, source: &Arc<String>) -> Arc<Mutex<CommonMarkCache>> {
    let id = egui::Id::new("detail_markdown_cache");
    ui.ctx().data_mut(|data| {
        if let Some(state) = data.get_temp::<MarkdownCacheState>(id) {
            if state.path == file.path
                && state.modified == file.modified
                && state.size == file.size
                && Arc::ptr_eq(&state.source, source)
            {
                return state.cache;
            }
        }
        let cache = Arc::new(Mutex::new(CommonMarkCache::default()));
        data.insert_temp(
            id,
            MarkdownCacheState {
                path: file.path.clone(),
                modified: file.modified,
                size: file.size,
                source: Arc::clone(source),
                cache: Arc::clone(&cache),
            },
        );
        cache
    })
}

pub(super) fn render_markdown_preview(ui: &mut egui::Ui, file: &FileEntry) {
    super::text_preview::render_content_header(
        ui,
        file,
        rust_i18n::t!("preview.markdown_content").to_string(),
    );

    let content = match super::text_preview::load_markdown(ui.ctx(), file, ui.visuals().dark_mode) {
        super::text_preview::PreviewContentStatus::Loading => {
            ui.add(egui::Spinner::new());
            ui.label(rust_i18n::t!("preview.content_loading").to_string());
            return;
        }
        super::text_preview::PreviewContentStatus::Failed => {
            ui.colored_label(
                egui::Color32::from_rgb(232, 17, 35),
                rust_i18n::t!("preview.content_unavailable").to_string(),
            );
            return;
        }
        super::text_preview::PreviewContentStatus::Ready(content) => content,
    };

    let id = egui::Id::new("detail_markdown_scroll").with(&file.path);
    let cache = cache_for(ui, file, &content.text);
    egui::ScrollArea::both()
        .id_salt(id)
        .max_height(480.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut cache = cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let max_image_width = Some(ui.available_width().max(1.0) as usize);
            show_markdown(ui, &mut cache, &content.text, max_image_width);
        });

    if content.truncated {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(rust_i18n::t!("preview.text_partial").to_string())
                .small()
                .color(egui::Color32::GRAY),
        );
    }
}

pub(crate) fn show_markdown(
    ui: &mut egui::Ui,
    cache: &mut CommonMarkCache,
    markdown: &str,
    max_image_width: Option<usize>,
) {
    let commands_before = ui.ctx().output(|output| output.commands.len());
    CommonMarkViewer::new()
        .explicit_image_uri_scheme(true)
        .max_image_width(max_image_width)
        .show(ui, cache, markdown);
    retain_safe_commands(ui, commands_before);
}

fn retain_safe_commands(ui: &egui::Ui, commands_before: usize) {
    ui.ctx().output_mut(|output| {
        retain_safe_links(&mut output.commands, commands_before);
    });
}

fn retain_safe_links(commands: &mut Vec<OutputCommand>, first: usize) {
    let mut index = 0;
    commands.retain(|command| {
        let keep = index < first
            || match command {
                OutputCommand::OpenUrl(url) => url::Url::parse(&url.url).is_ok_and(|url| {
                    url.scheme() == "https"
                        && url.host_str().is_some()
                        && url.username().is_empty()
                        && url.password().is_none()
                }),
                _ => true,
            };
        index += 1;
        keep
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_detection_is_case_insensitive_and_supports_readme() {
        assert!(is_markdown(Path::new("guide.MD")));
        assert!(is_markdown(Path::new("README")));
        assert!(!is_markdown(Path::new("image.png")));
    }

    #[test]
    fn markdown_links_reject_file_and_application_schemes() {
        let mut commands = vec![
            OutputCommand::OpenUrl(egui::OpenUrl::new_tab("file:///file.txt")),
            OutputCommand::OpenUrl(egui::OpenUrl::new_tab("javascript:alert(1)")),
            OutputCommand::OpenUrl(egui::OpenUrl::new_tab("https://example.org/page")),
        ];
        retain_safe_links(&mut commands, 0);
        assert_eq!(commands.len(), 1);
        assert!(
            matches!(&commands[0], OutputCommand::OpenUrl(url) if url.url == "https://example.org/page")
        );
    }
}
