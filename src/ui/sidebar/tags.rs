use super::{SidebarAction, SidebarContext};
use eframe::egui::{self, Color32, Pos2, Rect, Sense};
use rust_i18n::t;
use rustc_hash::FxHashMap;

const TAG_HEADER_H: f32 = 22.0;
const TAG_ROW_H: f32 = 26.0;
const TAG_BOTTOM_PADDING_H: f32 = 8.0;
const AUTO_SCROLL_MARGIN: f32 = 30.0;
const AUTO_SCROLL_SPEED: f32 = 8.0;
/// Horizontal indentation applied per hierarchy level.
const TAG_LEVEL_INDENT: f32 = 14.0;

/// Shared state threaded through the recursive tree walk.
struct TagTreeState<'a> {
    drag_id: egui::Id,
    drag_source: Option<i64>,
    drag_source_parent: Option<i64>,
    pointer_pos: Option<Pos2>,
    clip_rect: Rect,
    children: &'a FxHashMap<i64, Vec<i64>>,
    drop_target: Option<Option<i64>>,
    action: &'a mut Option<SidebarAction>,
}

/// Builds the sidebar tree from the user's flat drag order: roots in flat
/// order, each parent's children in flat order too (see `reorder_sidebar_tag`).
fn build_sidebar_tag_tree(
    sidebar_tag_ids: &[i64],
    tag_definitions: &FxHashMap<i64, crate::domain::file_tag::FileTag>,
) -> (Vec<i64>, FxHashMap<i64, Vec<i64>>) {
    let mut roots: Vec<i64> = Vec::new();
    let mut children: FxHashMap<i64, Vec<i64>> = FxHashMap::default();
    for id in sidebar_tag_ids.iter().copied() {
        let Some(tag) = tag_definitions.get(&id) else {
            continue;
        };
        match tag
            .parent_id
            .filter(|parent| tag_definitions.contains_key(parent))
        {
            Some(parent) => children.entry(parent).or_default().push(id),
            None => roots.push(id),
        }
    }
    (roots, children)
}

/// Renders Tags content inside the active sidebar layout.
pub fn render_tags_section(
    ui: &mut egui::Ui,
    ctx: &mut SidebarContext,
    action: &mut Option<SidebarAction>,
) {
    if !ctx.show_tags || ctx.tag_definitions.is_empty() {
        return;
    }

    let drag_id = egui::Id::new("sidebar_tag_reorder_drag");
    let drag_source: Option<i64> = ui.ctx().data(|data| data.get_temp(drag_id));
    let pointer_pos = ui.ctx().input(|input| {
        input
            .pointer
            .hover_pos()
            .or_else(|| input.pointer.interact_pos())
    });
    let (primary_released, cancel_drag) = ui.ctx().input(|input| {
        let primary_released = input.pointer.primary_released();
        let cancel_drag = input.key_pressed(egui::Key::Escape)
            || !input.viewport().focused.unwrap_or(true)
            || (!input.pointer.primary_down() && !primary_released);
        (primary_released, cancel_drag)
    });
    let clip_rect = ui.clip_rect();

    let (header_rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), TAG_HEADER_H),
        Sense::hover(),
    );
    if let Some(command) = super::section_menu::header_menu(
        ui,
        header_rect,
        "tags",
        ctx.is_renaming || ctx.is_item_dragging,
    ) {
        *action = Some(command);
    }
    let toggle_size = 18.0;
    let toggle_rect = Rect::from_center_size(
        Pos2::new(
            header_rect.max.x - toggle_size / 2.0 - 3.0,
            header_rect.center().y,
        ),
        egui::vec2(toggle_size, toggle_size),
    );
    let toggle_response = ui
        .interact(
            toggle_rect,
            egui::Id::new("sidebar_toggle_tags"),
            Sense::click(),
        )
        .on_hover_text(if ctx.collapse_tags {
            t!("sidebar.expand_section")
        } else {
            t!("sidebar.collapse_section")
        });

    if ui.is_rect_visible(header_rect) {
        ui.painter().text(
            Pos2::new(header_rect.min.x + 8.0, header_rect.center().y),
            egui::Align2::LEFT_CENTER,
            t!("tags.section"),
            egui::FontId::proportional(10.0),
            Color32::from_gray(120),
        );

        let toggle_icon = if ctx.collapse_tags { "v" } else { "^" };
        let toggle_color = if toggle_response.hovered() {
            ui.visuals().text_color()
        } else {
            Color32::from_gray(140)
        };
        ui.painter().text(
            toggle_rect.center(),
            egui::Align2::CENTER_CENTER,
            toggle_icon,
            egui::FontId::proportional(14.0),
            toggle_color,
        );
    }

    if toggle_response.clicked() && action.is_none() {
        *action = Some(SidebarAction::ToggleTags);
    }

    if ctx.collapse_tags {
        ui.add_space(TAG_BOTTOM_PADDING_H);
        return;
    }

    ui.add_space(4.0);

    let (roots, children) = build_sidebar_tag_tree(ctx.sidebar_tag_ids, ctx.tag_definitions);
    let drag_source_parent = drag_source.and_then(|id| {
        ctx.tag_definitions
            .get(&id)
            .and_then(|tag| tag.parent_id)
            .filter(|parent| ctx.tag_definitions.contains_key(parent))
    });

    let mut state = TagTreeState {
        drag_id,
        drag_source,
        drag_source_parent,
        pointer_pos,
        clip_rect,
        children: &children,
        drop_target: None,
        action,
    };
    render_tag_level(ui, ctx, &mut state, &roots, 0);

    if let Some(source) = drag_source {
        if cancel_drag {
            ui.ctx().data_mut(|data| data.remove::<i64>(drag_id));
        } else {
            auto_scroll_during_drag(ui, pointer_pos, clip_rect);

            if primary_released {
                ui.ctx().data_mut(|data| data.remove::<i64>(drag_id));
                if let Some(before_tag_id) = state.drop_target {
                    if state.action.is_none() {
                        *state.action = Some(SidebarAction::ReorderTag {
                            tag_id: source,
                            before_tag_id,
                        });
                    }
                }
            }
        }
    }

    ui.add_space(TAG_BOTTOM_PADDING_H);
}

