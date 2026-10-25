//! Additional enabled columns shared by the toolbar's sorting and grouping selectors.
use crate::app::ImageViewerApp;
use crate::domain::list_columns::{ColumnId, ColumnLayout, ColumnScope};
use crate::infrastructure::windows::column_properties::PropertyColumn;

pub(super) fn label(id: ColumnId, catalog: &[PropertyColumn]) -> String {
    match id {
        ColumnId::Tags => rust_i18n::t!("column_headers.app_tags").to_string(),
        ColumnId::OriginalLocation => rust_i18n::t!("column_headers.original_location").to_string(),
        _ => catalog
            .iter()
            .find(|column| column.id == id)
            .map(|column| column.label.clone())
            .unwrap_or_else(|| rust_i18n::t!("column_headers.custom_sort").to_string()),
    }
}

fn additional(layout: &ColumnLayout, catalog: &[PropertyColumn]) -> Vec<(ColumnId, String)> {
    layout
        .columns
        .iter()
        .filter(|column| column.visible)
        .filter(|column| {
            matches!(
                column.id,
                ColumnId::Property(_) | ColumnId::Tags | ColumnId::OriginalLocation
            )
        })
        .map(|column| (column.id, label(column.id, catalog)))
        .collect()
}

pub(super) fn for_app(app: &ImageViewerApp) -> Vec<(ColumnId, String)> {
    let scope = ColumnScope::from_view(
        app.navigation_state.is_computer_view,
        app.navigation_state.is_recycle_bin_view,
    );
    additional(app.columns.preferences.layout(scope), &app.columns.catalog)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::list_columns::PropertyId;
    #[test]
    fn selectors_follow_enabled_columns_and_their_current_order_without_default_duplicates() {
        let property = ColumnId::Property(PropertyId { fmtid: 1, pid: 3 });
        let catalog = vec![PropertyColumn {
            id: property,
            canonical_name: "System.Media.Duration".into(),
            label: "Length".into(),
        }];
        let mut layout = ColumnLayout::defaults(ColumnScope::Files);
        layout.set_visible(property, true);
        layout.set_visible(ColumnId::Tags, true);
        layout.move_before(ColumnId::Tags, Some(property));
        let choices = additional(&layout, &catalog);
        assert_eq!(
            choices.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            [ColumnId::Tags, property]
        );
        assert_eq!(choices[1].1, "Length");
        layout.set_visible(property, false);
        assert_eq!(additional(&layout, &catalog).len(), 1);
    }

    #[test]
    fn hidden_columns_leave_the_menu_but_keep_the_label_used_by_an_active_criterion() {
        let id = ColumnId::Property(PropertyId { fmtid: 1, pid: 3 });
        let catalog = vec![PropertyColumn {
            id,
            canonical_name: "System.Media.Duration".into(),
            label: "Length".into(),
        }];
        let mut layout = ColumnLayout::defaults(ColumnScope::Files);
        layout.set_visible(id, true);
        assert!(additional(&layout, &catalog)
            .iter()
            .any(|(column, _)| *column == id));
        layout.set_visible(id, false);
        assert!(!additional(&layout, &catalog)
            .iter()
            .any(|(column, _)| *column == id));
        assert_eq!(label(id, &catalog), "Length");
    }

    #[test]
    fn selectors_only_use_the_layout_of_the_current_view_scope() {
        let id = ColumnId::Property(PropertyId { fmtid: 1, pid: 3 });
        let mut preferences = crate::domain::list_columns::ColumnPreferences::default();
        preferences.files.set_visible(id, true);
        preferences
            .recycle_bin
            .set_visible(ColumnId::OriginalLocation, true);
        assert_eq!(
            additional(preferences.layout(ColumnScope::Files), &[])
                .iter()
                .map(|(id, _)| *id)
                .collect::<Vec<_>>(),
            [id]
        );
        assert!(additional(preferences.layout(ColumnScope::Computer), &[]).is_empty());
        assert_eq!(
            additional(preferences.layout(ColumnScope::RecycleBin), &[])
                .iter()
                .map(|(id, _)| *id)
                .collect::<Vec<_>>(),
            [ColumnId::OriginalLocation]
        );
    }
}
