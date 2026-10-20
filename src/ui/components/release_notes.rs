use eframe::egui::{self, OutputCommand};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

pub(crate) fn show(ui: &mut egui::Ui, cache: &mut CommonMarkCache, markdown: &str) {
    ui.scope(|ui| {
        // Settings intentionally uses zero vertical spacing. Markdown needs its
        // own paragraph/list rhythm, independent of that surrounding layout.
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
        ui.style_mut().url_in_tooltip = true;
        let commands_before = ui.ctx().output(|output| output.commands.len());
        // Image/file/network loaders are disabled in this dependency. HTML is
        // not executed, and relative images must never become local file URLs.
        CommonMarkViewer::new()
            .explicit_image_uri_scheme(true)
            .show(ui, cache, markdown);
        ui.ctx().output_mut(|output| {
            retain_safe_links(&mut output.commands, commands_before);
        });
    });
}

fn retain_safe_links(commands: &mut Vec<OutputCommand>, first: usize) {
    let mut index = 0;
    commands.retain(|command| {
        let keep = index < first
            || match command {
                OutputCommand::OpenUrl(url) => reqwest::Url::parse(&url.url).is_ok_and(|url| {
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
    fn release_links_cannot_open_files_or_application_protocols() {
        let mut commands = vec![OutputCommand::OpenUrl(egui::OpenUrl::new_tab(
            "file:///previous-control",
        ))];
        for url in [
            "https://example.org/release",
            "file:///file.txt",
            "javascript:alert(1)",
            "custom-app:command",
            "http://example.org",
            "https://user@example.org",
        ] {
            commands.push(OutputCommand::OpenUrl(egui::OpenUrl::new_tab(url)));
        }
        retain_safe_links(&mut commands, 1);
        assert_eq!(commands.len(), 2);
        assert!(
            matches!(&commands[1], OutputCommand::OpenUrl(url) if url.url == "https://example.org/release")
        );
    }

    #[test]
    fn release_notes_render_headings_lists_and_emphasis_without_markdown_markers() {
        let ctx = egui::Context::default();
        let mut cache = CommonMarkCache::default();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            show(
                ui,
                &mut cache,
                "# Release notes\n\n## Fixes\n\n- **Fixed** a problem.\n- Added `code` support.",
            );
        });
        let rendered = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) => Some(text.galley.text()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ");
        assert!(rendered.contains("Release notes"));
        assert!(rendered.contains("Fixed"));
        assert!(!rendered.contains("# Release"));
        assert!(!rendered.contains("**Fixed**"));
        assert!(!rendered.contains("`code`"));
    }
}
