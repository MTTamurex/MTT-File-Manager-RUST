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

pub fn render_toolbar(
    ui: &mut egui::Ui,
    current_path: &str,
    path_input: &mut String,
    is_editing_path: &mut bool,
    search_query: &mut String,
    navigation: &NavigationHistory,
    view_mode: ViewMode,
    sort_mode: SortMode,
    sort_descending: bool,
    thumbnail_size: &mut f32,
    show_preview_panel: bool,
    _is_renaming: bool,
    computer_icon: Option<&egui::TextureHandle>,
    svg_manager: &mut SvgIconManager,
) -> Option<ToolbarAction> {
    let mut action = None;

    ui.horizontal(|ui| {
        ui.style_mut().spacing.item_spacing.x = 8.0;

        // 1. NAVEGAÇÃO (ESQUERDA) - Bloqueados durante renomeação
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

        // Botão de Nova Pasta
        if widgets::icon_button(
            ui,
            svg_manager,
            theme::ICON_FOLDER_ADD,
            "Criar Nova Pasta (Ctrl+Shift+N)",
            None,
        )
        .clicked()
            && !_is_renaming
        {
            action = Some(ToolbarAction::CreateFolder);
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

        // 2. ELEMENTOS DA DIREITA (DIREITA -> ESQUERDA)
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(4.0);

            // Zoom
            ui.add_sized(
                egui::vec2(80.0, 20.0),
                egui::Slider::new(thumbnail_size, 64.0..=256.0).show_value(false),
            );
            ui.label("Zoom");

            ui.separator();

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

            // View Mode
            if widgets::toggle_icon_button(
                ui,
                svg_manager,
                theme::ICON_LIST,
                matches!(view_mode, ViewMode::List),
                "Lista",
            )
            .clicked()
            {
                if !matches!(view_mode, ViewMode::List) {
                    action = Some(ToolbarAction::ToggleViewMode);
                }
            }
            if widgets::toggle_icon_button(
                ui,
                svg_manager,
                theme::ICON_GRID,
                matches!(view_mode, ViewMode::Grid),
                "Grade",
            )
            .clicked()
            {
                if !matches!(view_mode, ViewMode::Grid) {
                    action = Some(ToolbarAction::ToggleViewMode);
                }
            }

            ui.separator();

            // Ordenação
            //Ordenação - Botão de seta
            let sort_symbol = if sort_descending { "↓" } else { "↑" };
            
            ui.scope(|ui| {
                // Make sort button transparent/frameless, only show background on hover
                ui.visuals_mut().widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
                ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
                ui.visuals_mut().widgets.inactive.fg_stroke = egui::Stroke::NONE;
                ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
                
                ui.visuals_mut().widgets.hovered.bg_fill = egui::Color32::from_gray(240);
                ui.visuals_mut().widgets.hovered.fg_stroke = egui::Stroke::NONE;
                ui.visuals_mut().widgets.hovered.bg_stroke = egui::Stroke::NONE;
                
                if ui
                    .add(egui::Button::new(egui::RichText::new(sort_symbol).color(egui::Color32::BLACK)))
                    .on_hover_text("Inverter Ordem")
                    .clicked()
                {
                    action = Some(ToolbarAction::ToggleSortDescending);
                }
            });

            // ComboBox com fundo branco e borda preta permanente
            ui.scope(|ui| {
                // Force all visual states to white background with black strokes
                let black_stroke = egui::Stroke::new(1.0, egui::Color32::BLACK);
                
                // noninteractive (used for some default rendering)
                ui.visuals_mut().widgets.noninteractive.bg_fill = egui::Color32::WHITE;
                ui.visuals_mut().widgets.noninteractive.fg_stroke = black_stroke; // Arrow
                ui.visuals_mut().widgets.noninteractive.bg_stroke = egui::Stroke::NONE; // No Border
                
                // inactive (normal state)
                ui.visuals_mut().widgets.inactive.bg_fill = egui::Color32::WHITE;
                ui.visuals_mut().widgets.inactive.fg_stroke = black_stroke; // Arrow
                ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE; // No Border
                
                // hovered
                ui.visuals_mut().widgets.hovered.bg_fill = egui::Color32::from_gray(245); // Subtle feedback
                ui.visuals_mut().widgets.hovered.fg_stroke = black_stroke;
                ui.visuals_mut().widgets.hovered.bg_stroke = egui::Stroke::NONE;
                
                // active
                ui.visuals_mut().widgets.active.bg_fill = egui::Color32::from_gray(235);
                ui.visuals_mut().widgets.active.fg_stroke = black_stroke;
                ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::NONE;
                
                ui.visuals_mut().override_text_color = Some(egui::Color32::BLACK);
                
                egui::ComboBox::from_id_salt("sort_mode")
                .selected_text(match sort_mode {
                    SortMode::Name => "Nome",
                    SortMode::Date => "Data",
                    SortMode::Size => "Tamanho",
                    SortMode::Type => "Tipo",
                })
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_value(&mut SortMode::Name, sort_mode, "Nome")
                        .clicked()
                    {
                        action = Some(ToolbarAction::ChangeSortMode(SortMode::Name));
                    }
                    if ui
                        .selectable_value(&mut SortMode::Date, sort_mode, "Data")
                        .clicked()
                    {
                        action = Some(ToolbarAction::ChangeSortMode(SortMode::Date));
                    }
                    if ui
                        .selectable_value(&mut SortMode::Size, sort_mode, "Tamanho")
                        .clicked()
                    {
                        action = Some(ToolbarAction::ChangeSortMode(SortMode::Size));
                    }
                    if ui
                        .selectable_value(&mut SortMode::Type, sort_mode, "Tipo")
                        .clicked()
                    {
                        action = Some(ToolbarAction::ChangeSortMode(SortMode::Type));
                    }
                });
            });

            ui.separator();

            // Busca
            let search_width = 250.0;
            // Cria um container visualmente similar a um input, mas manual para conter o botão
            let (search_rect, search_resp) = ui.allocate_exact_size(
                egui::vec2(search_width, 22.0),
                egui::Sense::click_and_drag(),
            );

            // Desenha o fundo branco para campo de busca
            let visuals = ui.style().interact(&search_resp);
            ui.painter().rect_filled(
                search_rect,
                visuals.corner_radius,
                egui::Color32::WHITE,
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

            // Padding esquerdo
            search_ui.add_space(6.0);

            // Ícone de busca (agora dentro da barra, à esquerda, estilo premium)
            crate::ui::svg_icons::icon_image(&mut search_ui, svg_manager, "search", 14.0);
            search_ui.add_space(4.0);

            let has_text = !search_query.is_empty();
            // Lógica para largura do texto: Total - Ícones - Paddings
            // Total (250) - Icon(14) - Pad(6+4) - ClearBtn(18 se houver) - Pad(4) - RightPad(4)
            // Antes faltava o RightPad no cálculo e na adição.
            let text_available_w = search_ui.available_width() - if has_text { 22.0 + 4.0 } else { 4.0 };

            let text_resp = search_ui.add_sized(
                egui::vec2(text_available_w, 20.0),
                egui::TextEdit::singleline(search_query)
                    .frame(false)
                    .hint_text(egui::RichText::new("Buscar...").color(egui::Color32::from_gray(120)))
                    .text_color(egui::Color32::BLACK)
                    .vertical_align(egui::Align::Center),
            );

            if text_resp.changed() {
                action = Some(ToolbarAction::Search(search_query.clone()));
            }

            // Botão Limpar (X)
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

            // Foca no input se clicar no container vazio
            if search_resp.clicked() {
                text_resp.request_focus();
            }

            ui.separator();

            // 3. BARRA DE ENDEREÇO (Breadcrumbs ou Edição)
            // Mesma técnica da barra de busca: allocate + new_child
            let addr_width = (ui.available_width() - 4.0).max(100.0);
            let (addr_rect, addr_resp) = ui.allocate_exact_size(
                egui::vec2(addr_width, 22.0),
                egui::Sense::click(),
            );

            // Desenha fundo branco
            ui.painter().rect_filled(
                addr_rect,
                4.0,
                egui::Color32::WHITE,
            );
            ui.painter().rect_stroke(
                addr_rect,
                4.0,
                ui.visuals().widgets.inactive.bg_stroke,
                egui::StrokeKind::Inside,
            );

            // Cria UI filha dentro do retângulo (igual à busca)
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
                    addr_ui.label(egui::RichText::new("Este Computador").size(13.0).color(egui::Color32::BLACK));
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

                        // Breadcrumb clicável
                        if addr_ui.button(&display).clicked() {
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

                // Clique na área vazia abre edição
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
