//! Context menu rendering
//! Follows .cursorrules: single responsibility, < 300 lines

use eframe::egui;
use std::path::PathBuf;

use crate::application::context_menu::ContextMenuState;

/// Operations that can be performed from context menu
pub trait ContextMenuOperations {
    fn create_new_folder(&mut self);
    fn command_copy(&mut self);
    fn command_cut(&mut self);
    fn command_paste(&mut self);
    fn rename_item(&mut self, idx: usize);
    fn delete_with_shell(&mut self);
}

/// Renders the context menu
pub fn render_context_menu(
    ctx: &egui::Context,
    menu_state: &mut ContextMenuState,
    clipboard_file: &Option<PathBuf>,
    ops: &mut dyn ContextMenuOperations,
) -> bool {
    if !menu_state.is_open {
        return false;
    }

    // Exibe o menu
    let mut menu_closed = false;
    egui::Area::new(egui::Id::new("context_menu"))
        .fixed_pos(menu_state.position)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(180.0);

                // Menu em área vazia: mostra "Criar pasta" e "Colar" (se houver algo no clipboard)
                if menu_state.is_empty_area {
                    // Criar pasta
                    if ui.button("Criar pasta").clicked() {
                        ops.create_new_folder();
                        menu_closed = true;
                    }

                    // Colar (só se tiver algo no clipboard)
                    let can_paste = clipboard_file.is_some();
                    if ui
                        .add_enabled(can_paste, egui::Button::new("Colar"))
                        .clicked()
                    {
                        ops.command_paste();
                        menu_closed = true;
                    }
                } else {
                    // Menu em item: mostra todas as opções

                    // Copiar (só se tiver item selecionado)
                    let can_copy = menu_state.item_index.is_some();
                    if ui
                        .add_enabled(can_copy, egui::Button::new("Copiar"))
                        .clicked()
                    {
                        ops.command_copy();
                        menu_closed = true;
                    }

                    // Recortar (só se tiver item selecionado)
                    let can_cut = menu_state.item_index.is_some();
                    if ui
                        .add_enabled(can_cut, egui::Button::new("Recortar"))
                        .clicked()
                    {
                        ops.command_cut();
                        menu_closed = true;
                    }

                    // Colar (só se tiver algo no clipboard)
                    let can_paste = clipboard_file.is_some();
                    if ui
                        .add_enabled(can_paste, egui::Button::new("Colar"))
                        .clicked()
                    {
                        ops.command_paste();
                        menu_closed = true;
                    }

                    ui.separator();

                    // Renomear (só se tiver item selecionado)
                    let can_rename = menu_state.item_index.is_some();
                    if ui
                        .add_enabled(can_rename, egui::Button::new("Renomear"))
                        .clicked()
                    {
                        if let Some(idx) = menu_state.item_index {
                            ops.rename_item(idx);
                        }
                        menu_closed = true;
                    }

                    // Excluir (só se tiver item selecionado)
                    let can_delete = menu_state.item_index.is_some();
                    if ui
                        .add_enabled(can_delete, egui::Button::new("Excluir"))
                        .clicked()
                    {
                        ops.delete_with_shell();
                        menu_closed = true;
                    }
                }
            });
        });

    // Fecha o menu se uma ação foi executada
    if menu_closed {
        menu_state.close();
        return true;
    }

    // Fecha o menu se clicar fora (qualquer clique fora do menu)
    if ctx.input(|i| i.pointer.any_click()) {
        let pointer_pos = ctx.pointer_interact_pos();
        if let Some(pos) = pointer_pos {
            // Verifica se o clique foi fora do menu
            // O menu tem aproximadamente 180x200 pixels
            let menu_rect =
                egui::Rect::from_min_size(menu_state.position, egui::vec2(180.0, 200.0));
            if !menu_rect.contains(pos) {
                menu_state.close();
                return true;
            }
        } else {
            // Se não conseguiu obter a posição do ponteiro, fecha o menu por segurança
            menu_state.close();
            return true;
        }
    }

    false
}
