use crate::application::navigation::NavigationHistory;
use crate::domain::file_entry::{SortMode, ViewMode};
use crate::domain::special_paths::{COMPUTER_VIEW_ID, RECYCLE_BIN_VIEW_ID};
use crate::ui::svg_icons::SvgIconManager;
use crate::ui::theme;
use crate::ui::widgets;
use eframe::egui;
use rust_i18n::t;
use std::cell::RefCell;
use std::path::Component;

// M-3: Cache breadcrumb segments — recomputed only when current_path changes.
// Each entry is (display_label, navigation_target).
thread_local! {
    static BREADCRUMB_CACHE: RefCell<(String, Vec<(String, String)>)> =
        RefCell::new((String::new(), Vec::new()));
}

// Returns the pre-split breadcrumb segments for `current_path`.
// On a cache hit (same path) the cached Vec is cloned; on a miss segments are
// recomputed from Path::components() and the cache is updated.
fn breadcrumb_segments(current_path: &str) -> Vec<(String, String)> {
    BREADCRUMB_CACHE.with(|cache| {
        let mut c = cache.borrow_mut();
        // Include locale in cache key so breadcrumbs refresh on language change
        let current_locale = rust_i18n::locale().to_string();
        let cache_key = format!("{}|{}", current_path, current_locale);
        if c.0 == cache_key {
            return c.1.clone();
        }
        // cache miss — recompute
        let mut full = std::path::PathBuf::new();
        let path = std::path::Path::new(current_path);
        let components: Vec<_> = path.components().collect();
        let mut segs = Vec::with_capacity(components.len());
        for (i, comp) in components.iter().enumerate() {
            full.push(comp.as_os_str());

            if matches!(comp, Component::RootDir) {
                continue;
            }

            let comp_str = comp.as_os_str().to_string_lossy();
            let display_name = comp_str.trim_end_matches('\\');
            if display_name.is_empty() && i > 0 {
                continue;
            }
            let target = {
                let mut p = full.to_string_lossy().into_owned();
                if p.len() == 2 && p.ends_with(':') {
                    p.push('\\');
                }
                p
            };
            // Use translated name for known special folders (Desktop, Documents, etc.)
            let display = crate::infrastructure::onedrive::special_folder_display_name(&full)
                .unwrap_or_else(|| {
                    if display_name.is_empty() {
                        comp_str.to_string()
                    } else {
                        display_name.to_string()
                    }
                });
            segs.push((display, target));
        }
        c.0 = cache_key;
        c.1 = segs.clone();
        segs
    })
}

/// Measure the width of a text string using the given font.
fn measure_text_width(ui: &egui::Ui, text: &str, font_id: &egui::FontId) -> f32 {
    ui.fonts(|fonts| {
        fonts
            .layout_no_wrap(text.to_string(), font_id.clone(), ui.visuals().text_color())
            .size()
            .x
    })
}

/// Render a single breadcrumb button with transparent background and hover effect.
fn render_breadcrumb_button(
    ui: &mut egui::Ui,
    display: &str,
    target_path: String,
    action: &mut Option<ToolbarAction>,
) {
    let btn_resp = ui
        .scope(|ui| {
            let hover_color = if ui.visuals().dark_mode {
                theme::color_dark_hover()
            } else {
                theme::color_hover()
            };

            ui.visuals_mut().widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
            ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
            ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;

            ui.visuals_mut().widgets.hovered.bg_fill = hover_color;
            ui.visuals_mut().widgets.hovered.weak_bg_fill = hover_color;
            ui.visuals_mut().widgets.hovered.bg_stroke = egui::Stroke::NONE;

            ui.visuals_mut().widgets.active.bg_fill = if ui.visuals().dark_mode {
                egui::Color32::from_gray(70)
            } else {
                egui::Color32::from_gray(210)
            };
            ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::NONE;

            ui.button(
                egui::RichText::new(display).color(theme::text_color(ui.visuals().dark_mode)),
            )
        })
        .inner;

    if btn_resp.clicked() {
        *action = Some(ToolbarAction::Navigate(target_path));
    }
}

#[derive(Debug, Clone)]
pub enum ToolbarAction {
    Navigate(String),
    GoBack,
    GoForward,
    GoUp,
    Refresh,
    CreateFolder,
    NavigateToComputer,
    NavigateToRecycleBin,
    ToggleViewMode,
    TogglePreviewPanel,
    ChangeSortMode(SortMode),
    ToggleSortDescending,
    ChangeZoom(f32),
    Search(String),
    OpenSettings,
    StartAddressEdit,
    StartAddressEditWithHistory,
    UpdatePathInput(String),
    CommitPathInput(String),
    CancelPathInput,
    SelectAddressHistoryPath(String),
}

