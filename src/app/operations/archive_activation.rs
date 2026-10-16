//! Resolve synthetic archive folders without filesystem probes or tab mutations
//! during rendering (especially while an inactive pane is swapped into the app).
use crate::app::{dual_panel::ActivePanel, ImageViewerApp};
use eframe::egui;

#[derive(Clone, Debug, PartialEq, Eq)]
struct ActivationSource {
    outer_tab: usize,
    inner_tab: Option<usize>,
    dual_panel: bool,
    panel: ActivePanel,
    path: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(panel: ActivePanel) -> ActivationSource {
        ActivationSource {
            outer_tab: 1,
            inner_tab: Some(2),
            dual_panel: true,
            panel,
            path: "D:\\Documents".into(),
        }
    }

    #[test]
    fn background_resolution_preserves_each_physical_pane_until_results_are_consumed() {
        let ctx = egui::Context::default();
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("sample.ZIP");
        std::fs::write(&archive, b"").unwrap();
        for panel in [ActivePanel::Left, ActivePanel::Right] {
            let origin = source(panel);
            let path = archive.to_string_lossy().into_owned();
            resolve_in_background(ctx.clone(), origin.clone(), path.clone())
                .join()
                .unwrap();
            let results = take_results(&ctx);
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].source, origin);
            assert_eq!(results[0].path, path);
            assert!(results[0].is_file);
            assert!(take_results(&ctx).is_empty());
        }
    }

    #[test]
    fn background_probe_distinguishes_real_folders_missing_files_and_archive_entries() {
        let ctx = egui::Context::default();
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("folder.zip");
        let archive = temp.path().join("sample.zip");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(&archive, b"").unwrap();
        for path in [
            folder,
            temp.path().join("missing.zip"),
            archive.join("inner.zip"),
        ] {
            resolve_in_background(
                ctx.clone(),
                source(ActivePanel::Right),
                path.to_string_lossy().into_owned(),
            )
            .join()
            .unwrap();
            let results = take_results(&ctx);
            assert_eq!(results.len(), 1);
            assert!(!results[0].is_file);
        }
    }
}

#[derive(Clone)]
struct ResolvedActivation {
    source: ActivationSource,
    path: String,
    is_file: bool,
}

fn results_id() -> egui::Id {
    egui::Id::new("resolved_archive_activations")
}

fn take_results(ctx: &egui::Context) -> Vec<ResolvedActivation> {
    ctx.data_mut(|data| {
        let results = data
            .get_temp::<Vec<ResolvedActivation>>(results_id())
            .unwrap_or_default();
        data.remove::<Vec<ResolvedActivation>>(results_id());
        results
    })
}

fn resolve_in_background(
    ctx: egui::Context,
    source: ActivationSource,
    path: String,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let is_file = std::path::Path::new(&path).is_file();
        ctx.data_mut(|data| {
            data.get_temp_mut_or_default::<Vec<ResolvedActivation>>(results_id())
                .push(ResolvedActivation {
                    source,
                    path,
                    is_file,
                });
        });
        ctx.request_repaint();
    })
}

impl ImageViewerApp {
    fn archive_activation_source(&self, panel: ActivePanel, path: String) -> ActivationSource {
        let tab = self.tab_manager.active();
        ActivationSource {
            outer_tab: tab.id,
            inner_tab: tab
                .dual_panel_tabs
                .as_ref()
                .and_then(|tabs| tabs.panel(panel).active())
                .map(|tab| tab.id),
            dual_panel: self.dual_panel_enabled,
            panel,
            path,
        }
    }

    pub(super) fn queue_archive_activation(&self, path: &str) {
        let panel = if self.in_inactive_panel_context {
            self.dual_panel_active.other()
        } else {
            self.dual_panel_active
        };
        let source =
            self.archive_activation_source(panel, self.navigation_state.current_path.clone());
        let _ = resolve_in_background(self.ui_ctx.clone(), source, path.to_owned());
    }

    /// Called after both panes finish rendering, outside temporary panel swaps.
    pub(crate) fn process_pending_archive_activations(&mut self) {
        for result in take_results(&self.ui_ctx) {
            if self.tab_navigation_blocked() || self.in_inactive_panel_context {
                continue;
            }
            let panel = result.source.panel;
            let current_path = if self.dual_panel_enabled && panel != self.dual_panel_active {
                self.dual_panel_inactive_state
                    .as_ref()
                    .map(|state| state.path.clone())
            } else {
                Some(self.navigation_state.current_path.clone())
            };
            let Some(current_path) = current_path else {
                continue;
            };
            if result.source != self.archive_activation_source(panel, current_path) {
                continue; // The user navigated or changed tabs while the probe was pending.
            }
            if result.is_file {
                self.open_location_in_new_tab(panel, &result.path);
            } else if self.dual_panel_enabled && panel != self.dual_panel_active {
                self.with_inactive_panel(|app| app.navigate_to(&result.path));
            } else {
                self.navigate_to(&result.path);
            }
        }
    }
}
