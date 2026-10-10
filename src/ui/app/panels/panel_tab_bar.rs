use crate::app::dual_panel::ActivePanel;
use crate::domain::special_paths::{COMPUTER_VIEW_ID, RECYCLE_BIN_VIEW_ID};
use crate::tabs::PanelTabSet;
use crate::ui::icon_loader::IconLoader;
use crate::ui::svg_icons::SvgIconManager;
use eframe::egui;

const TAB_CORNER_RADIUS: u8 = 7;
const TAB_HEIGHT: f32 = 28.0;
const TAB_MIN_WIDTH: f32 = 92.0;
const TAB_MAX_WIDTH: f32 = 160.0;
const CLOSE_BUTTON_SIZE: f32 = 16.0;

pub(super) enum PanelTabAction {
    Select(ActivePanel, usize),
    Add(ActivePanel),
    Close(ActivePanel, usize),
}

fn layout_tab_title(
    ui: &egui::Ui,
    title: &str,
    font_id: egui::FontId,
    color: egui::Color32,
    max_width: f32,
) -> std::sync::Arc<egui::Galley> {
    let painter = ui.painter();
    let full_galley = painter.layout_no_wrap(title.to_owned(), font_id.clone(), color);
    if full_galley.size().x <= max_width {
        return full_galley;
    }

    let mut boundaries: Vec<usize> = title.char_indices().map(|(index, _)| index).collect();
    boundaries.push(title.len());
    let mut low = 0usize;
    let mut high = boundaries.len().saturating_sub(1);

    while low < high {
        let middle = (low + high).div_ceil(2);
        let mut candidate = title[..boundaries[middle]].to_owned();
        candidate.push('…');
        let galley = painter.layout_no_wrap(candidate, font_id.clone(), color);
        if galley.size().x <= max_width {
            low = middle;
        } else {
            high = middle - 1;
        }
    }

    let mut truncated = title[..boundaries[low]].to_owned();
    truncated.push('…');
    painter.layout_no_wrap(truncated, font_id, color)
}

