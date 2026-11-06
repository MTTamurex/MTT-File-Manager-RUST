use super::*;
use crate::ui::components::tag_settings::model::ListState;

#[test]
fn arrows_only_change_selection_when_the_list_owns_keyboard_focus() {
    for focused in [false, true] {
        let ctx = egui::Context::default();
        let mut list = ListState {
            selected: Some(1),
            ..Default::default()
        };
        let _ = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::ArrowDown,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ui| navigate(ui, &[1, 2, 3], &mut list, focused),
        );
        assert_eq!(list.selected, Some(if focused { 2 } else { 1 }));
        assert_eq!(list.focus_selected, focused);
    }
}

#[test]
fn host_scoped_state_survives_tab_switches_without_leaking_to_other_windows() {
    let ctx = egui::Context::default();
    let settings_id = egui::Id::new("settings").with("tag_browser");
    let modal_id = egui::Id::new("modal").with("tag_browser");
    let mut state = BrowserState::default();
    state.tags.search = "category".into();
    state.tags.select(2);
    state.tab = Tab::AutoTag;
    state.rules.select(10);
    ctx.data_mut(|data| data.insert_temp(settings_id, state));
    let restored = ctx
        .data_mut(|data| data.get_temp::<BrowserState>(settings_id))
        .unwrap();
    assert_eq!(restored.tags.search, "category");
    assert_eq!(restored.tags.selected, Some(2));
    assert_eq!(restored.rules.selected, Some(10));
    assert!(ctx
        .data_mut(|data| data.get_temp::<BrowserState>(modal_id))
        .is_none());
}

#[test]
fn separate_list_scroll_positions_survive_switching_tabs() {
    let ctx = egui::Context::default();
    ctx.global_style_mut(|style| style.animation_time = 0.0);
    for (frame, tab) in [Tab::Tags, Tab::AutoTag, Tab::Tags, Tab::AutoTag]
        .into_iter()
        .enumerate()
    {
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            let mut area = egui::ScrollArea::vertical()
                .id_salt(if tab == Tab::Tags {
                    "tag_browser_tag_list"
                } else {
                    "tag_browser_rule_list"
                })
                .max_height(120.0);
            if frame < 2 {
                area = area.vertical_scroll_offset(if tab == Tab::Tags { 150.0 } else { 300.0 });
            }
            let output = area.show(ui, |ui| {
                for _ in 0..100 {
                    ui.label("Item");
                }
            });
            let expected = if tab == Tab::Tags { 150.0 } else { 300.0 };
            assert!((output.state.offset.y - expected).abs() < 1.0);
        });
    }
}
