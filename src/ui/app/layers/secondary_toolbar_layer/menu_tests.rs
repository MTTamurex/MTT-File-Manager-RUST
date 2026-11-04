use eframe::egui;

fn has_scrollbar(output: &egui::FullOutput) -> bool {
    output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .any(|(_, node)| node.role() == egui::accesskit::Role::ScrollBar)
}

#[test]
fn overflowing_menus_keep_a_working_scrollbar() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::ui::theme::apply_scroll_style(&ctx);
    let mut viewport = egui::Rect::NOTHING;
    let mut first_row = egui::Rect::NOTHING;
    let mut initial_top = 0.0;
    for frame in 0..12 {
        let events = if frame > 8 {
            vec![
                egui::Event::PointerMoved(viewport.center()),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: egui::vec2(0.0, -60.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        } else {
            vec![]
        };
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                time: Some(frame as f64 * 0.1),
                events,
                ..Default::default()
            },
            |ui| {
                let combo = egui::ComboBox::from_id_salt("long_menu_test")
                    .popup_style(super::menu_popup_style.into())
                    .selected_text("Option");
                if frame == 0 {
                    egui::Popup::open_id(
                        ui.ctx(),
                        ui.make_persistent_id(egui::IdSalt::new("long_menu_test"))
                            .with("popup"),
                    );
                }
                combo.show_ui(ui, |ui| {
                    viewport = ui.max_rect().intersect(ui.clip_rect());
                    for index in 0..32 {
                        let row = ui.selectable_label(index == 0, format!("Option {index}"));
                        if index == 0 {
                            first_row = row.rect;
                        }
                    }
                });
            },
        );
        if frame == 8 {
            initial_top = first_row.top();
            assert!(has_scrollbar(&output), "test must detect a real scrollbar");
            let bar = output
                .platform_output
                .accesskit_update
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .find(|(_, node)| node.role() == egui::accesskit::Role::ScrollBar)
                .unwrap()
                .1
                .bounds()
                .unwrap();
            assert!(
                f64::from(first_row.right()) <= bar.x0 - 4.0,
                "menu items must not overlap the scrollbar's expanded hit target"
            );
        }
    }
    assert!(
        first_row.top() < initial_top,
        "wheel must scroll overflowing options"
    );
}

#[test]
fn short_menus_do_not_scroll_at_fractional_text_and_display_scales() {
    for text_scale in [1.0, 1.05, 1.2] {
        for dpi in [1.0, 1.25, 1.5, 1.75, 2.0] {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            ctx.set_pixels_per_point(dpi);
            crate::ui::theme::apply_scroll_style(&ctx);
            let mut fonts = egui::FontDefinitions::default();
            crate::ui::fonts::install_system_fonts(&mut fonts);
            crate::ui::fonts::sizing::apply(&mut fonts, text_scale);
            ctx.set_fonts(fonts);
            let mut pointer = egui::Pos2::ZERO;
            let mut previous_rows = Vec::new();
            for frame in 0..16 {
                let mut rows = Vec::new();
                let output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(800.0, 600.0),
                        )),
                        time: Some(frame as f64 * 0.1),
                        events: vec![egui::Event::PointerMoved(pointer)],
                        ..Default::default()
                    },
                    |ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(12.0, 0.0);
                        let combo = egui::ComboBox::from_id_salt("sort_test")
                            .popup_style(super::menu_popup_style.into())
                            .selected_text("Name");
                        if frame == 0 {
                            egui::Popup::open_id(
                                ui.ctx(),
                                ui.make_persistent_id(egui::IdSalt::new("sort_test"))
                                    .with("popup"),
                            );
                        }
                        combo.show_ui(ui, |ui| {
                            for label in ["Name", "Date", "Size", "Type"] {
                                rows.push(ui.selectable_label(label == "Name", label).rect);
                            }
                        });
                    },
                );
                assert_eq!(rows.len(), 4, "popup must be open");
                pointer = rows[1].right_center() - egui::vec2(2.0, 0.0);
                if frame > 8 {
                    assert!(
                        !has_scrollbar(&output),
                        "unnecessary scrollbar at text={text_scale}, dpi={dpi}"
                    );
                    assert_eq!(rows, previous_rows, "hover must not move menu items");
                }
                previous_rows = rows;
            }
        }
    }
}
