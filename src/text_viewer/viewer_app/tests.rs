use super::*;

#[test]
fn viewer_opens_utf16_log_content() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("sample.log");
    std::fs::write(&path, b"\xff\xfeL\0o\0g\0 \0l\0i\0n\0e\0\r\0\n\0").unwrap();

    let app = TextViewerApp::new(path, false).unwrap();
    assert_eq!(app.content.as_str(), "Log line\r\n");
    assert_eq!(app.encoding_label, "UTF-16LE");
    assert!(!app.is_markdown);
}

#[test]
fn markdown_viewer_renders_sanitized_content() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("guide.md");
    std::fs::write(
        &path,
        "# Heading\n\n![remote](https://example.org/image.png)\n\n**bold**",
    )
    .unwrap();

    let mut app = TextViewerApp::new(path, false).unwrap();
    assert!(app.is_markdown);
    assert!(!app.content.contains("https://"));
    assert!(app.content.contains("Heading"));
    assert!(app.content.contains("bold"));

    let context = egui::Context::default();
    let _ = context.run_ui(egui::RawInput::default(), |ui| app.show_content(ui));
}

#[test]
fn markdown_viewer_renders_only_visible_blocks_after_measurement() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("large-guide.md");
    let mut markdown = String::new();
    for section in 0..300 {
        markdown.push_str(&format!(
            "## Section {section}\n\nA paragraph with **formatted text** and a [safe link](https://example.org).\n\n```\nExample license text.\n```\n\n"
        ));
    }
    markdown.push_str(
        "## Mixed block types\n\n```rust\nfn example() {}\n```\n\n| left | right |\n| --- | --- |\n| one | two |\n",
    );
    std::fs::write(&path, markdown).unwrap();

    let mut app = TextViewerApp::new(path.clone(), false).unwrap();
    let context = egui::Context::default();
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    for _ in 0..3 {
        let _ = context.run_ui(
            egui::RawInput {
                screen_rect: Some(viewport),
                ..Default::default()
            },
            |ui| app.show_content(ui),
        );
    }

    assert!(app.markdown_document.rendered_blocks.len() < 30);
    assert!(!app.markdown_document.rendered_blocks.is_empty());
}
