//! Invalidate shared column widths when returning from a split workspace.

pub(super) fn pending_on_restore(
    was_dual_panel: bool,
    is_dual_panel: bool,
    listing_empty: bool,
    restored_panel_pending: bool,
) -> bool {
    // Single-panel tabs share LayoutState widths, which a dual-panel tab can
    // replace. The pane's cached fitted viewport width does not detect this.
    listing_empty || (was_dual_panel && !is_dual_panel) || (is_dual_panel && restored_panel_pending)
}

#[cfg(test)]
mod tests {
    use super::pending_on_restore;
    use crate::domain::list_columns::ColumnId;
    use crate::ui::views::list_view::autofit::{allocate, ColumnMeasurement};
    use crate::ui::views::list_view::autofit_layout::{needs_refit, record_fitted_width};
    use eframe::egui;

    #[test]
    fn loaded_normal_tab_refits_on_every_return_from_dual_panel() {
        let ctx = egui::Context::default();
        let measured = vec![
            ColumnMeasurement {
                id: ColumnId::Name,
                width: 100.0,
            },
            ColumnMeasurement {
                id: ColumnId::Date,
                width: 120.0,
            },
            ColumnMeasurement {
                id: ColumnId::Type,
                width: 80.0,
            },
            ColumnMeasurement {
                id: ColumnId::Size,
                width: 80.0,
            },
        ];
        for _ in 0..3 {
            // Restoring a dual-panel tab replaces the shared LayoutState widths.
            let mut columns = [100.0; 5];
            let _ = ctx.run_ui(Default::default(), |ui| {
                // Same physical pane and viewport as the normal tab's last fit.
                record_fitted_width(ui);
                assert!(!needs_refit(ui, false));
                let pending = pending_on_restore(true, false, false, false);
                assert!(needs_refit(ui, pending));
                let widths = allocate(ui.available_width(), &measured).unwrap();
                for (slot, (_, width)) in columns.iter_mut().zip(widths) {
                    *slot = width;
                }
                assert!(
                    (columns[..4].iter().sum::<f32>() - (ui.available_width() - 8.0)).abs() < 1.0
                );
                assert!(columns[0] > 100.0);
                record_fitted_width(ui);
                assert!(!needs_refit(ui, false));
            });
        }
    }

    #[test]
    fn unchanged_layouts_do_not_reset_manual_column_edits() {
        assert!(!pending_on_restore(false, false, false, false));
        assert!(!pending_on_restore(true, true, false, false));
        assert!(!pending_on_restore(false, true, false, false));
    }

    #[test]
    fn empty_listings_and_pending_dual_panel_fits_are_preserved() {
        for was_dual_panel in [false, true] {
            for is_dual_panel in [false, true] {
                assert!(pending_on_restore(
                    was_dual_panel,
                    is_dual_panel,
                    true,
                    false
                ));
            }
            assert!(pending_on_restore(was_dual_panel, true, false, true));
        }
        assert!(!pending_on_restore(false, false, false, true));
    }
}
