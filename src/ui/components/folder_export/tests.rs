use super::*;
use crate::domain::{
    date_format::DateFormat,
    file_entry::{FoldersPosition, SortMode},
    list_columns::ColumnId,
};

fn state() -> FolderExportState {
    FolderExportState {
        options: Options {
            root: std::path::PathBuf::from("folder"),
            selection: None,
            recursive: false,
            contents: Contents::Both,
            format: Format::Csv,
            csv_separator: CsvSeparator::Semicolon,
            calculate_sizes: false,
            folder_sizes: Default::default(),
            tree: true,
            columns: vec![
                ExportColumn {
                    field: Field::Column(ColumnId::Name),
                    label: "Name".into(),
                    enabled: true,
                },
                ExportColumn {
                    field: Field::Column(ColumnId::Size),
                    label: "Size".into(),
                    enabled: true,
                },
            ],
            sort: SortMode::Name,
            descending: false,
            folders: FoldersPosition::First,
            date_format: DateFormat::YearMonthDay,
            tags: Default::default(),
            tag_names: Default::default(),
        },
        selected_paths: vec!["file.txt".into()],
        selected_only: false,
        clipboard: false,
        job: None,
        status: None,
    }
}

struct Snapshot {
    text: Vec<(String, egui::Pos2, f32)>,
    rect: egui::Rect,
    body_size: f32,
}

fn render(width: f32, expanded: bool) -> Snapshot {
    let ctx = egui::Context::default();
    let mut state = state();
    let mut rect = egui::Rect::NOTHING;
    let mut body_size = 0.0;
    let mut draw = |events, time| {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 2000.0),
                )),
                events,
                time: Some(time),
                ..Default::default()
            },
            |ui| {
                ui.set_width(width);
                body_size = egui::TextStyle::Body.resolve(ui.style()).size;
                controls::render(ui, &mut state);
                rect = ui.min_rect();
            },
        )
    };
    let mut output = draw(Vec::new(), 0.0);
    if expanded {
        let mut time = 0.1;
        for label in [t!("export_listing.columns"), t!("export_listing.sorting")] {
            let position = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text == label => {
                        Some(text.pos + egui::vec2(5.0, 5.0))
                    }
                    _ => None,
                })
                .expect("advanced section header");
            for pressed in [true, false] {
                let _ = draw(
                    vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    time,
                );
                time += 0.1;
            }
            // Let the native collapsing animation finish before measuring.
            time += 1.0;
            output = draw(Vec::new(), time);
        }
    }
    let text = output
        .shapes
        .into_iter()
        .filter_map(|shape| match shape.shape {
            egui::Shape::Text(text) => Some((
                text.galley.job.text.clone(),
                text.pos,
                text.galley.job.sections[0].format.font_id.size,
            )),
            _ => None,
        })
        .collect();
    Snapshot {
        text,
        rect,
        body_size,
    }
}

#[test]
fn compact_export_layout_keeps_top_columns_and_hides_advanced_options() {
    let snapshot = render(680.0, false);
    let source = snapshot
        .text
        .iter()
        .find(|(text, _, _)| text == &t!("export_listing.source_content"))
        .unwrap();
    let output = snapshot
        .text
        .iter()
        .find(|(text, _, _)| text == &t!("export_listing.format_destination"))
        .unwrap();
    assert!((source.1.y - output.1.y).abs() < 1.0);
    assert!(output.1.x - source.1.x > 300.0);
    assert!(
        snapshot.rect.height() <= 470.0,
        "default export controls are too tall"
    );
    assert!(!snapshot.text.iter().any(|(text, _, _)| text == "Size"));
    assert!(!snapshot
        .text
        .iter()
        .any(|(text, _, _)| text == &t!("export_listing.sort_by")));
}

#[test]
fn narrow_export_layout_stacks_sections_without_horizontal_overflow() {
    let snapshot = render(360.0, false);
    let source = snapshot
        .text
        .iter()
        .find(|(text, _, _)| text == &t!("export_listing.source_content"))
        .unwrap();
    let output = snapshot
        .text
        .iter()
        .find(|(text, _, _)| text == &t!("export_listing.format_destination"))
        .unwrap();
    assert!(output.1.y > source.1.y + 150.0);
    assert!((output.1.x - source.1.x).abs() < 1.0);
    assert!(
        snapshot.rect.width() <= 361.0,
        "export controls exceeded their available width"
    );
}

#[test]
fn size_explanation_uses_readable_body_font() {
    let snapshot = render(680.0, false);
    let hint = snapshot
        .text
        .iter()
        .find(|(text, _, _)| text == &t!("export_listing.sizes_hint"))
        .unwrap();
    assert_eq!(hint.2, snapshot.body_size);
}

#[test]
fn advanced_options_remain_available_and_compact_when_expanded() {
    let snapshot = render(680.0, true);
    assert!(snapshot.text.iter().any(|(text, _, _)| text == "Size"));
    assert!(snapshot
        .text
        .iter()
        .any(|(text, _, _)| text == &t!("export_listing.sort_by")));
    assert!(
        snapshot.rect.height() <= 720.0,
        "expanded export controls are too tall"
    );
    assert!(
        snapshot.rect.width() <= 681.0,
        "expanded export controls overflow horizontally"
    );
}

#[test]
fn footer_stays_next_to_separator_instead_of_filling_remaining_height() {
    for busy in [false, true] {
        let ctx = egui::Context::default();
        let mut footer_rect = egui::Rect::NOTHING;
        let mut separator_bottom = 0.0;
        let mut content_bottom = 0.0;
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 1400.0),
                )),
                ..Default::default()
            },
            |ui| {
                ui.set_width(680.0);
                ui.spacing_mut().item_spacing = egui::vec2(10.0, 6.0);
                ui.label("Content");
                separator_bottom = ui.separator().rect.bottom();
                footer_rect = footer::render(ui, busy, true).response.rect;
                content_bottom = ui.min_rect().bottom();
            },
        );
        assert!(
            footer_rect.height() <= 40.0,
            "footer consumed the remaining window height"
        );
        assert!(footer_rect.top() - separator_bottom <= 8.0);
        assert!(content_bottom - footer_rect.bottom() <= 1.0);
        for shape in output.shapes {
            if let egui::Shape::Text(text) = shape.shape {
                if text.galley.job.text != "Content" {
                    assert!(
                        text.pos.y - separator_bottom <= 24.0,
                        "button was pushed away from the separator"
                    );
                }
            }
        }
    }
}