fn render_tag_level(
    ui: &mut egui::Ui,
    ctx: &mut SidebarContext,
    state: &mut TagTreeState,
    ids: &[i64],
    depth: usize,
) {
    for (index, tag_id) in ids.iter().copied().enumerate() {
        let Some(tag) = ctx.tag_definitions.get(&tag_id) else {
            continue;
        };
        let child_ids = state.children.get(&tag_id);
        let has_children = child_ids.is_some_and(|children| !children.is_empty());
        let is_selected = ctx.active_tag_filter == Some(tag.id);
        let own_count = ctx.tag_counts.get(&tag.id).copied().unwrap_or(0);
        let count = if has_children {
            ctx.subtree_tag_counts
                .get(&tag.id)
                .copied()
                .unwrap_or(own_count)
        } else {
            own_count
        };
        let (mut rect, mut response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), TAG_ROW_H),
            Sense::click_and_drag(),
        );
        rect.min.x = state.clip_rect.min.x;
        rect.max.x = state.clip_rect.max.x;

        // Start internal drag for reordering (anywhere on the row, like Quick Access)
        if response.drag_started_by(egui::PointerButton::Primary) {
            ui.ctx()
                .data_mut(|data| data.insert_temp(state.drag_id, tag.id));
        }

        let mut indicator_y = None;
        if state.drag_source.is_some() {
            if let Some(pointer) = state.pointer_pos.filter(|pointer| rect.contains(*pointer)) {
                // Drops only reorder siblings: ignore rows of other levels.
                if tag.parent_id == state.drag_source_parent {
                    if pointer.y < rect.center().y {
                        state.drop_target = Some(Some(tag.id));
                        indicator_y = Some(rect.top());
                    } else {
                        // Bottom half = after this tag = before its next
                        // sibling (`None` = end of the sibling chain).
                        state.drop_target = Some(ids.get(index + 1).copied());
                        indicator_y = Some(rect.bottom());
                    }
                }
            }
        }

        let text_left = rect.min.x + 30.0 + depth as f32 * TAG_LEVEL_INDENT;
        let chevron_rect = Rect::from_center_size(
            Pos2::new(text_left - 24.0, rect.center().y),
            egui::vec2(14.0, TAG_ROW_H),
        );

        if ui.is_rect_visible(rect) {
            if state.drag_source == Some(tag.id) {
                ui.painter().rect_filled(
                    rect,
                    0.0,
                    Color32::from_rgba_premultiplied(100, 120, 215, 60),
                );
            } else if is_selected {
                ui.painter()
                    .rect_filled(rect, 0.0, crate::ui::theme::selection_color(ui.ctx()));
            } else if (response.hovered() || response.dragged()) && !ctx.is_item_dragging {
                ui.painter().rect_filled(
                    rect,
                    0.0,
                    crate::ui::theme::selection_hover_color(ui.ctx()),
                );
            }

            let text_color = if is_selected {
                crate::ui::theme::selection_text_color(ui.ctx())
            } else {
                ui.visuals().text_color()
            };

            if has_children {
                let is_collapsed = ctx.sidebar_tags_collapsed.contains(&tag.id);
                let chevron_color = if chevron_rect.contains(state.pointer_pos.unwrap_or_default())
                {
                    ui.visuals().text_color()
                } else {
                    Color32::from_gray(140)
                };
                ui.painter().text(
                    chevron_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    if is_collapsed { "v" } else { "^" },
                    egui::FontId::proportional(11.0),
                    chevron_color,
                );
            }

            let tag_icon_center = Pos2::new(text_left - 12.0, rect.center().y);
            crate::ui::tag_icon::paint_filled(
                ui.painter(),
                tag_icon_center,
                10.0,
                tag.color.to_color32(),
            );
            ui.painter().text(
                Pos2::new(text_left, rect.center().y),
                egui::Align2::LEFT_CENTER,
                &tag.name,
                egui::FontId::proportional(11.5),
                text_color,
            );

            if count > 0 {
                ui.painter().text(
                    Pos2::new(rect.max.x - 12.0, rect.center().y),
                    egui::Align2::RIGHT_CENTER,
                    count.to_string(),
                    egui::FontId::proportional(10.0),
                    Color32::from_gray(130),
                );
            }

            if let Some(y) = indicator_y {
                ui.painter().hline(
                    rect.left()..=rect.right(),
                    y,
                    egui::Stroke::new(2.0, Color32::from_rgb(0, 120, 215)),
                );
            }
        }

        if has_children {
            let hint = if ctx.sidebar_tags_collapsed.contains(&tag.id) {
                t!("tags.expand_children")
            } else {
                t!("tags.collapse_children")
            };
            response = response.on_hover_text(format!(
                "{}\n{}",
                t!("tags.subtree_count_tooltip", own = own_count, total = count),
                hint
            ));
        }

        if response.middle_clicked()
            && !ctx.is_renaming
            && !ctx.is_item_dragging
            && state.action.is_none()
        {
            *state.action = Some(SidebarAction::OpenInNewTab(
                crate::domain::special_paths::tag_view_path(tag.id),
            ));
        } else if response.clicked() && !ctx.is_renaming && state.action.is_none() {
            let clicked_chevron = has_children
                && response
                    .interact_pointer_pos()
                    .is_some_and(|pointer| chevron_rect.contains(pointer));
            if clicked_chevron {
                *state.action = Some(SidebarAction::ToggleTagCollapse(tag.id));
            } else {
                let next = if is_selected { None } else { Some(tag.id) };
                *state.action = Some(SidebarAction::FilterByTag(next));
            }
        }

        if has_children && !ctx.sidebar_tags_collapsed.contains(&tag.id) {
            if let Some(child_ids) = child_ids {
                render_tag_level(ui, ctx, state, child_ids, depth + 1);
            }
        }
    }
}

