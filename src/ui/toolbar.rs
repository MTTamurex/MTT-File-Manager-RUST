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
        let is_renaming = false; // TODO: Pass is_renaming as param? For now assume false or add to signature if critical. Plan didn't have it but main.rs uses it.
                                 // Adding is_renaming to signature is better.

        let can_back = navigation.can_go_back() && !is_renaming;
        if widgets::icon_button(ui, svg_manager, theme::ICON_ARROW_LEFT, "Voltar", None).clicked()
            && can_back
        {
            action = Some(ToolbarAction::GoBack);
        }

        let can_forward = navigation.can_go_forward() && !is_renaming;
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
            && !is_renaming
        {
            action = Some(ToolbarAction::GoUp);
        }

        if widgets::icon_button(ui, svg_manager, theme::ICON_REFRESH, "Recarregar", None).clicked()
            && !is_renaming
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
            && !is_renaming
        {
            action = Some(ToolbarAction::CreateFolder);
        }

        ui.separator();

        // Home / Computer
        // Note: We need the computer icon texture. Passing it as an Option<&TextureHandle> in signature would be good.
        // For now finding "home" icon usage.
        if widgets::icon_button(
            ui,
            svg_manager,
            theme::ICON_HOME,
            "Este Computador",
            computer_icon,
        )
        .clicked()
            && !is_renaming
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
            let sort_symbol = if sort_descending { "↓" } else { "↑" };
            if ui
                .button(sort_symbol)
                .on_hover_text("Inverter Ordem")
                .clicked()
            {
                action = Some(ToolbarAction::ToggleSortDescending);
            }

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

            ui.separator();

            // Busca
            let search_width = 250.0;
            // Cria um container visualmente similar a um input, mas manual para conter o botão
            let (search_rect, search_resp) = ui.allocate_exact_size(
                egui::vec2(search_width, 22.0),
                egui::Sense::click_and_drag(),
            );

            // Desenha o fundo (imitando o estilo de input do tema)
            let visuals = ui.style().interact(&search_resp);
            ui.painter().rect_filled(
                search_rect,
                visuals.corner_radius,
                ui.visuals().widgets.inactive.bg_fill,
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
                    .hint_text("Buscar...")
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
            let addr_width = (ui.available_width() - 4.0).max(100.0);
            let (addr_rect, _addr_response) =
                ui.allocate_exact_size(egui::vec2(addr_width, 24.0), egui::Sense::hover());

            // IMPORTANTE: Usar allocate_new_ui com closure para ter o novo Ui com layout correto
            ui.allocate_new_ui(
                egui::UiBuilder::new()
                    .max_rect(addr_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    if *is_editing_path {
                        let edit_response = ui.add_sized(
                            ui.available_size(),
                            egui::TextEdit::singleline(path_input)
                                .hint_text("Caminho...")
                                .id_source("address_edit"),
                        );

                        if edit_response.clicked_elsewhere()
                            || (edit_response.lost_focus()
                                && !ui.input(|i| i.key_pressed(egui::Key::Enter)))
                        {
                            action = Some(ToolbarAction::CancelPathInput);
                        }

                        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            action = Some(ToolbarAction::CommitPathInput(path_input.clone()));
                        }

                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            action = Some(ToolbarAction::CancelPathInput);
                        }
                    } else {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;

                            if current_path == "Este Computador" {
                                ui.label(egui::RichText::new("Este Computador").size(13.0));
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
                                    // Normaliza drive roots: "Z:" -> "Z:\" para navegação correta
                                    let target_path = {
                                        let p = full_accumulated.to_string_lossy().to_string();
                                        if p.len() == 2 && p.ends_with(':') {
                                            format!("{}\\", p)
                                        } else {
                                            p
                                        }
                                    };

                                    // Nome do drive ou pasta
                                    let display = if display_name.is_empty() {
                                        comp_str.into_owned() // Root / ou C:\
                                    } else {
                                        display_name.to_string()
                                    };

                                    if ui.button(display).clicked() {
                                        action = Some(ToolbarAction::Navigate(target_path));
                                    }

                                    if i < components.len() - 1 {
                                        ui.label(
                                            egui::RichText::new("›")
                                                .size(14.0)
                                                .color(egui::Color32::from_gray(120)),
                                        );
                                    }
                                }
                            }

                            // Espaço clicável à direita para entrar no modo edição
                            let remaining = ui.available_width();
                            if remaining > 0.0 {
                                let (_rect, resp) = ui.allocate_exact_size(
                                    egui::vec2(remaining, ui.available_height()),
                                    egui::Sense::click(),
                                );
                                if resp.clicked() {
                                    action = Some(ToolbarAction::StartAddressEdit);
                                }
                            }
                        });
                    }
                },
            );

            if matches!(action, Some(ToolbarAction::StartAddressEdit)) {
                ui.ctx().memory_mut(|m| {
                    m.request_focus(egui::Id::from("address_edit").with("text_edit"))
                });
            }
        });
    });

    action
}
