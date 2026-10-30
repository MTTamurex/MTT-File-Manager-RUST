use super::*;
use mtt_search_protocol::{DiskAnalysisRecord, DiskAnalysisSnapshot};

#[test]
fn metadata_columns_fit_full_text_even_when_the_table_needs_horizontal_scroll() {
    let mut model = DiskAnalysisModel::build(DiskAnalysisSnapshot {
        drive_letter: 'R',
        records: vec![DiskAnalysisRecord {
            frn: 5,
            parent_frn: 5,
            name: String::new(),
            size: 0,
            allocated_size: 0,
            is_dir: true,
            is_reparse: false,
        }],
    });
    model.nodes[model.root as usize].subtree_files = u64::MAX;
    model.nodes[model.root as usize].subtree_size = u64::MAX;
    let mut state = DiskAnalysisState::new();
    state.largest_rows = Arc::new(vec![model.root]);
    let ctx = egui::Context::default();
    let _ = ctx.run_ui(Default::default(), |ui| {
        for available in [300.0, 1800.0] {
            let widths = widths(ui, available, &model, &state);
            let rect = egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(widths.iter().sum(), ROW_HEIGHT),
            );
            let mut x = 0.0;
            for label in [
                t!("disk_analysis.type_file"),
                t!("disk_analysis.type_folder"),
            ] {
                assert!(!paint_cell(
                    ui.painter(),
                    rect,
                    &mut x,
                    widths[2],
                    &label,
                    false,
                    analyzer_text_color(ui)
                ));
            }
            for (slot, text) in values(&model, model.root, None).iter().enumerate() {
                assert!(!paint_cell(
                    ui.painter(),
                    rect,
                    &mut x,
                    widths[slot + 3],
                    text,
                    true,
                    analyzer_text_color(ui)
                ));
            }
            if available < 400.0 {
                assert!(widths.iter().sum::<f32>() > available);
            }
            assert!(paint_cell(
                ui.painter(),
                rect,
                &mut x,
                60.0,
                &"Example long name ".repeat(10),
                false,
                analyzer_text_color(ui)
            ));
        }
    });
}
