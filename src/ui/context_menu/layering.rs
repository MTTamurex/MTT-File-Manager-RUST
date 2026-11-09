//! Keep search menus above their modal even when the modal is brought to front.
use crate::application::context_menu::ContextMenuOrigin;
use eframe::egui;

pub(super) fn attach_menu(ctx: &egui::Context, menu_id: egui::Id, origin: ContextMenuOrigin) {
    if origin == ContextMenuOrigin::GlobalSearch {
        ctx.set_sublayer(
            crate::ui::global_search_overlay::modal_layer_id(),
            egui::LayerId::new(egui::Order::Foreground, menu_id),
        );
    }
}

pub(super) fn attach_submenu(ctx: &egui::Context, owner: egui::LayerId, submenu_id: egui::Id) {
    let modal = crate::ui::global_search_overlay::modal_layer_id();
    if ctx.memory(|memory| memory.areas().parent_layer(owner)) == Some(modal) {
        // egui supports only one level of sublayers. All search menus therefore
        // share the modal as their parent, including deeply nested submenus.
        ctx.set_sublayer(
            modal,
            egui::LayerId::new(egui::Order::Foreground, submenu_id),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_menu_and_nested_submenus_stay_above_refocused_modal() {
        let ctx = egui::Context::default();
        let modal = crate::ui::global_search_overlay::modal_layer_id();
        let menu = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("test_menu"));
        let submenu = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("test_submenu"));
        let nested = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("test_nested"));
        for reopened in 0..3 {
            for _ in 0..4 {
                let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                    let ctx = ui.ctx();
                    for (layer, position) in [
                        (modal, egui::pos2(20.0, 20.0)),
                        (menu, egui::pos2(40.0, 40.0)),
                        (submenu, egui::pos2(160.0, 40.0)),
                        (nested, egui::pos2(280.0, 40.0)),
                    ] {
                        if reopened == 1 && layer != modal {
                            continue;
                        }
                        if layer == menu {
                            attach_menu(ctx, menu.id, ContextMenuOrigin::GlobalSearch);
                        }
                        if layer == submenu {
                            attach_submenu(ctx, menu, submenu.id);
                        }
                        if layer == nested {
                            attach_submenu(ctx, submenu, nested.id);
                        }
                        egui::Area::new(layer.id)
                            .order(layer.order)
                            .fixed_pos(position)
                            .show(ctx, |ui| {
                                ui.allocate_exact_size(
                                    egui::vec2(if layer == modal { 500.0 } else { 100.0 }, 200.0),
                                    egui::Sense::click(),
                                );
                            });
                    }
                    // Reproduce the modal taking front priority on interaction.
                    ctx.move_to_top(modal);
                });
            }
            if reopened != 1 {
                for (layer, point) in [
                    (menu, egui::pos2(80.0, 80.0)),
                    (submenu, egui::pos2(200.0, 80.0)),
                    (nested, egui::pos2(320.0, 80.0)),
                ] {
                    assert_eq!(ctx.layer_id_at(point), Some(layer));
                }
            }
        }
    }
}
