use crate::app::ImageViewerApp;
use crate::domain::folder_appearance::FolderAppearance;
use crate::infrastructure::folder_compose::FolderComposer;
use eframe::egui;

pub(crate) fn current(ctx: &egui::Context) -> FolderAppearance {
    ctx.data(|data| data.get_temp(egui::Id::new(FolderAppearance::PREFERENCE_KEY)))
        .unwrap_or_default()
}

pub(crate) fn remember(ctx: &egui::Context, appearance: FolderAppearance) {
    ctx.data_mut(|data| {
        data.insert_temp(egui::Id::new(FolderAppearance::PREFERENCE_KEY), appearance)
    });
}

impl ImageViewerApp {
    pub(crate) fn apply_folder_appearance(&mut self, ctx: &egui::Context) {
        remember(ctx, self.folder_appearance);
        self.cache_manager.folder_preview_requests.invalidate_all();
        self.cache_manager.folder_preview_cache.clear();
        self.cache_manager.clear_folder_preview_inflight_state();
        self.pending_folder_preview_replace.clear();
        self.suppress_next_folder_preview_invalidation.clear();
        if let Some(composer) = FolderComposer::try_new() {
            let (pixels, width, height) = composer
                .with_appearance(self.folder_appearance)
                .compose_empty();
            self.cache_manager
                .set_folder_icon(ctx, &pixels, width, height);
            self.item_icon_loader
                .set_folder_icon(ctx, &pixels, width, height);
        }
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::navigation_state::ThemeMode;

    #[test]
    fn folder_choice_is_independent_of_theme_changes() {
        let ctx = egui::Context::default();
        assert_eq!(current(&ctx), FolderAppearance::Original);
        for appearance in FolderAppearance::ALL {
            remember(&ctx, appearance);
            for theme in [
                ThemeMode::Light,
                ThemeMode::Dark,
                ThemeMode::DarkDeep,
                ThemeMode::GitHubDark,
                ThemeMode::System,
            ] {
                crate::ui::theme::apply_theme_visuals(&ctx, theme);
                assert_eq!(current(&ctx), appearance);
            }
        }
        remember(&ctx, FolderAppearance::Original);
        assert_eq!(current(&ctx), FolderAppearance::Original);
    }
}
