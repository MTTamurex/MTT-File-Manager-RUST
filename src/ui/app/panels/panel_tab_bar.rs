use crate::app::dual_panel::ActivePanel;
use crate::domain::special_paths::{COMPUTER_VIEW_ID, RECYCLE_BIN_VIEW_ID};
use crate::tabs::PanelTabSet;
use crate::ui::icon_loader::IconLoader;
use crate::ui::svg_icons::SvgIconManager;
use crate::ui::tab_bar::{should_activate_tab_on_drag_hover, tab_icon_style};
use eframe::egui;

mod appearance;
mod layout;
mod reorder;

use appearance::{layout_tab_title, paint_tab_background};
pub(super) use layout::panel_tab_strip_rect;

const TAB_CORNER_RADIUS: u8 = 7;
const TAB_HEIGHT: f32 = 28.0;
pub(super) const TAB_STRIP_HEIGHT: f32 = 36.0;
const TAB_MIN_WIDTH: f32 = 92.0;
const TAB_MAX_WIDTH: f32 = 160.0;
const CLOSE_BUTTON_SIZE: f32 = 16.0;
const ADD_BUTTON_WIDTH: f32 = 20.0;
const ADD_BUTTON_HEIGHT: f32 = 20.0;
const ADD_BUTTON_GAP: f32 = 2.0;

pub(super) enum PanelTabAction {
    Select(ActivePanel, usize),
    Add(ActivePanel),
    Close(ActivePanel, usize),
    Detach(ActivePanel, usize),
    Context(ActivePanel, usize, crate::tabs::actions::TabAction),
    Move {
        panel: ActivePanel,
        from: usize,
        gap: usize,
    },
}