fn paint_tab_background(ui: &egui::Ui, rect: egui::Rect, color: egui::Color32) {
    ui.painter().rect_filled(
        rect,
        egui::CornerRadius {
            nw: TAB_CORNER_RADIUS,
            ne: TAB_CORNER_RADIUS,
            sw: 0,
            se: 0,
        },
        color,
    );

    if color.a() == u8::MAX {
        let body_top = (rect.min.y + TAB_CORNER_RADIUS as f32 - 1.0).min(rect.max.y);
        let body = egui::Rect::from_min_max(egui::pos2(rect.min.x, body_top), rect.max);
        ui.painter().rect_filled(body, 0.0, color);
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_panel_tab_bar(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    workspace_id: usize,
    panel: ActivePanel,
    tabs: &PanelTabSet,
    titles: &[String],
    live_path: Option<&str>,
    computer_icon: Option<&egui::TextureHandle>,
    svg_icons: &mut SvgIconManager,
    icon_loader: &mut IconLoader,
) -> Option<PanelTabAction> {
    let mut action = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.set_clip_rect(rect);
        egui::ScrollArea::horizontal()
            .id_salt(("panel_tab_strip", workspace_id, panel))
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.set_min_height(rect.height());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let tab_width = ((rect.width() - 25.0) / tabs.tabs.len().max(1) as f32)
                        .clamp(TAB_MIN_WIDTH, TAB_MAX_WIDTH);
                    let dark = ui.visuals().dark_mode;
                    let active_bg = if dark {
                        egui::Color32::from_rgb(45, 45, 45)
                    } else {
                        egui::Color32::from_rgb(243, 243, 243)
                    };
                    let inactive_bg = if dark {
                        egui::Color32::from_rgb(30, 30, 30)
                    } else {
                        egui::Color32::from_rgb(230, 230, 230)
                    };
                    let hover_bg = if dark {
                        egui::Color32::from_rgb(52, 52, 52)
                    } else {
                        egui::Color32::from_rgb(216, 216, 216)
                    };
                    let active_text = if dark {
                        egui::Color32::from_rgb(220, 220, 220)
                    } else {
                        egui::Color32::from_rgb(30, 30, 30)
                    };
                    let inactive_text = if dark {
                        egui::Color32::from_rgb(160, 160, 160)
                    } else {
                        egui::Color32::from_rgb(100, 100, 100)
                    };

                    for (index, tab) in tabs.tabs.iter().enumerate() {
                        ui.push_id((workspace_id, panel, tab.id), |ui| {
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
                                egui::Sense::click(),
                            );
                            if tab_response.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }

                            let fill = if selected {
                                active_bg
                            } else if tab_response.hovered() {
                                hover_bg
                            } else {
                                inactive_bg
                            };
                            paint_tab_background(ui, tab_rect, fill);
                            if tab_response.clicked() {
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
                            let icon_name = if path == COMPUTER_VIEW_ID {
                                "home"
                            } else {
                                "folder"
                            };
                            let icon_color = if selected {
                                [30, 90, 180, 255]
                            } else {
                                [80, 80, 80, 255]
                            };
                            let native_icon = if path == COMPUTER_VIEW_ID {
                                computer_icon.cloned()
                            } else if path == RECYCLE_BIN_VIEW_ID {
                                icon_loader.ensure_recycle_bin_icon(ui.ctx())
                            } else {
                                icon_loader
                                    .get_or_load_registered_folder_icon(ui.ctx(), path)
                                    .or_else(|| {
                                        icon_loader.get_or_load_folder_path_icon(ui.ctx(), path)
                                    })
                            };
                            if let Some(texture) = native_icon {
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
                            let close_response = (tabs.tabs.len() > 1).then(|| {
                                ui.interact(
                                    close_rect,
                                    ui.id().with("panel_tab_close"),
                                    egui::Sense::click(),
                                )
                            });
                            if let Some(response) = close_response.as_ref() {
                                if response.hovered() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                    ui.painter().rect_filled(
                                        close_rect,
                                        egui::CornerRadius::same(4),
                                        hover_bg,
                                    );
                                }
                                if response.clicked() {
                                    action = Some(PanelTabAction::Close(panel, index));
                                }

                                let close_center = close_rect.center();
                                let close_radius = CLOSE_BUTTON_SIZE * 0.25;
                                let close_stroke = egui::Stroke::new(
                                    1.5,
                                    if response.hovered() {
                                        active_text
                                    } else {
                                        inactive_text
                                    },
                                );
                                ui.painter().line_segment(
                                    [
                                        close_center + egui::vec2(-close_radius, -close_radius),
                                        close_center + egui::vec2(close_radius, close_radius),
                                    ],
                                    close_stroke,
                                );
                                ui.painter().line_segment(
                                    [
                                        close_center + egui::vec2(close_radius, -close_radius),
                                        close_center + egui::vec2(-close_radius, close_radius),
                                    ],
                                    close_stroke,
                                );
                            }

                            let title_x = icon_rect.right() + 6.0;
                            let title = titles.get(index).map_or(path, String::as_str);
                            let title_max_width = if close_response.is_some() {
                                close_rect.left() - title_x - 4.0
                            } else {
                                content_rect.right() - title_x
                            };
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

                            if tab_response.hovered()
                                && !close_response.as_ref().is_some_and(egui::Response::hovered)
                            {
                                tab_response.clone().on_hover_text(path);
                            }
                        });
                    }

                    let (add_rect, add_response) =
                        ui.allocate_exact_size(egui::vec2(23.0, TAB_HEIGHT), egui::Sense::click());
                    if add_response.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        ui.painter()
                            .rect_filled(add_rect, egui::CornerRadius::same(4), hover_bg);
                    }
                    ui.painter().text(
                        add_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "+",
                        egui::FontId::proportional(17.0),
                        active_text,
                    );
                    if add_response.clicked() {
                        action = Some(PanelTabAction::Add(panel));
                    }
                    add_response.on_hover_text(rust_i18n::t!("tabs.panel_new"));
                });
            });
    });
    action
}
