use crate::app::{dual_panel::ActivePanel, state::ImageViewerApp};
use crate::infrastructure::windows::system_tray::{
    coordinator,
    model::{Entry, LocationKind, Snapshot, Target, Workspace},
};
use eframe::egui;
use std::time::{Duration, Instant};

impl ImageViewerApp {
    pub(super) fn publish_tray_snapshot(&self, force: bool) {
        if self.native_hwnd.is_none() {
            return;
        }
        let key = egui::Id::new("tray_snapshot_updated");
        let now = Instant::now();
        if !force
            && self
                .ui_ctx
                .data(|data| data.get_temp::<Instant>(key))
                .is_some_and(|last| now.duration_since(last) < Duration::from_millis(250))
        {
            return;
        }
        self.ui_ctx.data_mut(|data| data.insert_temp(key, now));
        coordinator::publish(self.tray_snapshot());
    }

    fn tray_snapshot(&self) -> Snapshot {
        let display_path =
            |path: &str| display_path(path, self.tag_view_display_name_for_path(path));
        let active = self.tab_manager.active().id;
        let workspaces = self
            .tab_manager
            .tabs
            .iter()
            .map(|tab| {
                let is_active = tab.id == active;
                let path = if is_active {
                    &self.navigation_state.current_path
                } else {
                    &tab.path
                };
                let dual = if is_active {
                    self.dual_panel_enabled
                } else {
                    tab.dual_panel_enabled
                };
                let panels = tab.dual_panel_tabs.as_ref().filter(|_| dual).map(|panels| {
                    [ActivePanel::Left, ActivePanel::Right].map(|panel| {
                        let set = panels.panel(panel);
                        set.tabs
                            .iter()
                            .enumerate()
                            .map(|(index, entry)| {
                                let path = if is_active && index == set.active_tab {
                                    if panel == self.dual_panel_active {
                                        &self.navigation_state.current_path
                                    } else {
                                        self.dual_panel_inactive_state
                                            .as_ref()
                                            .map(|s| &s.path)
                                            .unwrap_or(&entry.snapshot.path)
                                    }
                                } else {
                                    &entry.snapshot.path
                                };
                                Entry {
                                    id: entry.id,
                                    path: display_path(path),
                                    icon: LocationKind::for_path(path),
                                }
                            })
                            .collect()
                    })
                });
                Workspace {
                    tab: Entry {
                        id: tab.id,
                        path: display_path(path),
                        icon: LocationKind::for_path(path),
                    },
                    panels,
                }
            })
            .collect();
        Snapshot {
            workspaces,
            active_path: display_path(&self.navigation_state.current_path),
            active_icon: LocationKind::for_path(&self.navigation_state.current_path),
            hidden_at: None,
        }
    }

    pub(super) fn activate_tray_target(&mut self, target: &Target) {
        // Validate the complete target before changing any active view. The
        // native menu is a snapshot and its tab may have closed in the meantime.
        let Some(resolved) = self.tray_snapshot().resolve(target) else {
            return;
        };
        let index = resolved.workspace_index;
        if self.tab_manager.active_tab != index {
            self.sync_to_tab();
            self.tab_manager.switch_to(index);
            self.sync_from_tab();
        }
        if let Some((right, index)) = resolved.panel {
            let panel = if right {
                ActivePanel::Right
            } else {
                ActivePanel::Left
            };
            self.dual_panel_select_tab(panel, index);
        }
        self.update_video_visibility();
        self.save_preferences();
        self.publish_tray_snapshot(true);
    }
}

fn display_path(path: &str, tag_label: Option<String>) -> String {
    match path {
        crate::domain::special_paths::COMPUTER_VIEW_ID => {
            rust_i18n::t!("nav.computer").into_owned()
        }
        crate::domain::special_paths::RECYCLE_BIN_VIEW_ID => {
            rust_i18n::t!("nav.recycle_bin").into_owned()
        }
        _ => tag_label.unwrap_or_else(|| path.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_virtual_locations_use_navigation_labels() {
        assert_eq!(
            display_path(crate::domain::special_paths::COMPUTER_VIEW_ID, None),
            rust_i18n::t!("nav.computer")
        );
        assert_eq!(
            display_path(crate::domain::special_paths::RECYCLE_BIN_VIEW_ID, None),
            rust_i18n::t!("nav.recycle_bin")
        );
        assert_eq!(display_path("C:\\Documents", None), "C:\\Documents");
    }

    #[test]
    fn tray_tag_locations_use_resolved_names_instead_of_internal_paths() {
        let path = crate::domain::special_paths::tag_view_path(42);
        let label = rust_i18n::t!("tags.filter_active", name = "Example").into_owned();
        assert_eq!(display_path(&path, Some(label.clone())), label);
        let renamed = rust_i18n::t!("tags.filter_active", name = "Renamed").into_owned();
        assert_eq!(display_path(&path, Some(renamed.clone())), renamed);
    }
}