fn tab_tooltip_text<'a>(path: &'a str, title: &'a str) -> &'a str {
    if crate::domain::special_paths::is_virtual_path(path) {
        title
    } else {
        path
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_panel_tab_bar(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    workspace_id: usize,
    panel: ActivePanel,
    active_panel: ActivePanel,
    tabs: &PanelTabSet,
    titles: &[String],
    tag_definitions: &rustc_hash::FxHashMap<i64, crate::domain::file_tag::FileTag>,
    pinned_folders: &[crate::domain::pinned_folder::PinnedFolder],
    live_path: Option<&str>,
    tree_state: &crate::app::state::sidebar_tree_state::SidebarTreeState,
    is_item_dragging: bool,
    can_add_tab: bool,
    computer_icon: Option<&egui::TextureHandle>,
    svg_icons: &mut SvgIconManager,
    icon_loader: &mut IconLoader,
) -> Option<PanelTabAction> {
    let mut action = None;
    let mut drag = reorder::TabDrag::new(ui.ctx(), workspace_id, panel, is_item_dragging);
    let (scroll_rect, tab_width) = layout::tab_strip_layout(rect, tabs.tabs.len());
    let add_rect = egui::Rect::from_min_size(
        egui::pos2(
            scroll_rect.right() + ADD_BUTTON_GAP,
            rect.top() + (TAB_HEIGHT - ADD_BUTTON_HEIGHT).max(0.0) * 0.5,
        ),
        egui::vec2(ADD_BUTTON_WIDTH, ADD_BUTTON_HEIGHT),
    );
    let active_tab_signature = tabs.active().map(|tab| {
        (
            tab.id,
            tabs.active_tab,
            tabs.tabs.len(),
            scroll_rect.width().to_bits(),
        )
    });
    let scroll_state_id = egui::Id::new(("panel_tab_active_scroll", workspace_id, panel));
    let previous_active_tab_signature = ui
        .ctx()
        .data(|data| data.get_temp::<(usize, usize, usize, u32)>(scroll_state_id));
    let reveal_active_tab = active_tab_signature
        .is_some_and(|signature| previous_active_tab_signature != Some(signature));
    if let Some(signature) = active_tab_signature {
        ui.ctx()
            .data_mut(|data| data.insert_temp(scroll_state_id, signature));
    }
    let dark = ui.visuals().dark_mode;
    let active_bg = if dark {
        crate::ui::theme::palette::surface(ui.ctx(), egui::Color32::from_rgb(45, 45, 45))
    } else {
        egui::Color32::WHITE
    };
    let inactive_bg = if dark {
        crate::ui::theme::palette::background(ui.ctx(), egui::Color32::from_rgb(30, 30, 30))
    } else {
        egui::Color32::from_rgb(230, 230, 230)
    };
    let hover_bg = if dark {
        crate::ui::theme::palette::hover(ui.ctx(), egui::Color32::from_rgb(52, 52, 52))
    } else {
        egui::Color32::from_rgb(216, 216, 216)
    };
    let active_text = if dark {
        crate::ui::theme::text_color(ui.ctx())
    } else {
        egui::Color32::from_rgb(30, 30, 30)
    };
    let inactive_text = if dark {
        crate::ui::theme::secondary_text_color(ui.ctx())
    } else {
        egui::Color32::from_rgb(100, 100, 100)
    };
    let tab_row_rect = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), TAB_HEIGHT));
    ui.painter().rect_filled(tab_row_rect, 0.0, inactive_bg);

    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.set_clip_rect(rect);
        ui.scope_builder(egui::UiBuilder::new().max_rect(scroll_rect), |scroll_ui| {
            scroll_ui.set_clip_rect(scroll_rect);
            egui::ScrollArea::horizontal()
                .id_salt(("panel_tab_strip", workspace_id, panel))
                .scroll_source(
                    egui::scroll_area::ScrollSource::SCROLL_BAR
                        | egui::scroll_area::ScrollSource::MOUSE_WHEEL,
                )
                .auto_shrink([false, false])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                .show(scroll_ui, |scroll_ui| {
                    scroll_ui.set_min_height(TAB_HEIGHT);
                    scroll_ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let mut preview = crate::ui::tab_drag_preview::TabDragPreview::default();
                        for (index, tab) in tabs.tabs.iter().enumerate() {
                            ui.push_id((workspace_id, panel, tab.id), |ui| {
                                let paint_start = crate::ui::tab_drag_preview::TabDragPreview::begin(ui);
                                let path = if index == tabs.active_tab {
                                    live_path.unwrap_or(&tab.snapshot.path)
                                } else {
                                    &tab.snapshot.path
                                };
                                let selected = index == tabs.active_tab;
                                let text_color = if selected { active_text } else { inactive_text };
                                let (tab_rect, _) = ui.allocate_exact_size(
                                    egui::vec2(tab_width, TAB_HEIGHT),
                                    egui::Sense::hover(),
                                );
                                let tab_response = ui.interact(
                                    tab_rect,
                                    ui.id().with("panel_tab_select"),
                                    egui::Sense::click_and_drag(),
                                );
                                let active_panel_tab = selected && panel == active_panel;
                                if tab_response.middle_clicked() && !tab.pinned {
                                    action = Some(PanelTabAction::Close(panel, index));
                                }
                                let drag_hovering = is_item_dragging
                                    && !active_panel_tab
                                    && tab_response.contains_pointer();
                                let activate_on_drag = should_activate_tab_on_drag_hover(
                                    ui,
                                    ui.id().with("panel_tab_drag_dwell"),
                                    is_item_dragging,
                                    active_panel_tab,
                                    tab_response.contains_pointer(),
                                );
                                if tab_response.hovered() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                }

                                let fill = if drag_hovering {
                                    egui::Color32::from_rgba_unmultiplied(60, 130, 220, 90)
                                } else if selected {
                                    active_bg
                                } else if tab_response.hovered() {
                                    hover_bg
                                } else {
                                    inactive_bg
                                };
                                paint_tab_background(ui, tab_rect, fill);
                                if tab_response.clicked() || activate_on_drag {
                                    action = Some(PanelTabAction::Select(panel, index));
                                }

                                let content_rect = tab_rect.shrink2(egui::vec2(8.0, 4.0));
                                let icon_size = 16.0;
                                let icon_rect = egui::Rect::from_min_size(
                                    egui::pos2(
                                        content_rect.min.x,
                                        content_rect.center().y - icon_size * 0.5,
                                    ),
                                    egui::Vec2::splat(icon_size),
                                );
                                let (icon_name, icon_color) =
                                    tab_icon_style(path, selected, tag_definitions);
                                let native_icon =
                                    if crate::domain::special_paths::is_tag_view_path(path) {
                                        None
                                    } else if path == COMPUTER_VIEW_ID {
                                        computer_icon.cloned()
                                    } else if path == RECYCLE_BIN_VIEW_ID {
                                        icon_loader.ensure_recycle_bin_icon(ui.ctx())
                                    } else {
                                        icon_loader
                                            .get_or_load_folder_navigation_icon(ui.ctx(), path)
                                    };
                                if icon_name == "tag" {
                                    crate::ui::tag_icon::paint_filled(
                                        ui.painter(),
                                        icon_rect.center(),
                                        icon_size,
                                        egui::Color32::from_rgba_unmultiplied(
                                            icon_color[0],
                                            icon_color[1],
                                            icon_color[2],
                                            icon_color[3],
                                        ),
                                    );
                                } else if let Some(texture) = native_icon {
                                    ui.painter().image(
                                        texture.id(),
                                        icon_rect,
                                        egui::Rect::from_min_max(
                                            egui::pos2(0.0, 0.0),
                                            egui::pos2(1.0, 1.0),
                                        ),
                                        egui::Color32::WHITE,
                                    );
                                } else if let Some(texture) =
                                    svg_icons.get_icon(ui.ctx(), icon_name, 32, icon_color)
                                {
                                    ui.painter().image(
                                        texture.id(),
                                        icon_rect,
                                        egui::Rect::from_min_max(
                                            egui::pos2(0.0, 0.0),
                                            egui::pos2(1.0, 1.0),
                                        ),
                                        egui::Color32::WHITE,
                                    );
                                }

                                let close_rect = egui::Rect::from_min_size(
                                    egui::pos2(
                                        tab_rect.right() - CLOSE_BUTTON_SIZE - 8.0,
                                        content_rect.center().y - CLOSE_BUTTON_SIZE * 0.5,
                                    ),
                                    egui::Vec2::splat(CLOSE_BUTTON_SIZE),
                                );
                                if drag.record_tab(ui, tab.id, &tab_response, close_rect)
                                    && !active_panel_tab
                                {
                                    action = Some(PanelTabAction::Select(panel, index));
                                }
                                let close_response =
                                    crate::ui::tab_bar::context_menu::close_control(
                                        ui,
                                        close_rect,
                                        ui.id().with("panel_tab_close"),
                                        tab.pinned,
                                        hover_bg,
                                        active_text,
                                        inactive_text,
                                    );
                                if close_response.clicked() {
                                    action = Some(if tab.pinned {
                                        PanelTabAction::Context(
                                            panel,
                                            index,
                                            crate::tabs::TabAction::TogglePin,
                                        )
                                    } else {
                                        PanelTabAction::Close(panel, index)
                                    });
                                }

                                let title_x = icon_rect.right() + 6.0;
                                let title = titles.get(index).map_or(path, String::as_str);
                                let title_max_width = close_rect.left() - title_x - 4.0;
                                let galley = layout_tab_title(
                                    ui,
                                    title,
                                    egui::FontId::proportional(13.0),
                                    text_color,
                                    title_max_width.max(0.0),
                                );
                                let title_pos = egui::pos2(
                                    title_x,
                                    content_rect.center().y - galley.size().y * 0.5,
                                );
                                ui.painter().galley(title_pos, galley, text_color);

                                if selected && reveal_active_tab && !drag.active() {
                                    ui.scroll_to_rect(tab_rect, Some(egui::Align::Center));
                                }

                                if tab_response.hovered()
                                    && !drag.active()
                                    && !close_response.hovered()
                                {
                                    tab_response
                                        .clone()
                                        .on_hover_text(tab_tooltip_text(path, title));
                                }
                                if !is_item_dragging {
                                    use crate::tabs::actions::{close_candidates, TabAction};
                                    let context_response = if tab.pinned {
                                        tab_response.union(close_response)
                                    } else {
                                        tab_response
                                    };
                                    if let Some(command) =
                                        crate::ui::tab_bar::context_menu::tab_context_menu(
                                            &context_response,
                                            tab.pinned,
                                            can_add_tab,
                                            !close_candidates(
                                                &tabs.tabs,
                                                index,
                                                TabAction::CloseLeft,
                                                |t| t.pinned,
                                            )
                                            .is_empty(),
                                            !close_candidates(
                                                &tabs.tabs,
                                                index,
                                                TabAction::CloseRight,
                                                |t| t.pinned,
                                            )
                                            .is_empty(),
                                            crate::tabs::quick_access::tab_quick_access_action(
                                                path,
                                                pinned_folders,
                                            ),
                                            tree_state.tab_tree_action(
                                                crate::app::state::sidebar_tree_state::reveal::RevealOwner::panel(workspace_id, tab.id),
                                                path,
                                            ),
                                            None,
                                        )
                                    {
                                        action =
                                            Some(PanelTabAction::Context(panel, index, command));
                                    }
                                }
                                preview.record(ui, tab_rect, paint_start);
                            });
                        }
                        preview.paint(ui, ui.id().with(("tab_preview", workspace_id, panel)), drag.preview_source(ui), scroll_rect);
                        if let Some(index) = drag.detach_source(ui) {
                            action = Some(PanelTabAction::Detach(panel, index));
                        } else if let Some((from, gap)) = drag.finish(ui, scroll_rect) {
                            action = Some(PanelTabAction::Move { panel, from, gap });
                        }
                    });
                });
        });

        let add_response = ui.interact(
            add_rect,
            ui.id().with(("panel_tab_add", workspace_id, panel)),
            egui::Sense::click(),
        );
        if add_response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            ui.painter()
                .rect_filled(add_rect, egui::CornerRadius::same(4), hover_bg);
        }
        ui.painter().text(
            egui::pos2(add_rect.center().x, add_rect.center().y - 1.0),
            egui::Align2::CENTER_CENTER,
            "+",
            egui::FontId::proportional(17.0),
            ui.visuals().text_color(),
        );
        if add_response.clicked() {
            action = Some(PanelTabAction::Add(panel));
        }
        add_response.on_hover_text(rust_i18n::t!("tabs.panel_new"));
    });
    action
}

#[cfg(test)]
mod tests;
