use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use eframe::egui;

use super::{render_miller_column, MillerColumnAction, MillerColumnContext, MillerColumnOutput};
use crate::application::grouping::{build_group_projection, GroupKey, NameGroup};
use crate::domain::file_entry::{FileEntry, GroupMode};
use crate::ui::cache::FxHashSet;
use crate::ui::icon_loader::{IconFailureTracker, IconLoadTracker, IconLoader};
use crate::ui::views::rectangle_selection::RectangleSelectionMetrics;

fn frame(
    ctx: &egui::Context,
    mode: GroupMode,
    collapsed: &FxHashSet<GroupKey>,
    events: Vec<egui::Event>,
) -> (egui::FullOutput, MillerColumnOutput) {
    let items: Vec<_> = ["zeta", "alpha"]
        .iter()
        .map(|name| FileEntry {
            path: PathBuf::from(name),
            name: name.to_string(),
            is_dir: true,
            size: 0,
            modified: 0,
            created: None,
            folder_cover: None,
            drive_info: None,
            sync_status: Default::default(),
            is_hidden: false,
            recycle_bin: None,
        })
        .collect();
    let projection = build_group_projection(&items, mode, false);
    let mut icon_loader = IconLoader::new();
    let mut icon_requests = Vec::new();
    let mut drop_target = None;
    let loading_icons = IconLoadTracker::default();
    let failed_icons = IconFailureTracker::new(NonZeroUsize::new(8).unwrap());
    let selected = FxHashSet::default();
    let mut output = None;
    let full_output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(240.0, 300.0),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            output = Some(render_miller_column(
                ui,
                "column",
                &mut MillerColumnContext {
                    items: &items,
                    group_projection: &projection,
                    collapsed_groups: collapsed,
                    directory: Path::new("folder"),
                    selected_child: None,
                    selected_file: None,
                    multi_selection: &selected,
                    renaming_state: None,
                    focus_rename: false,
                    rectangle_selection_state: None,
                    icon_loader: &mut icon_loader,
                    folder_icon: None,
                    loading_icons: &loading_icons,
                    failed_icons: &failed_icons,
                    icon_requests: &mut icon_requests,
                    is_item_dragging: false,
                    drop_target: &mut drop_target,
                    external_drop_pointer_pos: None,
                    is_loading: false,
                    scroll_interpolation_enabled: false,
                },
            ));
        },
    );
    (full_output, output.unwrap())
}

fn text_y(output: &egui::FullOutput, text: &str) -> Option<f32> {
    output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Text(shape) if shape.galley.job.text == text => Some(shape.pos.y),
        _ => None,
    })
}

#[test]
fn ancestor_column_renders_groups_in_visual_order_and_hides_collapsed_items() {
    let ctx = egui::Context::default();
    let (paint, output) = frame(&ctx, GroupMode::Name, &FxHashSet::default(), vec![]);
    assert!(text_y(&paint, "alpha").unwrap() < text_y(&paint, "zeta").unwrap());
    let Some(RectangleSelectionMetrics::Grouped(metrics)) =
        output.rectangle_selection_frame.metrics
    else {
        panic!("expected grouped selection geometry");
    };
    assert_eq!(metrics.visual_order().collect::<Vec<_>>(), vec![1, 0]);
    let collapsed = FxHashSet::from_iter([GroupKey::Name(NameGroup::AToH)]);
    let (paint, output) = frame(&ctx, GroupMode::Name, &collapsed, vec![]);
    assert!(text_y(&paint, "alpha").is_none());
    assert!(text_y(&paint, "zeta").is_some());
    let Some(RectangleSelectionMetrics::Grouped(metrics)) =
        output.rectangle_selection_frame.metrics
    else {
        panic!("expected grouped selection geometry");
    };
    assert_eq!(metrics.visual_order().collect::<Vec<_>>(), vec![0]);
}

#[test]
fn ancestor_column_supports_every_group_mode_and_ungrouped_fallback() {
    for mode in [
        GroupMode::None,
        GroupMode::Name,
        GroupMode::Date,
        GroupMode::Type,
        GroupMode::Size,
    ] {
        let ctx = egui::Context::default();
        let (_, output) = frame(&ctx, mode, &FxHashSet::default(), vec![]);
        assert_eq!(
            matches!(
                output.rectangle_selection_frame.metrics,
                Some(RectangleSelectionMetrics::Grouped(_))
            ),
            mode != GroupMode::None
        );
    }
}

#[test]
fn clicking_group_header_toggles_without_selecting_or_navigating() {
    let ctx = egui::Context::default();
    let collapsed = FxHashSet::default();
    let (_, output) = frame(&ctx, GroupMode::Name, &collapsed, vec![]);
    let viewport = output.rectangle_selection_frame.viewport_rect.unwrap();
    let position = viewport.min + egui::vec2(40.0, 15.0);
    for pressed in [true, false] {
        let (_, output) = frame(
            &ctx,
            GroupMode::Name,
            &collapsed,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        if !pressed {
            assert_eq!(output.toggled_group, Some(GroupKey::Name(NameGroup::AToH)));
            assert!(output.action.is_none());
        }
    }
}

#[test]
fn reordered_rows_keep_the_original_listing_index_for_actions() {
    let ctx = egui::Context::default();
    let collapsed = FxHashSet::default();
    let (_, output) = frame(&ctx, GroupMode::Name, &collapsed, vec![]);
    let viewport = output.rectangle_selection_frame.viewport_rect.unwrap();
    let Some(RectangleSelectionMetrics::Grouped(metrics)) =
        output.rectangle_selection_frame.metrics
    else {
        panic!("expected grouped selection geometry");
    };
    let position = viewport.min + metrics.sections[0].origin.to_vec2() + egui::vec2(40.0, 10.0);
    for pressed in [true, false] {
        let (_, output) = frame(
            &ctx,
            GroupMode::Name,
            &collapsed,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        if !pressed {
            assert!(matches!(
                output.action,
                Some(MillerColumnAction::Clicked(1))
            ));
            assert!(output.toggled_group.is_none());
        }
    }
}
