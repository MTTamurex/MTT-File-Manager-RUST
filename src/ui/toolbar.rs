use crate::application::navigation::NavigationHistory;
use crate::domain::file_entry::{SortMode, ViewMode};
use crate::ui::svg_icons::SvgIconManager;
use crate::ui::theme;
use crate::ui::widgets;
use eframe::egui;

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
    UpdatePathInput(String),
    CommitPathInput(String),
    CancelPathInput,
}

#[allow(clippy::too_many_arguments)]
pub fn render_toolbar(
    ui: &mut egui::Ui,
    current_path: &str,
    path_input: &mut String,
    is_editing_path: &mut bool,
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

    ui.horizontal(|ui| {
        ui.style_mut().spacing.item_spacing.x = 8.0;

        // 1. NAVIGATION (LEFT) - Blocked during renaming
        let can_back = navigation.can_go_back() && !_is_renaming;
        if widgets::icon_button(ui, svg_manager, theme::ICON_ARROW_LEFT, "Voltar", None).clicked()
            && can_back
        {
            action = Some(ToolbarAction::GoBack);
        }

        let can_forward = navigation.can_go_forward() && !_is_renaming;
        if widgets::icon_button(ui, svg_manager, theme::ICON_ARROW_RIGHT, "Avançar", None).clicked()
            && can_forward
        {
            action = Some(ToolbarAction::GoForward);
        }

        if widgets::icon_button(
            ui,
            svg_manager,
            theme::ICON_ARROW_UP,
            "Subir um nível",
            None,
        )
        .clicked()
            && !_is_renaming
        {
            action = Some(ToolbarAction::GoUp);
        }

        if widgets::icon_button(ui, svg_manager, theme::ICON_REFRESH, "Recarregar", None).clicked()
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
            "Este Computador",
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
                "Detalhes",
            )
            .clicked()
            {
                action = Some(ToolbarAction::TogglePreviewPanel);
            }

            ui.separator();

            // Search
            let search_width = 250.0;
            let input_height = 26.0;
            // Creates a container visually similar to an input, but manual to contain the button
            let (search_rect, search_resp) = ui.allocate_exact_size(
                egui::vec2(search_width, input_height),
                egui::Sense::click_and_drag(),
            );

            // Draw white background for search field
            let visuals = ui.style().interact(&search_resp);
            ui.painter()
                .rect_filled(search_rect, visuals.corner_radius, egui::Color32::WHITE);
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

            let text_resp = search_ui.add_sized(
                egui::vec2(text_available_w, input_height - 2.0),
                egui::TextEdit::singleline(search_query)
                    .frame(false)
                    .hint_text(
                        egui::RichText::new("Buscar...").color(egui::Color32::from_gray(120)),
                    )
                    .text_color(egui::Color32::BLACK)
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
                    .on_hover_text("Limpar busca")
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

            // Draw white background
            ui.painter()
                .rect_filled(addr_rect, 4.0, egui::Color32::WHITE);
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

            if *is_editing_path {
                let edit_response = addr_ui.add_sized(
                    addr_ui.available_size(),
                    egui::TextEdit::singleline(path_input)
                        .hint_text("Caminho...")
                        .id_source("address_edit")
                        .frame(false)
                        .text_color(egui::Color32::BLACK),
                );

                if edit_response.clicked_elsewhere()
                    || (edit_response.lost_focus()
                        && !addr_ui.input(|i| i.key_pressed(egui::Key::Enter)))
                {
                    action = Some(ToolbarAction::CancelPathInput);
                }

                if addr_ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    action = Some(ToolbarAction::CommitPathInput(path_input.clone()));
                }

                if addr_ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    action = Some(ToolbarAction::CancelPathInput);
                }
            } else {
                addr_ui.spacing_mut().item_spacing.x = 2.0;

                if current_path == "Este Computador" {
                    addr_ui.label(
                        egui::RichText::new("Este Computador")
                            .size(13.0)
                            .color(egui::Color32::BLACK),
                    );
                } else {
                    let path = std::path::Path::new(current_path);
                    let mut full_accumulated = std::path::PathBuf::new();
                    let components: Vec<_> = path.components().collect();

                    for (i, comp) in components.iter().enumerate() {
                        let comp_str = comp.as_os_str().to_string_lossy();
                        let display_name = comp_str.trim_end_matches('\\');

                        if display_name.is_empty() && i > 0 {
                            continue;
                        }

                        full_accumulated.push(comp);
                        let target_path = {
                            let p = full_accumulated.to_string_lossy().to_string();
                            if p.len() == 2 && p.ends_with(':') {
                                format!("{}\\", p)
                            } else {
                                p
                            }
                        };

                        let display = if display_name.is_empty() {
                            comp_str.into_owned()
                        } else {
                            display_name.to_string()
                        };

                        // Clickable breadcrumb - transparent, light gray on hover
                        let btn_resp = addr_ui
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
                                ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;

                                ui.visuals_mut().widgets.hovered.bg_fill = hover_color;
                                ui.visuals_mut().widgets.hovered.weak_bg_fill = hover_color;
                                ui.visuals_mut().widgets.hovered.bg_stroke = egui::Stroke::NONE;

                                ui.visuals_mut().widgets.active.bg_fill =
                                    egui::Color32::from_gray(210);
                                ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::NONE;

                                ui.button(egui::RichText::new(&display).color(egui::Color32::BLACK))
                            })
                            .inner;

                        if btn_resp.clicked() {
                            action = Some(ToolbarAction::Navigate(target_path));
                        }

                        if i < components.len() - 1 {
                            addr_ui.label(
                                egui::RichText::new("›")
                                    .size(14.0)
                                    .color(egui::Color32::from_gray(120)),
                            );
                        }
                    }
                }

                // Click on empty area opens editing
                if addr_resp.clicked() && action.is_none() {
                    action = Some(ToolbarAction::StartAddressEdit);
                }
            }

            if matches!(action, Some(ToolbarAction::StartAddressEdit)) {
                ui.ctx().memory_mut(|m| {
                    m.request_focus(egui::Id::from("address_edit").with("text_edit"))
                });
            }
        });
    });

    action
}
