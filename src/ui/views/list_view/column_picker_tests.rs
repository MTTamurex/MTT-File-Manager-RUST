use super::*;

#[test]
fn menu_groups_active_columns_without_changing_the_saved_layout() {
    use crate::domain::list_columns::ColumnScope;
    for scope in [
        ColumnScope::Files,
        ColumnScope::Computer,
        ColumnScope::RecycleBin,
    ] {
        let mut layout = ColumnLayout::defaults(scope);
        layout.set_visible(ColumnId::Name, false);
        let hidden = layout.columns[1].id;
        layout.set_visible(hidden, false);
        layout.set_visible(ColumnId::Tags, true);
        let saved = layout.clone();
        let ids = menu_ids(&layout, scope, true);
        let expected: Vec<_> = layout
            .columns
            .iter()
            .filter(|c| c.visible)
            .chain(layout.columns.iter().filter(|c| !c.visible))
            .map(|c| c.id)
            .collect();
        assert_eq!(ids, expected);
        assert_eq!(ids.last(), Some(&hidden));
        assert_eq!(layout, saved);
        assert!(layout.is_visible(ColumnId::Name));

        layout.set_visible(hidden, true);
        assert_eq!(
            menu_ids(&layout, scope, true),
            layout.columns.iter().map(|c| c.id).collect::<Vec<_>>()
        );
    }
}

#[test]
fn menu_retains_hidden_defaults_and_filters_status_outside_onedrive() {
    use crate::domain::list_columns::ColumnScope;
    let mut layout = ColumnLayout::defaults(ColumnScope::Files);
    layout.set_visible(ColumnId::Status, false);
    layout.set_visible(ColumnId::Type, false);
    let ids = menu_ids(&layout, ColumnScope::Files, false);
    assert!(!ids.contains(&ColumnId::Status));
    assert!(ids.contains(&ColumnId::Type));
    assert!(menu_ids(&layout, ColumnScope::Files, true).contains(&ColumnId::Status));
}

#[test]
fn hidden_extras_leave_the_menu_but_remain_available_in_the_picker() {
    use crate::domain::list_columns::PropertyId;
    let property = ColumnId::Property(PropertyId { fmtid: 0, pid: 42 });
    for scope in [
        ColumnScope::Files,
        ColumnScope::Computer,
        ColumnScope::RecycleBin,
    ] {
        let mut layout = ColumnLayout::defaults(scope);
        let defaults = layout.clone();
        for extra in [ColumnId::Tags, ColumnId::OriginalLocation, property] {
            layout.set_visible(extra, true);
            assert!(menu_ids(&layout, scope, true).contains(&extra));
            layout.set_visible(extra, false);
        }
        let saved = layout.clone();
        assert_eq!(
            menu_ids(&layout, scope, true),
            defaults
                .columns
                .iter()
                .map(|column| column.id)
                .collect::<Vec<_>>()
        );
        let choices = column_choices::choices(&layout, &[], scope, |_| "Label".into());
        for extra in [ColumnId::Tags, ColumnId::OriginalLocation, property] {
            assert!(choices.iter().any(|(id, _, _)| *id == extra));
        }
        assert_eq!(layout, saved);

        layout.set_visible(property, true);
        assert!(menu_ids(&layout, scope, true).contains(&property));
    }
}

#[test]
fn default_menu_columns_are_specific_to_each_scope() {
    for scope in [
        ColumnScope::Files,
        ColumnScope::Computer,
        ColumnScope::RecycleBin,
    ] {
        let mut layout = ColumnLayout::defaults(scope);
        let defaults = layout.clone();
        for id in [ColumnId::Date, ColumnId::TotalSpace, ColumnId::Status] {
            layout.set_visible(id, false);
            assert_eq!(
                menu_ids(&layout, scope, true).contains(&id),
                defaults.is_visible(id)
            );
        }
    }
}

