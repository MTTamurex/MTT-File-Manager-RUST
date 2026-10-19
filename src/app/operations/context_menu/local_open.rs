//! App-only folder action; independent of Windows Shell registrations.

use crate::application::context_menu::{ContextMenuItem, ContextMenuState, OPEN_IN_MTT_ID};
use rust_i18n::t;

fn app_icon(ctx: &eframe::egui::Context) -> Option<eframe::egui::TextureHandle> {
    use eframe::egui;

    let key = egui::Id::new("mtt_context_menu_icon");
    if let Some(icon) = ctx.data_mut(|data| data.get_temp::<egui::TextureHandle>(key)) {
        return Some(icon);
    }
    let image = image::load_from_memory(crate::embedded_assets::APP_ICON_256_PNG)
        .ok()?
        .resize_exact(32, 32, image::imageops::FilterType::Lanczos3)
        .to_rgba8();
    let icon = ctx.load_texture(
        "mtt_context_menu_icon",
        egui::ColorImage::from_rgba_unmultiplied([32, 32], image.as_raw()),
        egui::TextureOptions::LINEAR,
    );
    ctx.data_mut(|data| data.insert_temp(key, icon.clone()));
    Some(icon)
}

pub(super) fn ensure_more_options_item(menu: &mut ContextMenuState, ctx: &eframe::egui::Context) {
    if menu.is_empty_area
        || menu.primary_is_directory != Some(true)
        || menu.target_paths.len() != 1
        || !menu.items.iter().any(|item| item.id == -20)
    {
        return;
    }

    let mut item = ContextMenuItem::new(OPEN_IN_MTT_ID, t!("context_menu.open_in_mtt"))
        .with_command("open_in_mtt");
    if let Some(icon) = app_icon(ctx) {
        item.icon = Some(icon);
    } else {
        item.svg_icon_name = Some("folder".to_string());
    }
    if let Some(more) = menu.items.iter_mut().find(|item| item.id == -99) {
        if !more.sub_items.iter().any(|item| item.id == OPEN_IN_MTT_ID) {
            more.sub_items.insert(0, item);
        }
    } else if let Some(index) = menu.items.iter().position(|item| item.id == -200) {
        // Keep the local action usable while third-party Shell items are loading.
        let mut loading = menu.items.remove(index);
        loading.text = t!("context_menu.loading").to_string();
        menu.items.insert(
            index,
            ContextMenuItem::new(-99, t!("context_menu.show_more"))
                .with_subitems(vec![item, loading]),
        );
    } else {
        if menu.items.last().is_some_and(|item| !item.is_separator) {
            menu.items.push(ContextMenuItem::separator());
        }
        menu.items.push(
            ContextMenuItem::new(-99, t!("context_menu.show_more")).with_subitems(vec![item]),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn ensure_more_options_item(menu: &mut ContextMenuState) {
        super::ensure_more_options_item(menu, &eframe::egui::Context::default());
    }

    fn folder_menu() -> ContextMenuState {
        let mut menu = ContextMenuState::new();
        menu.target_paths = vec![PathBuf::from(r"C:\Example\Folder")];
        menu.primary_is_directory = Some(true);
        menu.items = vec![ContextMenuItem::new(-20, "Open")];
        menu
    }

    fn local_item(menu: &ContextMenuState) -> &ContextMenuItem {
        let more = menu.items.iter().find(|item| item.id == -99).unwrap();
        more.sub_items
            .iter()
            .find(|item| item.id == OPEN_IN_MTT_ID)
            .unwrap()
    }

    #[test]
    fn adds_a_localized_internal_action_only_inside_more_options() {
        let mut menu = folder_menu();
        ensure_more_options_item(&mut menu);
        let item = local_item(&menu);
        assert_eq!(item.text, t!("context_menu.open_in_mtt").to_string());
        assert_eq!(item.command_string.as_deref(), Some("open_in_mtt"));
        assert!(item.svg_icon_name.is_none());
        assert!(item.icon.is_some());
        assert!(item.is_enabled);
        assert!(!menu.items.iter().any(|item| item.id == OPEN_IN_MTT_ID));
    }

    #[test]
    fn caches_the_embedded_app_icon_in_the_ui_context() {
        let ctx = eframe::egui::Context::default();
        let first = app_icon(&ctx).unwrap();
        let second = app_icon(&ctx).unwrap();
        assert_eq!(first.id(), second.id());
        assert_eq!(first.size(), [32, 32]);
    }

    #[test]
    fn preserves_shell_items_without_duplicating_the_local_action() {
        let mut menu = folder_menu();
        menu.items.push(
            ContextMenuItem::new(-99, "More options")
                .with_subitems(vec![ContextMenuItem::new(10, "Example Shell command")]),
        );
        ensure_more_options_item(&mut menu);
        ensure_more_options_item(&mut menu);
        let more = menu.items.iter().find(|item| item.id == -99).unwrap();
        assert_eq!(more.sub_items.len(), 2);
        assert_eq!(more.sub_items[0].id, OPEN_IN_MTT_ID);
        assert_eq!(more.sub_items[1].id, 10);
    }

    #[test]
    fn local_action_is_enabled_while_the_shell_is_still_loading() {
        let mut menu = folder_menu();
        menu.items.push(ContextMenuItem {
            id: -200,
            is_enabled: false,
            is_loading_placeholder: true,
            ..Default::default()
        });
        ensure_more_options_item(&mut menu);
        let more = menu.items.iter().find(|item| item.id == -99).unwrap();
        assert!(more.is_enabled);
        assert_eq!(more.sub_items[0].id, OPEN_IN_MTT_ID);
        assert!(more.sub_items[1].is_loading_placeholder);
        assert_eq!(menu.items.iter().filter(|item| item.id == -99).count(), 1);
    }

    #[test]
    fn survives_rebuilding_more_options_after_an_empty_shell_result() {
        let mut menu = folder_menu();
        ensure_more_options_item(&mut menu);
        menu.items.retain(|item| item.id != -99);
        while menu.items.last().is_some_and(|item| item.is_separator) {
            menu.items.pop();
        }
        ensure_more_options_item(&mut menu);
        assert!(local_item(&menu).is_enabled);
        assert_eq!(menu.items.iter().filter(|item| item.id == -99).count(), 1);
    }

    #[test]
    fn excludes_files_background_unknown_targets_and_multiple_selections() {
        let mut variants = vec![
            folder_menu(),
            folder_menu(),
            folder_menu(),
            folder_menu(),
            folder_menu(),
        ];
        variants[0].primary_is_directory = Some(false);
        variants[1].is_empty_area = true;
        variants[2].primary_is_directory = None;
        variants[3]
            .target_paths
            .push(PathBuf::from(r"C:\Example\Other Folder"));
        variants[4].items.clear();
        for mut menu in variants {
            ensure_more_options_item(&mut menu);
            assert!(!menu.items.iter().any(|item| item.id == -99));
        }
    }
}
