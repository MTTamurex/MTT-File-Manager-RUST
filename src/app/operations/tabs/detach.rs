use crate::app::{dual_panel::ActivePanel, ImageViewerApp};
use crate::infrastructure::windows::app_instances::{
    allows_multiple_instances, ALLOW_MULTIPLE_INSTANCES_KEY,
};
use crate::tabs::TabTransfer;
use eframe::egui;

#[derive(Clone)]
enum Source {
    Workspace(usize),
    Panel {
        workspace: usize,
        panel: ActivePanel,
        tab: usize,
    },
}

#[derive(Clone)]
struct Pending {
    source: Source,
    result: crossbeam_channel::Receiver<bool>,
}

fn pending_id() -> egui::Id {
    egui::Id::new("pending_tab_transfer")
}

pub(crate) fn transfer_pending(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<Pending>(pending_id()))
        .is_some()
}

pub(crate) fn render_transfer_progress(ctx: &egui::Context) {
    if transfer_pending(ctx) {
        egui::Modal::new(pending_id()).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(rust_i18n::t!("tabs.detaching"));
            });
        });
    }
}

impl ImageViewerApp {
    fn tab_detach_enabled(&self) -> bool {
        self.ui_ctx
            .data(|data| data.get_temp::<Pending>(pending_id()))
            .is_none()
            && allows_multiple_instances(
                self.app_state_db
                    .get_preference(ALLOW_MULTIPLE_INSTANCES_KEY)
                    .as_deref(),
            )
    }

    pub(crate) fn detach_main_tab(&mut self, index: usize) {
        if !self.tab_detach_enabled() || !self.tab_manager.can_detach_tab(index) {
            return;
        }
        self.sync_to_tab();
        let Some(tab) = self.tab_manager.tabs.get(index) else {
            return;
        };
        let source = Source::Workspace(tab.id);
        let session = TabTransfer::from_tab(tab);
        self.begin_tab_transfer(source, session);
    }

    pub(crate) fn detach_panel_tab(&mut self, panel: ActivePanel, index: usize) {
        if !self.tab_detach_enabled() || !self.dual_panel_enabled {
            return;
        }
        self.sync_to_tab();
        let workspace = self.tab_manager.active();
        let Some(tabs) = workspace.dual_panel_tabs.as_ref() else {
            return;
        };
        let Some(tab) = tabs.panel(panel).tabs.get(index) else {
            return;
        };
        if tabs.count() <= 1 && self.tab_manager.tabs.len() <= 1 {
            return;
        }
        let source = Source::Panel {
            workspace: workspace.id,
            panel,
            tab: tab.id,
        };
        let session = TabTransfer::from_panel(&tab.snapshot, tab.pinned, workspace);
        self.begin_tab_transfer(source, session);
    }

    fn begin_tab_transfer(&mut self, source: Source, session: TabTransfer) {
        let (sender, result) = crossbeam_channel::bounded(1);
        let ctx = self.ui_ctx.clone();
        if crate::spawn_named("tab-transfer", move || {
            let result = crate::app::tab_transfer::launch(&session);
            if let Err(error) = &result {
                log::warn!(
                    "[TAB-TRANSFER] Transfer failed kind={:?} os_code={:?}",
                    error.kind(),
                    error.raw_os_error(),
                );
            }
            let success = result.is_ok();
            let _ = sender.send(success);
            ctx.request_repaint();
        })
        .is_ok()
        {
            self.ui_ctx
                .data_mut(|data| data.insert_temp(pending_id(), Pending { source, result }));
        } else {
            self.notifications
                .warning(rust_i18n::t!("tabs.detach_failed").to_string());
        }
    }

    pub(crate) fn poll_tab_transfer(&mut self, ctx: &egui::Context) {
        let Some(pending) = ctx.data(|data| data.get_temp::<Pending>(pending_id())) else {
            return;
        };
        match pending.result.try_recv() {
            Ok(true) => {
                ctx.data_mut(|data| data.remove::<Pending>(pending_id()));
                self.finish_tab_transfer(pending.source);
            }
            Ok(false) | Err(crossbeam_channel::TryRecvError::Disconnected) => {
                ctx.data_mut(|data| data.remove::<Pending>(pending_id()));
                self.notifications
                    .warning(rust_i18n::t!("tabs.detach_failed").to_string());
            }
            Err(crossbeam_channel::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
        }
    }

    fn finish_tab_transfer(&mut self, source: Source) {
        self.sync_to_tab();
        let workspace_id = match source {
            Source::Workspace(id) => id,
            Source::Panel { workspace, .. } => workspace,
        };
        let Some(index) = self
            .tab_manager
            .tabs
            .iter()
            .position(|tab| tab.id == workspace_id)
        else {
            return;
        };
        match source {
            Source::Workspace(_) => {
                if self.tab_manager.tabs.len() > 1 {
                    self.tab_manager.tabs[index].pinned = false;
                    self.close_main_tab(index);
                }
            }
            Source::Panel { panel, tab, .. } => {
                self.tab_manager.switch_to(index);
                self.sync_from_tab();
                let Some(tabs) = self.tab_manager.active_mut().dual_panel_tabs.as_mut() else {
                    return;
                };
                let Some(index) = tabs
                    .panel(panel)
                    .tabs
                    .iter()
                    .position(|entry| entry.id == tab)
                else {
                    return;
                };
                tabs.panel_mut(panel).tabs[index].pinned = false;
                let last_in_panel = tabs.panel(panel).tabs.len() == 1;
                self.dual_panel_close_tab(panel, index);
                if last_in_panel {
                    // Split-mode restoration requires two nonempty stored sets.
                    // Reset only the now-dormant side, never retain the moved tab
                    // for a later re-enable of split mode.
                    let snapshot =
                        self.new_panel_tab_snapshot(crate::domain::special_paths::COMPUTER_VIEW_ID);
                    if let Some(tabs) = self.tab_manager.active_mut().dual_panel_tabs.as_mut() {
                        if let Some(entry) = tabs.panel_mut(panel).tabs.first_mut() {
                            entry.snapshot = snapshot;
                            entry.pinned = false;
                        }
                    }
                }
                self.sync_to_tab();
                self.save_preferences();
            }
        }
    }
}
