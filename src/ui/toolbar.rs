use crate::domain::file_entry::{SortMode, ViewMode};
use crate::domain::special_paths::{COMPUTER_VIEW_ID, RECYCLE_BIN_VIEW_ID};
use crate::ui::svg_icons::SvgIconManager;
use crate::ui::theme;
use crate::ui::widgets;
use eframe::egui;
use rust_i18n::t;
mod address_bar;
mod breadcrumb_view;
mod breadcrumbs;
mod navigation_buttons;
use breadcrumbs::breadcrumb_segments;

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
    cloud_roots: &[crate::domain::cloud_root::CloudRoot],
    disks: &[(String, String)],
    current_path_display_override: Option<&str>,
    path_input: &mut String,
    is_editing_path: &mut bool,
    show_address_history_menu: &mut bool,
    address_bar_focus_request: &mut bool,
    search_query: &mut String,
    recent_paths: &[(String, String)],
    can_go_back: bool,
    can_go_forward: bool,
    can_go_up: bool,
    _view_mode: ViewMode,
    _sort_mode: SortMode,
    _sort_descending: bool,
    _thumbnail_size: &mut f32,
    show_preview_panel: bool,
    _is_renaming: bool,
    computer_icon: Option<&egui::TextureHandle>,
    svg_manager: &mut SvgIconManager,
    shortcuts: &crate::app::shortcuts::ShortcutBindings,
) -> Option<ToolbarAction> {
    let mut action = None;

    ui.horizontal(|ui| {
        ui.style_mut().spacing.item_spacing.x = 8.0;

        action = navigation_buttons::render(
            ui,
            svg_manager,
            [can_go_back, can_go_forward, can_go_up],
            _is_renaming,
            shortcuts,
        );

        if widgets::icon_button(
            ui,
            svg_manager,
            theme::ICON_REFRESH,
            &t!("toolbar.reload"),
            None,
        )
        .clicked()
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
                &format!(
                    "{} ({})",
                    t!("toolbar.details"),
                    shortcuts.label(crate::app::shortcuts::ShortcutAction::TogglePreviewPanel)
                ),
            )
            .clicked()
            {
                action = Some(ToolbarAction::TogglePreviewPanel);
            }

            ui.separator();

            // Search
            let input_height = 26.0;
            let search_width = (ui.available_width() * 0.45).clamp(120.0, 250.0);
            // Creates a container visually similar to an input, but manual to contain the button
            let (search_rect, search_resp) = ui.allocate_exact_size(
                egui::vec2(search_width, input_height),
                egui::Sense::click_and_drag(),
            );

            // Draw background for search field
            let visuals = ui.style().interact(&search_resp);
            ui.painter().rect_filled(
                search_rect,
                visuals.corner_radius,
                theme::input_bg_color(ui.ctx()),
            );
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

            let hint = egui::RichText::new(t!("toolbar.search_placeholder"))
                .color(egui::Color32::from_gray(120));
            let mut text_resp = search_ui.add_sized(
                egui::vec2(text_available_w, input_height - 2.0),
                egui::TextEdit::singleline(search_query)
                    .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(4, 2)))
                    .hint_text(hint)
                    .text_color(theme::text_color(ui.ctx()))
                    .vertical_align(egui::Align::Center),
            );
            let search_text_menu = crate::ui::components::inline_rename::add_text_context_menu(
                &mut search_ui,
                &text_resp,
                search_query,
                false,
            );
            if search_text_menu.text_changed {
                text_resp.mark_changed();
            }

            if text_resp.changed() {
                action = Some(ToolbarAction::Search(search_query.clone()));
            }

            let search_escape_pressed = search_ui.input(|i| i.key_pressed(egui::Key::Escape));
            if has_text
                && search_escape_pressed
                && (text_resp.has_focus() || text_resp.lost_focus())
            {
                search_query.clear();
                action = Some(ToolbarAction::Search(String::new()));
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
                .rect_filled(addr_rect, 4.0, theme::input_bg_color(ui.ctx()));
            ui.painter().rect_stroke(
                addr_rect,
                4.0,
                ui.visuals().widgets.inactive.bg_stroke,
                egui::StrokeKind::Inside,
            );

            // Create child UI inside the rectangle (same as search)
            let mut addr_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(addr_rect.shrink2(egui::vec2(4.0, 1.0)))
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            addr_ui.set_clip_rect(addr_rect);

            if *is_editing_path {
                let show_history_close_button =
                    *show_address_history_menu && !recent_paths.is_empty();
                let (text_rect, close_rect) = address_bar::edit_rects(
                    addr_rect,
                    show_history_close_button,
                    addr_ui.spacing().item_spacing.x,
                );
                let edit_response = addr_ui.put(
                    text_rect,
                    egui::TextEdit::singleline(path_input)
                        .hint_text(t!("toolbar.path_placeholder"))
                        .id_source("address_edit")
                        .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(4, 2)))
                        .text_color(theme::text_color(ui.ctx()))
                        .vertical_align(egui::Align::Center),
                );
                let address_text_menu = crate::ui::components::inline_rename::add_text_context_menu(
                    &mut addr_ui,
                    &edit_response,
                    path_input,
                    false,
                );

                let mut close_history_clicked = false;

                if let Some(close_rect) = close_rect {
                    let close_history_response = address_bar::render_centered_clear_button(
                        &mut addr_ui,
                        close_rect,
                        &t!("toolbar.close_history"),
                    );

                    if close_history_response.clicked() {
                        *show_address_history_menu = false;
                        close_history_clicked = true;
                    }
                }

                if edit_response.clicked() && !recent_paths.is_empty() {
                    *show_address_history_menu = true;
                }

                if !close_history_clicked
                    && !address_text_menu.menu_active
                    && (edit_response.clicked_elsewhere()
                        || (edit_response.lost_focus()
                            && !addr_ui.input(|i| i.key_pressed(egui::Key::Enter))))
                {
                    action = Some(ToolbarAction::CancelPathInput);
                }

                // Select only on explicit activation, not on subsequent cursor clicks.
                if *address_bar_focus_request {
                    edit_response.request_focus();
                    let mut state =
                        egui::TextEdit::load_state(ui.ctx(), edit_response.id).unwrap_or_default();
                    state
                        .cursor
                        .set_char_range(Some(egui::text::CCursorRange::two(
                            egui::text::CCursor::new(0),
                            egui::text::CCursor::new(path_input.chars().count()),
                        )));
                    state.store(ui.ctx(), edit_response.id);
                    ui.ctx().request_repaint();
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

                                for (display, actual_path) in recent_paths {
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
                                    let truncated =
                                        crate::ui::preview_panel::utils::truncate_text_to_fit(
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
                                        theme::text_color(ui.ctx()),
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

                if let Some(display) = current_path_display_override {
                    addr_ui.label(
                        egui::RichText::new(display)
                            .size(13.0)
                            .color(theme::text_color(addr_ui.ctx())),
                    );
                } else if current_path == COMPUTER_VIEW_ID {
                    addr_ui.label(
                        egui::RichText::new(t!("nav.computer"))
                            .size(13.0)
                            .color(theme::text_color(addr_ui.ctx())),
                    );
                } else if current_path == RECYCLE_BIN_VIEW_ID {
                    addr_ui.label(
                        egui::RichText::new(t!("nav.recycle_bin"))
                            .size(13.0)
                            .color(theme::text_color(addr_ui.ctx())),
                    );
                } else {
                    // Cache also tracks locale and discovered cloud metadata.
                    let mut segments = breadcrumb_segments(current_path, cloud_roots);
                    breadcrumbs::decorate_drive_segments(
                        current_path,
                        &mut segments,
                        cloud_roots,
                        disks,
                    );
                    if let Some(navigation) = breadcrumb_view::render(&mut addr_ui, &segments) {
                        action = Some(navigation);
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
                *address_bar_focus_request = true;
            }
        });
    });

    action
}

#[cfg(test)]
mod tests;