fn auto_scroll_during_drag(ui: &mut egui::Ui, pointer_pos: Option<Pos2>, viewport: Rect) {
    let Some(pointer) = pointer_pos else {
        return;
    };
    let delta = auto_scroll_delta(pointer, viewport);
    if delta == 0.0 {
        return;
    }

    let target_y = if delta < 0.0 {
        viewport.top() + delta
    } else {
        viewport.bottom() + delta
    };
    let target = Pos2::new(viewport.left(), target_y);
    let target_rect = Rect::from_min_max(target, target);
    ui.scroll_to_rect(target_rect, None);
    ui.ctx().request_repaint();
}

fn auto_scroll_delta(pointer: Pos2, viewport: Rect) -> f32 {
    if pointer.x < viewport.left() || pointer.x > viewport.right() {
        return 0.0;
    }

    if pointer.y < viewport.top() + AUTO_SCROLL_MARGIN {
        -((viewport.top() + AUTO_SCROLL_MARGIN - pointer.y).max(AUTO_SCROLL_SPEED))
    } else if pointer.y > viewport.bottom() - AUTO_SCROLL_MARGIN {
        (pointer.y - (viewport.bottom() - AUTO_SCROLL_MARGIN)).max(AUTO_SCROLL_SPEED)
    } else {
        0.0
    }
}

