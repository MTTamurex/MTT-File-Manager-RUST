use super::{needs_refit, record_fitted_width};
use crate::domain::file_entry::{FileEntry, SyncStatus};
use crate::ui::views::list_view::auto_fit_columns;
use eframe::egui;
use lru::LruCache;
use std::num::NonZeroUsize;

fn sample_items() -> Vec<FileEntry> {
    vec![FileEntry {
        path: "file.txt".into(),
        name: "file.txt".into(),
        is_dir: false,
        size: 1024,
        modified: 1_700_000_000,
        created: None,
        folder_cover: None,
        drive_info: None,
        sync_status: SyncStatus::None,
        is_hidden: false,
        recycle_bin: None,
    }]
}

fn render_pane(
    ui: &mut egui::Ui,
    pane: &str,
    width: f32,
    pending: &mut bool,
    columns: &mut [f32; 5],
    items: &[FileEntry],
) -> bool {
    let mut fitted = false;
    ui.scope_builder(
        egui::UiBuilder::new()
            .id(egui::Id::new(pane))
            .max_rect(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 600.0),
            )),
        |ui| {
            if needs_refit(ui, *pending) {
                let [name, date, type_col, size, status] = columns;
                fitted = auto_fit_columns(
                    ui,
                    items,
                    false,
                    false,
                    false,
                    ui.available_width(),
                    name,
                    date,
                    type_col,
                    size,
                    status,
                    &LruCache::new(NonZeroUsize::new(1).unwrap()),
                );
                if fitted {
                    *pending = false;
                    record_fitted_width(ui);
                }
            }
        },
    );
    fitted
}

#[test]
fn restored_inactive_pane_refits_after_window_geometry_settles() {
    let ctx = egui::Context::default();
    let items = sample_items();
    let mut left_pending = true;
    let mut right_pending = true;
    let mut left = [100.0; 5];
    let mut right = [100.0; 5];

    let _ = ctx.run_ui(Default::default(), |ui| {
        // Active right pane is rendered before the restored inactive left pane.
        assert!(render_pane(
            ui,
            "right",
            600.0,
            &mut right_pending,
            &mut right,
            &items
        ));
        assert!(render_pane(
            ui,
            "left",
            420.0,
            &mut left_pending,
            &mut left,
            &items
        ));
    });
    assert!(!left_pending);
    let provisional_name_width = left[0];
    let fitted_right = right;

    let _ = ctx.run_ui(Default::default(), |ui| {
        assert!(!render_pane(
            ui,
            "right",
            600.0,
            &mut right_pending,
            &mut right,
            &items
        ));
        assert!(render_pane(
            ui,
            "left",
            1000.0,
            &mut left_pending,
            &mut left,
            &items
        ));
    });
    assert!(left[0] > provisional_name_width + 400.0);
    assert!((left[..4].iter().sum::<f32>() - 992.0).abs() < 1.0);
    assert_eq!(right, fitted_right);

    // A manual column edit survives subsequent frames with unchanged geometry.
    left[0] -= 50.0;
    let manual_columns = left;
    let _ = ctx.run_ui(Default::default(), |ui| {
        assert!(!render_pane(
            ui,
            "left",
            1000.0,
            &mut left_pending,
            &mut left,
            &items
        ));
    });
    assert_eq!(left, manual_columns);
}

#[test]
fn unavailable_content_or_width_does_not_complete_the_fit() {
    let ctx = egui::Context::default();
    let items = sample_items();
    let mut pending = true;
    let mut columns = [100.0; 5];
    for (width, content) in [(1000.0, &[][..]), (4.0, items.as_slice())] {
        let _ = ctx.run_ui(Default::default(), |ui| {
            assert!(!render_pane(
                ui,
                "left",
                width,
                &mut pending,
                &mut columns,
                content
            ));
        });
        assert!(pending);
        assert_eq!(columns, [100.0; 5]);
    }
    let _ = ctx.run_ui(Default::default(), |ui| {
        assert!(render_pane(
            ui,
            "left",
            1000.0,
            &mut pending,
            &mut columns,
            &items
        ));
    });
    assert!(!pending);
    assert!((columns[..4].iter().sum::<f32>() - 992.0).abs() < 1.0);
}