#[test]
fn picker_uses_compact_width_and_keeps_the_scrollbar_at_the_content_right_edge() {
    let context = egui::Context::default();
    for _ in 0..3 {
        let _ = context.run_ui(Default::default(), |ui| {
            let window =
                picker_window(ui.ctx(), egui::Id::new("test_picker")).show(ui.ctx(), |ui| {
                    let width = ui.available_width();
                    assert!(
                        width <= 441.0,
                        "picker must not expand to unrelated viewport width"
                    );
                    let right = ui.available_rect_before_wrap().right();
                    let output = choices_scroll(ui, |ui| {
                        for _ in 0..100 {
                            ui.checkbox(&mut false, "Example property");
                        }
                    });
                    assert!(
                        right - output.inner_rect.right() < 24.0,
                        "scrollbar must not sit in the middle of the picker"
                    );
                    assert!(output.content_size.y > output.inner_rect.height());
                });
            assert!(window.is_some());
        });
    }
}

#[test]
fn long_property_labels_and_search_text_do_not_expand_the_picker_or_displace_its_scrollbar() {
    let context = egui::Context::default();
    let mut search = "Example search text ".repeat(30);
    for _ in 0..3 {
        let _ = context.run_ui(Default::default(), |ui| {
            picker_window(ui.ctx(), egui::Id::new("long_labels_picker")).show(ui.ctx(), |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                let available = ui.available_width();
                let search_response =
                    ui.add(egui::TextEdit::singleline(&mut search).desired_width(f32::INFINITY));
                assert!((search_response.rect.width() - available).abs() < 2.0);
                let right = ui.available_rect_before_wrap().right();
                let output = choices_scroll(ui, |ui| {
                    for _ in 0..30 {
                        ui.checkbox(
                            &mut false,
                            "Example long property label with several descriptive words ".repeat(4),
                        );
                    }
                });
                assert!(ui.min_rect().width() <= 441.0);
                assert!(right - output.inner_rect.right() < 24.0);
            });
        });
    }
}

#[test]
fn picker_opens_centered_and_recenters_after_being_closed() {
    let ctx = egui::Context::default();
    let id = egui::Id::new("centered_picker");
    for _ in 0..3 {
        let _ = ctx.run_ui(Default::default(), |ui| {
            let window = picker_window(ui.ctx(), id)
                .show(ui.ctx(), |ui| {
                    ui.label("Example property");
                })
                .unwrap();
            assert!(
                window
                    .response
                    .rect
                    .center()
                    .distance(ui.ctx().viewport_rect().center())
                    < 2.0
            );
            assert!(window.inner.is_some(), "picker is not collapsed");
        });
    }
    let displaced = ctx.viewport_rect().center() + egui::vec2(80.0, 60.0);
    let _ = ctx.run_ui(Default::default(), |ui| {
        picker_window(ui.ctx(), id)
            .movable(false)
            .current_pos(displaced)
            .show(ui.ctx(), |ui| {
                ui.label("Example property");
            });
    });
    let _ = ctx.run_ui(Default::default(), |ui| {
        let window = picker_window(ui.ctx(), id)
            .show(ui.ctx(), |ui| {
                ui.label("Example property");
            })
            .unwrap();
        assert!(
            window.response.rect.center().distance(displaced) < 2.0,
            "open picker remains movable"
        );
    });
    for _ in 0..2 {
        let _ = ctx.run_ui(Default::default(), |_| {});
    }
    ctx.data_mut(|data| data.insert_temp(id.with("center_on_open"), true));
    let _ = ctx.run_ui(Default::default(), |ui| {
        let window = picker_window(ui.ctx(), id)
            .show(ui.ctx(), |ui| {
                ui.label("Example property");
            })
            .unwrap();
        assert!(
            window
                .response
                .rect
                .center()
                .distance(ui.ctx().viewport_rect().center())
                < 2.0
        );
    });
}