#[allow(clippy::too_many_arguments)]
pub fn render_toolbar(
    ui: &mut egui::Ui,
    current_path: &str,
    path_input: &mut String,
    is_editing_path: &mut bool,
    show_address_history_menu: &mut bool,
    address_bar_focus_request: &mut bool,
    search_query: &mut String,
    navigation: &NavigationHistory,
    _view_mode: ViewMode,
    _sort_mode: SortMode,
    _sort_descending: bool,
    _thumbnail_size: &mut f32,
    show_preview_panel: bool,
    _is_renaming: bool,
    computer_icon: Option<&egui::TextureHandle>,
    svg_manager: &mut SvgIconManager,
) -> Option<ToolbarAction> {
    let mut action = None;
    let recent_paths: Vec<(String, String)> = navigation
        .recent_paths(5)
        .into_iter()
        .filter(|path| !path.is_empty())
        .map(|path| {
            let display = if path == COMPUTER_VIEW_ID {
                t!("nav.computer").to_string()
            } else if path == RECYCLE_BIN_VIEW_ID {
                t!("nav.recycle_bin").to_string()
            } else {
                path.clone()
            };
            (display, path)
        })
        .collect();

    ui.horizontal(|ui| {
        ui.style_mut().spacing.item_spacing.x = 8.0;

        // 1. NAVIGATION (LEFT) - Blocked during renaming
        let can_back = navigation.can_go_back() && !_is_renaming;
        if widgets::icon_button(ui, svg_manager, theme::ICON_ARROW_LEFT, &t!("toolbar.back"), None).clicked()
            && can_back
        {
            action = Some(ToolbarAction::GoBack);
        }

        let can_forward = navigation.can_go_forward() && !_is_renaming;
        if widgets::icon_button(ui, svg_manager, theme::ICON_ARROW_RIGHT, &t!("toolbar.forward"), None).clicked()
            && can_forward
        {
            action = Some(ToolbarAction::GoForward);
        }

        if widgets::icon_button(
            ui,
            svg_manager,
            theme::ICON_ARROW_UP,
            &t!("toolbar.up"),
            None,
        )
        .clicked()
            && !_is_renaming
        {
            action = Some(ToolbarAction::GoUp);
        }

        if widgets::icon_button(ui, svg_manager, theme::ICON_REFRESH, &t!("toolbar.reload"), None).clicked()
            && !_is_renaming
        {
            action = Some(ToolbarAction::Refresh);
        }

        ui.separator();

        // Home / Computer
        if widgets::icon_button(
            ui,
            svg_manager,
            theme::ICON_HOME,
            &t!("toolbar.home"),
            computer_icon,
        )
        .clicked()
            && !_is_renaming
        {
            action = Some(ToolbarAction::NavigateToComputer);
        }

        // 2. RIGHT-SIDE ELEMENTS (RIGHT -> LEFT)
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(4.0);

            // Detalhes (Preview Panel Toggle)
            if widgets::toggle_icon_button(
                ui,
                svg_manager,
                theme::ICON_DETAILS,
                show_preview_panel,
                &t!("toolbar.details"),
            )
            .clicked()
            {
                action = Some(ToolbarAction::TogglePreviewPanel);
            }

            ui.separator();

            // Search
            let input_height = 26.0;
            let search_width = (ui.available_width() * 0.45).min(250.0).max(120.0);
            // Creates a container visually similar to an input, but manual to contain the button
            let (search_rect, search_resp) = ui.allocate_exact_size(
                egui::vec2(search_width, input_height),
                egui::Sense::click_and_drag(),
            );

            // Draw background for search field
            let visuals = ui.style().interact(&search_resp);
            ui.painter()
                .rect_filled(search_rect, visuals.corner_radius, theme::input_bg_color(ui.visuals().dark_mode));
            ui.painter().rect_stroke(
                search_rect,
                visuals.corner_radius,
                ui.visuals().widgets.inactive.bg_stroke,
                egui::StrokeKind::Inside,
            );

            let mut search_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(search_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );

            // Left padding
            search_ui.add_space(6.0);

            // Search icon (now inside the bar, on the left, premium style)
            crate::ui::svg_icons::icon_image(&mut search_ui, svg_manager, "search", 14.0);
            search_ui.add_space(4.0);

            let has_text = !search_query.is_empty();
            // Text width logic: Total - Icons - Paddings
            // Total (250) - Icon(14) - Pad(6+4) - ClearBtn(18 if present) - Pad(4) - RightPad(4)
            // Previously RightPad was missing from the calculation and addition.
            let text_available_w =
                search_ui.available_width() - if has_text { 22.0 + 4.0 } else { 4.0 };

            let hint = egui::RichText::new(t!("toolbar.search_placeholder")).color(egui::Color32::from_gray(120));
            let text_resp = search_ui.add_sized(
                egui::vec2(text_available_w, input_height - 2.0),
                egui::TextEdit::singleline(search_query)
                    .frame(false)
                    .hint_text(hint)
                    .text_color(theme::text_color(ui.visuals().dark_mode))
                    .vertical_align(egui::Align::Center),
            );

            if text_resp.changed() {
                action = Some(ToolbarAction::Search(search_query.clone()));
            }

            // Clear Button (X)
            if has_text {
                if search_ui
                    .add(
                        egui::Button::new("✕")
                            .frame(false)
                            .min_size(egui::vec2(18.0, 18.0)),
                    )
                    .on_hover_text(t!("toolbar.clear_search"))
                    .clicked()
                {
                    search_query.clear();
                    action = Some(ToolbarAction::Search(String::new()));
                }
                search_ui.add_space(4.0);
            }

            // Focus input when clicking empty container
            if search_resp.clicked() {
                text_resp.request_focus();
            }

            ui.separator();

            // 3. ADDRESS BAR (Breadcrumbs or Editing)
            // Same technique as search bar: allocate + new_child
            let addr_width = (ui.available_width() - 4.0).max(100.0);
            let (addr_rect, addr_resp) =
                ui.allocate_exact_size(egui::vec2(addr_width, input_height), egui::Sense::click());

            // Draw background
            ui.painter()
                .rect_filled(addr_rect, 4.0, theme::input_bg_color(ui.visuals().dark_mode));
            ui.painter().rect_stroke(
                addr_rect,
                4.0,
                ui.visuals().widgets.inactive.bg_stroke,
                egui::StrokeKind::Inside,
            );

            // Create child UI inside the rectangle (same as search)
            let mut addr_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(addr_rect.shrink(4.0))
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            addr_ui.set_clip_rect(addr_rect);

            if *is_editing_path {
                let show_history_close_button = *show_address_history_menu && !recent_paths.is_empty();
                let close_button_width = if show_history_close_button { 22.0 } else { 0.0 };
                let text_width = (addr_ui.available_width() - close_button_width).max(40.0);
                let edit_response = addr_ui.add_sized(
                    egui::vec2(text_width, addr_ui.available_height()),
                    egui::TextEdit::singleline(path_input)
                        .hint_text(t!("toolbar.path_placeholder"))
                        .id_source("address_edit")
                        .frame(false)
                        .text_color(theme::text_color(ui.visuals().dark_mode)),
                );

                let mut close_history_clicked = false;

                if show_history_close_button {
                    let close_history_response = addr_ui
                        .add(
                            egui::Button::new("✕")
                                .frame(false)
                                .min_size(egui::vec2(18.0, 18.0)),
                        )
                        .on_hover_text(t!("toolbar.close_history"));

                    if close_history_response.clicked() {
                        *show_address_history_menu = false;
                        close_history_clicked = true;
                    }
                }

                if edit_response.clicked() && !recent_paths.is_empty() {
                    *show_address_history_menu = true;
                }

                if !close_history_clicked
                    && (edit_response.clicked_elsewhere()
                    || (edit_response.lost_focus()
                        && !addr_ui.input(|i| i.key_pressed(egui::Key::Enter))))
                {
                    action = Some(ToolbarAction::CancelPathInput);
                }

                // Ctrl+L: focar o campo diretamente na response (cursor posicionado no fim)
                if *address_bar_focus_request {
                    edit_response.request_focus();
                    *address_bar_focus_request = false;
                }

                if close_history_clicked {
                    edit_response.request_focus();
                }

                if addr_ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    action = Some(ToolbarAction::CommitPathInput(path_input.clone()));
                }

                if addr_ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    action = Some(ToolbarAction::CancelPathInput);
                }

                if *show_address_history_menu && !recent_paths.is_empty() {
                    let popup_id = egui::Id::new("address_history_popup");
                    let mut selected_path = None;

                    let popup_response = egui::Area::new(popup_id)
                        .order(egui::Order::Foreground)
                        .fixed_pos(egui::pos2(addr_rect.left(), addr_rect.bottom() + 2.0))
                        .show(ui.ctx(), |ui| {
                            egui::Frame::popup(ui.style()).show(ui, |ui| {
                                ui.set_min_width(addr_rect.width());
                                ui.set_max_width(addr_rect.width());
                                ui.spacing_mut().item_spacing.y = 0.0;

                                for (display, actual_path) in &recent_paths {
                                    let item_size = egui::vec2(addr_rect.width() - 8.0, 28.0);
                                    let (item_rect, response) =
                                        ui.allocate_exact_size(item_size, egui::Sense::click());
                                    let visuals = ui.style().interact(&response);

                                    if response.hovered() || response.highlighted() {
                                        ui.painter().rect_filled(
                                            item_rect,
                                            visuals.corner_radius,
                                            visuals.weak_bg_fill,
                                        );
                                    }

                                    let text_rect = item_rect.shrink2(egui::vec2(8.0, 0.0));
                                    let font_id = egui::TextStyle::Button.resolve(ui.style());
                                    let truncated = crate::ui::preview_panel::utils::truncate_text_to_fit(
                                        display,
                                        text_rect.width(),
                                        &font_id,
                                        ui,
                                    );
                                    ui.painter().text(
                                        egui::pos2(text_rect.left(), text_rect.center().y),
                                        egui::Align2::LEFT_CENTER,
                                        &truncated,
                                        font_id,
                                        theme::text_color(ui.visuals().dark_mode),
                                    );

                                    if truncated != *display {
                                        let _ = response.clone().on_hover_text(display.clone());
                                    }

                                    if response.clicked() {
                                        selected_path = Some(actual_path.clone());
                                    }
                                }
                            });
                        });

                    if let Some(path) = selected_path {
                        *show_address_history_menu = false;
                        action = Some(ToolbarAction::SelectAddressHistoryPath(path));
                    } else if ui.ctx().input(|i| i.pointer.any_pressed()) {
                        if let Some(pointer_pos) = ui.ctx().input(|i| i.pointer.press_origin()) {
                            let clicked_address_bar = addr_rect.contains(pointer_pos);
                            let clicked_popup = popup_response.response.rect.contains(pointer_pos);

                            if !clicked_address_bar && !clicked_popup {
                                *show_address_history_menu = false;
                            }
                        }
                    }
                }
            } else {
                addr_ui.spacing_mut().item_spacing.x = 2.0;

                if current_path == COMPUTER_VIEW_ID {
                    addr_ui.label(
                        egui::RichText::new(t!("nav.computer"))
                            .size(13.0)
                            .color(theme::text_color(addr_ui.visuals().dark_mode)),
                    );
                } else if current_path == RECYCLE_BIN_VIEW_ID {
                    addr_ui.label(
                        egui::RichText::new(t!("nav.recycle_bin"))
                            .size(13.0)
                            .color(theme::text_color(addr_ui.visuals().dark_mode)),
                    );
                } else {
                    // M-3: use cached segments — only recomputed on path change
                    let segments = breadcrumb_segments(current_path);
                    let seg_count = segments.len();

                    if seg_count == 0 {
                        // nothing to render
                    } else {
                        let btn_font = egui::TextStyle::Button.resolve(addr_ui.style());
                        let sep_font = egui::FontId::proportional(14.0);
                        let btn_pad_x = addr_ui.style().spacing.button_padding.x * 2.0;
                        let sep_width = measure_text_width(&addr_ui, "›", &sep_font);

                        let seg_widths: Vec<f32> = segments
                            .iter()
                            .map(|(display, _)| {
                                measure_text_width(&addr_ui, display, &btn_font) + btn_pad_x
                            })
                            .collect();

                        let total_width: f32 = seg_widths.iter().sum::<f32>()
                            + sep_width * (seg_count.saturating_sub(1)) as f32;
                        let available_width = addr_ui.available_width();

                        if total_width <= available_width {
                            // Everything fits — render all segments normally
                            for (seg_idx, (display, target_path)) in
                                segments.into_iter().enumerate()
                            {
                                render_breadcrumb_button(
                                    &mut addr_ui,
                                    &display,
                                    target_path,
                                    &mut action,
                                );
                                if seg_idx < seg_count - 1 {
                                    addr_ui.label(
                                        egui::RichText::new("›")
                                            .size(14.0)
                                            .color(egui::Color32::from_gray(120)),
                                    );
                                }
                            }
                        } else {
                            // Truncated layout: root + … + last segments that fit
                            let ellipsis_w =
                                measure_text_width(&addr_ui, "…", &btn_font) + btn_pad_x;
                            let root_width = seg_widths[0];
                            let last_width = seg_widths[seg_count - 1];

                            // Minimum needed for root + last (always try to keep these)
                            let mut needed = root_width + sep_width + last_width;
                            let mut show_ellipsis = false;

                            if seg_count > 2 {
                                let with_ellipsis = needed + sep_width + ellipsis_w;
                                if with_ellipsis <= available_width {
                                    needed = with_ellipsis;
                                    show_ellipsis = true;
                                }
                            }

                            // Try to fit additional segments from the end
                            let mut visible_end_count = 1;
                            if needed <= available_width && seg_count > 2 {
                                for i in (1..seg_count - 1).rev() {
                                    let extra = seg_widths[i] + sep_width;
                                    if needed + extra <= available_width {
                                        needed += extra;
                                        visible_end_count += 1;
                                    } else {
                                        break;
                                    }
                                }
                            }

                            if needed > available_width && seg_count > 1 {
                                // Fallback: even root+last doesn't fit — show as many tail
                                // segments as possible without root/ellipsis
                                let mut used = 0.0;
                                let mut vis = 0;
                                for i in (0..seg_count).rev() {
                                    let extra =
                                        seg_widths[i] + if vis > 0 { sep_width } else { 0.0 };
                                    if used + extra <= available_width {
                                        used += extra;
                                        vis += 1;
                                    } else {
                                        break;
                                    }
                                }
                                let start = seg_count - vis;
                                for i in start..seg_count {
                                    let (display, target_path) = segments[i].clone();
                                    render_breadcrumb_button(
                                        &mut addr_ui,
                                        &display,
                                        target_path,
                                        &mut action,
                                    );
                                    if i < seg_count - 1 {
                                        addr_ui.label(
                                            egui::RichText::new("›")
                                                .size(14.0)
                                                .color(egui::Color32::from_gray(120)),
                                        );
                                    }
                                }
                            } else {
                                // Root
                                render_breadcrumb_button(
                                    &mut addr_ui,
                                    &segments[0].0,
                                    segments[0].1.clone(),
                                    &mut action,
                                );
                                addr_ui.label(
                                    egui::RichText::new("›")
                                        .size(14.0)
                                        .color(egui::Color32::from_gray(120)),
                                );

                                let last_visible_start = seg_count - visible_end_count;
                                let has_hidden_middle = last_visible_start > 1;

                                if show_ellipsis && has_hidden_middle {
                                    // Clickable ellipsis with overflow popup
                                    let popup_id =
                                        egui::Id::new("breadcrumb_overflow_popup");
                                    let mut show_overflow = addr_ui.ctx().memory(|m| {
                                        m.data.get_temp::<bool>(popup_id).unwrap_or(false)
                                    });

                                    let ellipsis_resp = addr_ui
                                        .scope(|ui| {
                                            let hover_color = if ui.visuals().dark_mode {
                                                theme::color_dark_hover()
                                            } else {
                                                theme::color_hover()
                                            };
                                            ui.visuals_mut().widgets.inactive.bg_fill =
                                                egui::Color32::TRANSPARENT;
                                            ui.visuals_mut().widgets.inactive.weak_bg_fill =
                                                egui::Color32::TRANSPARENT;
                                            ui.visuals_mut().widgets.inactive.bg_stroke =
                                                egui::Stroke::NONE;

                                            ui.visuals_mut().widgets.hovered.bg_fill =
                                                hover_color;
                                            ui.visuals_mut().widgets.hovered.weak_bg_fill =
                                                hover_color;
                                            ui.visuals_mut().widgets.hovered.bg_stroke =
                                                egui::Stroke::NONE;

                                            ui.visuals_mut().widgets.active.bg_fill =
                                                if ui.visuals().dark_mode {
                                                    egui::Color32::from_gray(70)
                                                } else {
                                                    egui::Color32::from_gray(210)
                                                };
                                            ui.visuals_mut().widgets.active.bg_stroke =
                                                egui::Stroke::NONE;

                                            ui.button(
                                                egui::RichText::new("…")
                                                    .color(theme::text_color(
                                                        ui.visuals().dark_mode,
                                                    )),
                                            )
                                        })
                                        .inner;

                                    if ellipsis_resp.clicked() {
                                        show_overflow = !show_overflow;
                                        addr_ui.ctx().memory_mut(|m| {
                                            m.data.insert_temp(popup_id, show_overflow);
                                        });
                                    }

                                    if show_overflow {
                                        let mut selected_path = None;
                                        let popup_response = egui::Area::new(popup_id)
                                            .order(egui::Order::Foreground)
                                            .fixed_pos(egui::pos2(
                                                ellipsis_resp.rect.left(),
                                                ellipsis_resp.rect.bottom() + 2.0,
                                            ))
                                            .show(addr_ui.ctx(), |ui| {
                                                egui::Frame::popup(ui.style()).show(ui, |ui| {
                                                    ui.set_min_width(180.0);
                                                    ui.spacing_mut().item_spacing.y = 0.0;

                                                    for i in 1..last_visible_start {
                                                        let (display, actual_path) =
                                                            segments[i].clone();
                                                        let item_size =
                                                            egui::vec2(200.0, 28.0);
                                                        let (item_rect, response) = ui
                                                            .allocate_exact_size(
                                                                item_size,
                                                                egui::Sense::click(),
                                                            );
                                                        let visuals =
                                                            ui.style().interact(&response);

                                                        if response.hovered()
                                                            || response.highlighted()
                                                        {
                                                            ui.painter().rect_filled(
                                                                item_rect,
                                                                visuals.corner_radius,
                                                                visuals.weak_bg_fill,
                                                            );
                                                        }

                                                        let text_rect = item_rect
                                                            .shrink2(egui::vec2(8.0, 0.0));
                                                        ui.painter().text(
                                                            egui::pos2(
                                                                text_rect.left(),
                                                                text_rect.center().y,
                                                            ),
                                                            egui::Align2::LEFT_CENTER,
                                                            &display,
                                                            egui::TextStyle::Button
                                                                .resolve(ui.style()),
                                                            theme::text_color(
                                                                ui.visuals().dark_mode,
                                                            ),
                                                        );

                                                        if response.clicked() {
                                                            selected_path =
                                                                Some(actual_path);
                                                        }
                                                    }
                                                });
                                            });

                                        if let Some(path) = selected_path {
                                            addr_ui.ctx().memory_mut(|m| {
                                                m.data
                                                    .insert_temp::<bool>(popup_id, false);
                                            });
                                            action =
                                                Some(ToolbarAction::Navigate(path));
                                        } else if addr_ui
                                            .ctx()
                                            .input(|i| i.pointer.any_pressed())
                                        {
                                            if let Some(pointer_pos) = addr_ui
                                                .ctx()
                                                .input(|i| i.pointer.press_origin())
                                            {
                                                let clicked_ellipsis = ellipsis_resp
                                                    .rect
                                                    .contains(pointer_pos);
                                                let clicked_popup = popup_response
                                                    .response
                                                    .rect
                                                    .contains(pointer_pos);
                                                if !clicked_ellipsis && !clicked_popup {
                                                    addr_ui.ctx().memory_mut(|m| {
                                                        m.data.insert_temp::<bool>(
                                                            popup_id, false,
                                                        );
                                                    });
                                                }
                                            }
                                        }
                                    }

                                    addr_ui.label(
                                        egui::RichText::new("›")
                                            .size(14.0)
                                            .color(egui::Color32::from_gray(120)),
                                    );
                                }

                                // Render last visible_end_count segments
                                let start = seg_count - visible_end_count;
                                for i in start..seg_count {
                                    let (display, target_path) = segments[i].clone();
                                    render_breadcrumb_button(
                                        &mut addr_ui,
                                        &display,
                                        target_path,
                                        &mut action,
                                    );
                                    if i < seg_count - 1 {
                                        addr_ui.label(
                                            egui::RichText::new("›")
                                                .size(14.0)
                                                .color(egui::Color32::from_gray(120)),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }

                // Click on empty area opens editing
                if addr_resp.clicked() && action.is_none() {
                    action = Some(ToolbarAction::StartAddressEditWithHistory);
                }
            }

            if matches!(
                action,
                Some(ToolbarAction::StartAddressEdit)
                    | Some(ToolbarAction::StartAddressEditWithHistory)
            ) {
                ui.ctx().memory_mut(|m| {
                    m.request_focus(egui::Id::from("address_edit").with("text_edit"))
                });
            }
        });
    });

    action
}
