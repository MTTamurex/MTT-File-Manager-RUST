//! Apply transferred offsets after listings exist, not while an empty viewport
//! would clamp them to zero. Navigation invalidates this one-shot restoration.
use crate::app::{dual_panel::ActivePanel, ImageViewerApp};
use eframe::egui;

#[derive(Clone)]
struct ScrollRestore {
    workspace: usize,
    panel: ActivePanel,
    generation: usize,
    path: String,
    x: f32,
    y: f32,
}

#[derive(Debug, PartialEq)]
enum Readiness {
    Cancel,
    Wait,
    Ready,
}

impl ScrollRestore {
    fn readiness(
        &self,
        path: &str,
        generation: usize,
        loading: bool,
        rebuilding: bool,
        failed: bool,
    ) -> Readiness {
        if self.path != path || self.generation != generation || failed {
            Readiness::Cancel
        } else if loading || rebuilding {
            Readiness::Wait
        } else {
            Readiness::Ready
        }
    }

    fn poll(&self, app: &mut ImageViewerApp) -> bool {
        match self.readiness(
            &app.navigation_state.current_path,
            app.generation,
            app.is_loading_folder,
            app.pending_items_rebuild
                || app.items_rebuild_in_flight
                || app.inactive_final_items_rebuild_pending,
            app.folder_load_error.is_some(),
        ) {
            Readiness::Wait => true,
            Readiness::Cancel => false,
            Readiness::Ready => {
                app.scroll_offset_x = self.x;
                app.scroll_offset_y = self.y;
                false
            }
        }
    }
}

fn state_id() -> egui::Id {
    egui::Id::new("transferred_scroll_restore")
}

pub(super) fn remember(app: &ImageViewerApp) {
    let workspace = app.tab_manager.active().id;
    let mut pending = vec![ScrollRestore {
        workspace,
        panel: app.dual_panel_active,
        generation: app.generation,
        path: app.navigation_state.current_path.clone(),
        x: app.scroll_offset_x,
        y: app.scroll_offset_y,
    }];
    if app.dual_panel_enabled {
        if let Some(inactive) = &app.dual_panel_inactive_state {
            pending.push(ScrollRestore {
                workspace,
                panel: app.dual_panel_active.other(),
                generation: inactive.generation,
                path: inactive.path.clone(),
                x: inactive.scroll_offset_x,
                y: inactive.scroll_offset_y,
            });
        }
    }
    pending.retain(|restore| restore.x != 0.0 || restore.y != 0.0);
    if !pending.is_empty() {
        app.ui_ctx
            .data_mut(|data| data.insert_temp(state_id(), pending));
    }
}

pub(crate) fn restore_ready_scroll(app: &mut ImageViewerApp) {
    let ctx = app.ui_ctx.clone();
    let Some(mut pending) = ctx.data_mut(|data| data.remove_temp::<Vec<ScrollRestore>>(state_id()))
    else {
        return;
    };
    if ctx.input(|input| {
        input.pointer.any_pressed()
            || input.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::Key { pressed: true, .. } | egui::Event::MouseWheel { .. }
                )
            })
    }) {
        // Explicit input in the receiving window takes precedence over its
        // transferred position; do not jump back when a slow scan finishes.
        return;
    }
    pending.retain(|restore| {
        if app.tab_manager.active().id != restore.workspace {
            false
        } else if app.dual_panel_active == restore.panel {
            restore.poll(app)
        } else if app.dual_panel_enabled {
            app.with_inactive_panel(|app| restore.poll(app))
                .unwrap_or(false)
        } else {
            false
        }
    });
    if !pending.is_empty() {
        ctx.data_mut(|data| data.insert_temp(state_id(), pending));
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_waits_for_the_complete_listing_and_its_final_rebuild() {
        let restore = ScrollRestore {
            workspace: 0,
            panel: ActivePanel::Left,
            generation: 1,
            path: "C:\\".to_owned(),
            x: 0.0,
            y: 100.0,
        };
        assert_eq!(
            restore.readiness("C:\\", 1, true, false, false),
            Readiness::Wait
        );
        assert_eq!(
            restore.readiness("C:\\", 1, false, true, false),
            Readiness::Wait
        );
        assert_eq!(
            restore.readiness("C:\\", 1, false, false, false),
            Readiness::Ready
        );
        assert_eq!(
            restore.readiness("D:\\", 1, false, false, false),
            Readiness::Cancel
        );
        assert_eq!(
            restore.readiness("C:\\", 2, false, false, false),
            Readiness::Cancel
        );
        assert_eq!(
            restore.readiness("C:\\", 1, false, false, true),
            Readiness::Cancel
        );
    }
}