/// Flattens the sidebar tree in visible (pre-order) sequence, honoring the
/// user's sibling order and collapsed state.
#[cfg(test)]
fn visible_tag_sequence(
    sidebar_tag_ids: &[i64],
    tag_definitions: &FxHashMap<i64, crate::domain::file_tag::FileTag>,
    collapsed: &rustc_hash::FxHashSet<i64>,
) -> Vec<(i64, usize)> {
    fn walk(
        children: &FxHashMap<i64, Vec<i64>>,
        collapsed: &rustc_hash::FxHashSet<i64>,
        id: i64,
        depth: usize,
        out: &mut Vec<(i64, usize)>,
    ) {
        if collapsed.contains(&id) {
            return;
        }
        if let Some(child_ids) = children.get(&id) {
            for child in child_ids {
                out.push((*child, depth + 1));
                walk(children, collapsed, *child, depth + 1, out);
            }
        }
    }

    let (roots, children) = build_sidebar_tag_tree(sidebar_tag_ids, tag_definitions);
    let mut out = Vec::new();
    for root in roots {
        out.push((root, 0));
        walk(&children, collapsed, root, 0, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::auto_scroll_delta;
    use eframe::egui::{pos2, Rect};

    #[test]
    fn auto_scrolls_near_vertical_edges_only_inside_width() {
        let viewport = Rect::from_min_max(pos2(10.0, 20.0), pos2(110.0, 220.0));

        assert!(auto_scroll_delta(pos2(50.0, 25.0), viewport) < 0.0);
        assert!(auto_scroll_delta(pos2(50.0, 215.0), viewport) > 0.0);
        assert_eq!(auto_scroll_delta(pos2(50.0, 120.0), viewport), 0.0);
        assert_eq!(auto_scroll_delta(pos2(5.0, 25.0), viewport), 0.0);
    }

    #[test]
    fn visible_sequence_nests_children_and_honors_collapse() {
        use super::visible_tag_sequence;
        use crate::domain::file_tag::{FileTag, TagColor};
        use rustc_hash::{FxHashMap, FxHashSet};

        let mut defs: FxHashMap<i64, FileTag> = FxHashMap::default();
        let make = |id: i64, name: &str, position: i64, parent: Option<i64>| FileTag {
            id,
            name: name.to_string(),
            color: TagColor::Blue,
            position,
            parent_id: parent,
        };
        defs.insert(1, make(1, "Fotos", 0, None));
        defs.insert(2, make(2, "Músicas", 1, None));
        defs.insert(3, make(3, "Férias 2025", 0, Some(1)));
        defs.insert(4, make(4, "Passeio 2018", 1, Some(1)));
        defs.insert(5, make(5, "Álbuns", 0, Some(3)));

        // Flat order: Fotos, Férias 2025, Passeio 2018, Músicas, Álbuns —
        // "Álbuns" (child of "Férias 2025") sits at the end, as a
        // reconciliation would have appended it.
        let order = vec![1, 3, 4, 2, 5];
        let sequence = visible_tag_sequence(&order, &defs, &FxHashSet::default());
        assert_eq!(sequence, vec![(1, 0), (3, 1), (5, 2), (4, 1), (2, 0)]);

        let mut collapsed = FxHashSet::default();
        collapsed.insert(1);
        let sequence = visible_tag_sequence(&order, &defs, &collapsed);
        assert_eq!(sequence, vec![(1, 0), (2, 0)]);
    }
}
