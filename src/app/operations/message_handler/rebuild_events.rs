use crate::app::dual_panel::{ActivePanel, PanelSnapshot};
use crate::app::state::{ImageViewerApp, InactiveItemsRebuildResult, ItemsRebuildSignature};
use eframe::egui;
use std::collections::VecDeque;
use std::sync::mpsc::TryRecvError;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PhysicalPanelRoute {
    Current,
    InactiveSnapshot,
    Missing,
}

fn physical_panel_route(
    target: ActivePanel,
    active: ActivePanel,
    dual_panel_enabled: bool,
    has_inactive_snapshot: bool,
) -> PhysicalPanelRoute {
    if !dual_panel_enabled {
        PhysicalPanelRoute::Missing
    } else if target == active {
        PhysicalPanelRoute::Current
    } else if has_inactive_snapshot {
        PhysicalPanelRoute::InactiveSnapshot
    } else {
        PhysicalPanelRoute::Missing
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InactiveRebuildDecision {
    Apply,
    Retry,
    Cancel,
    Discard,
}

fn inactive_rebuild_decision(
    result_generation: usize,
    result_signature: &ItemsRebuildSignature,
    panel_generation: usize,
    panel_signature: &ItemsRebuildSignature,
    is_loading_folder: bool,
    pending_items_rebuild: bool,
    newer_rebuild_pending: bool,
) -> InactiveRebuildDecision {
    if newer_rebuild_pending {
        return InactiveRebuildDecision::Retry;
    }
    if result_generation != panel_generation {
        return InactiveRebuildDecision::Discard;
    }
    if is_loading_folder || !pending_items_rebuild {
        return InactiveRebuildDecision::Cancel;
    }
    if result_signature == panel_signature {
        InactiveRebuildDecision::Apply
    } else {
        InactiveRebuildDecision::Retry
    }
}

fn defer_inactive_rebuild_result(
    deferred: &mut VecDeque<InactiveItemsRebuildResult>,
    result: InactiveItemsRebuildResult,
    live_tab_ids: &[usize],
) -> bool {
    deferred.retain(|queued| live_tab_ids.contains(&queued.tab_id));
    if !live_tab_ids.contains(&result.tab_id) {
        return false;
    }

    if let Some(index) = deferred
        .iter()
        .position(|queued| queued.tab_id == result.tab_id && queued.panel == result.panel)
    {
        if deferred[index].generation <= result.generation {
            deferred[index] = result;
        }
    } else {
        deferred.push_back(result);
    }

    debug_assert!(deferred.len() <= live_tab_ids.len().saturating_mul(2));
    true
}

fn rebuild_drain_allows_next(
    processed: usize,
    elapsed: Duration,
    max_messages: usize,
    budget: Duration,
) -> bool {
    processed < max_messages && elapsed < budget
}

fn take_deferred_rebuild_results_for_tab(
    deferred: &mut VecDeque<InactiveItemsRebuildResult>,
    tab_id: usize,
) -> Vec<InactiveItemsRebuildResult> {
    let mut ready = Vec::new();
    let mut index = 0;
    while index < deferred.len() {
        if deferred[index].tab_id == tab_id {
            if let Some(result) = deferred.remove(index) {
                ready.push(result);
            }
        } else {
            index += 1;
        }
    }
    ready
}

fn snapshot_items_rebuild_signature(
    snapshot: &PanelSnapshot,
    tag_assignments_epoch: u64,
) -> ItemsRebuildSignature {
    ItemsRebuildSignature {
        items_revision: snapshot.items_revision,
        search_query: snapshot.search_query.clone(),
        active_tag_filter: snapshot.active_tag_filter,
        tag_assignments_epoch,
        sort_mode: snapshot.sort_mode,
        sort_descending: snapshot.sort_descending,
        folders_position: snapshot.folders_position,
        group_mode: snapshot.group_mode,
        group_descending: snapshot.group_descending,
        path: snapshot.path.clone(),
        is_computer_view: snapshot.is_computer_view,
        is_recycle_bin_view: snapshot.is_recycle_bin_view,
    }
}

impl ImageViewerApp {
    pub(super) fn process_items_rebuild_results(&mut self, ctx: &egui::Context) {
        const MAX_REBUILD_MSGS_PER_FRAME: usize = 24;
        let rebuild_budget = if self.frame_time_peak_ms > 33.33 {
            Duration::from_millis(1)
        } else if self.frame_time_peak_ms > 25.0 {
            Duration::from_millis(2)
        } else {
            Duration::from_millis(3)
        };

        let start = Instant::now();
        let mut processed_messages = 0usize;
        let mut has_more = false;
        let mut latest_valid = None;
        let mut stale_current_request = false;

        while processed_messages < MAX_REBUILD_MSGS_PER_FRAME {
            if start.elapsed() >= rebuild_budget {
                has_more = true;
                break;
            }

            match self.items_rebuild_receiver.try_recv() {
                Ok(result) => {
                    processed_messages += 1;
                    if result.generation == self.generation
                        && result.request_id == self.items_rebuild_request_id
                    {
                        if result.signature == self.current_items_rebuild_signature() {
                            // Keep only the most recent valid rebuild for this frame.
                            latest_valid = Some(result);
                        } else {
                            stale_current_request = true;
                        }
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }

        if processed_messages >= MAX_REBUILD_MSGS_PER_FRAME {
            has_more = true;
        }

        if let Some(result) = latest_valid {
            self.items_rebuild_in_flight = false;
            self.items = Arc::new(result.items);
            self.total_items = result.total_items;
            self.group_projection = Arc::new(result.group_projection);
            self.hold_visible_items_until_load_complete = false;

            // After rebuild: if a pending selection was requested (e.g., after rename),
            // find the item and select + scroll to it.
            if let Some(target_path) = self.pending_select_path.take() {
                let _ = self.select_item_by_path(&target_path);
            } else {
                self.reconcile_visible_selection_index();
            }

            if self.pending_items_rebuild {
                self.maybe_schedule_stream_items_rebuild(ctx);
            } else {
                ctx.request_repaint();
            }
        } else if stale_current_request {
            self.items_rebuild_in_flight = false;
            self.pending_items_rebuild = true;
            self.pending_items_count = usize::MAX;
            self.maybe_schedule_stream_items_rebuild(ctx);
        } else if has_more {
            ctx.request_repaint();
        }
    }

    fn apply_inactive_items_rebuild_result(&mut self, result: InactiveItemsRebuildResult) {
        self.items = Arc::new(result.items);
        self.total_items = result.total_items;
        self.group_projection = Arc::new(result.group_projection);
        self.pending_items_rebuild = false;
        self.pending_items_count = 0;
        self.inactive_final_items_rebuild_pending = false;
        self.hold_visible_items_until_load_complete = false;
        self.reconcile_visible_selection_index();
    }

    fn process_inactive_items_rebuild_result(
        &mut self,
        result: InactiveItemsRebuildResult,
        ctx: &egui::Context,
    ) {
        let route = physical_panel_route(
            result.panel,
            self.dual_panel_active,
            self.dual_panel_enabled,
            self.dual_panel_inactive_state.is_some(),
        );

        match route {
            PhysicalPanelRoute::Current => {
                let signature = self.current_items_rebuild_signature();
                match inactive_rebuild_decision(
                    result.generation,
                    &result.signature,
                    self.generation,
                    &signature,
                    self.is_loading_folder,
                    self.pending_items_rebuild,
                    self.inactive_final_items_rebuild_pending,
                ) {
                    InactiveRebuildDecision::Apply => {
                        self.apply_inactive_items_rebuild_result(result);
                        ctx.request_repaint();
                    }
                    InactiveRebuildDecision::Retry => {
                        self.spawn_inactive_final_items_rebuild_job(result.tab_id, result.panel);
                    }
                    InactiveRebuildDecision::Cancel => {
                        self.inactive_final_items_rebuild_pending = false;
                    }
                    InactiveRebuildDecision::Discard => {}
                }
            }
            PhysicalPanelRoute::InactiveSnapshot => {
                let decision = self
                    .dual_panel_inactive_state
                    .as_ref()
                    .map(|panel| {
                        let signature =
                            snapshot_items_rebuild_signature(panel, self.tag_assignments_epoch);
                        inactive_rebuild_decision(
                            result.generation,
                            &result.signature,
                            panel.generation,
                            &signature,
                            panel.is_loading_folder,
                            panel.pending_items_rebuild,
                            panel.inactive_final_items_rebuild_pending,
                        )
                    })
                    .unwrap_or(InactiveRebuildDecision::Discard);
                match decision {
                    InactiveRebuildDecision::Apply => {
                        self.with_inactive_panel(|app| {
                            app.apply_inactive_items_rebuild_result(result);
                        });
                        ctx.request_repaint();
                    }
                    InactiveRebuildDecision::Retry => {
                        let tab_id = result.tab_id;
                        let panel = result.panel;
                        self.with_inactive_panel(|app| {
                            app.spawn_inactive_final_items_rebuild_job(tab_id, panel);
                        });
                    }
                    InactiveRebuildDecision::Cancel => {
                        if let Some(panel) = self.dual_panel_inactive_state.as_mut() {
                            panel.inactive_final_items_rebuild_pending = false;
                        }
                    }
                    InactiveRebuildDecision::Discard => {}
                }
            }
            PhysicalPanelRoute::Missing => {}
        }
    }

    pub(super) fn process_inactive_items_rebuild_results(&mut self, ctx: &egui::Context) {
        const MAX_INACTIVE_REBUILD_MSGS_PER_FRAME: usize = 4;
        const INACTIVE_REBUILD_DRAIN_BUDGET: Duration = Duration::from_millis(2);

        let live_tab_ids: Vec<usize> = self.tab_manager.tabs.iter().map(|tab| tab.id).collect();
        self.deferred_inactive_items_rebuild_results
            .retain(|result| live_tab_ids.contains(&result.tab_id));

        let drain_started = Instant::now();
        let mut drained = 0;
        while rebuild_drain_allows_next(
            drained,
            drain_started.elapsed(),
            MAX_INACTIVE_REBUILD_MSGS_PER_FRAME,
            INACTIVE_REBUILD_DRAIN_BUDGET,
        ) {
            let result = match self.inactive_items_rebuild_receiver.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            };
            drained += 1;
            let tab_id = result.tab_id;
            let panel = result.panel;
            self.inactive_items_rebuild_registry.finish(tab_id, panel);
            defer_inactive_rebuild_result(
                &mut self.deferred_inactive_items_rebuild_results,
                result,
                &live_tab_ids,
            );
        }
        if !rebuild_drain_allows_next(
            drained,
            drain_started.elapsed(),
            MAX_INACTIVE_REBUILD_MSGS_PER_FRAME,
            INACTIVE_REBUILD_DRAIN_BUDGET,
        ) {
            ctx.request_repaint();
        }

        let active_tab_id = self.tab_manager.active().id;
        let ready = take_deferred_rebuild_results_for_tab(
            &mut self.deferred_inactive_items_rebuild_results,
            active_tab_id,
        );
        for result in ready {
            self.process_inactive_items_rebuild_result(result, ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::file_entry::{FoldersPosition, GroupMode, SortMode};

    fn signature(items_revision: u64) -> ItemsRebuildSignature {
        ItemsRebuildSignature {
            items_revision,
            search_query: String::new(),
            active_tag_filter: None,
            tag_assignments_epoch: 4,
            sort_mode: SortMode::Name,
            sort_descending: false,
            folders_position: FoldersPosition::First,
            group_mode: GroupMode::None,
            group_descending: false,
            path: r"C:\items".to_string(),
            is_computer_view: false,
            is_recycle_bin_view: false,
        }
    }

    fn result(
        tab_id: usize,
        panel: ActivePanel,
        items_revision: u64,
    ) -> InactiveItemsRebuildResult {
        InactiveItemsRebuildResult {
            tab_id,
            panel,
            generation: 7,
            items: Vec::new(),
            total_items: 0,
            group_projection: Default::default(),
            signature: signature(items_revision),
        }
    }

    #[test]
    fn physical_result_routes_to_same_panel_after_focus_switch() {
        assert_eq!(
            physical_panel_route(ActivePanel::Right, ActivePanel::Right, true, true),
            PhysicalPanelRoute::Current
        );
        assert_eq!(
            physical_panel_route(ActivePanel::Right, ActivePanel::Left, true, true),
            PhysicalPanelRoute::InactiveSnapshot
        );
    }

    #[test]
    fn physical_result_is_missing_when_its_inactive_panel_was_closed() {
        assert_eq!(
            physical_panel_route(ActivePanel::Right, ActivePanel::Left, false, false),
            PhysicalPanelRoute::Missing
        );
        assert_eq!(
            physical_panel_route(ActivePanel::Left, ActivePanel::Left, false, false),
            PhysicalPanelRoute::Missing
        );
    }

    #[test]
    fn final_result_decision_accepts_its_own_pending_state() {
        let signature = signature(9);
        assert_eq!(
            inactive_rebuild_decision(7, &signature, 7, &signature, false, true, false,),
            InactiveRebuildDecision::Apply
        );
    }

    #[test]
    fn final_result_decision_retries_only_stale_current_pending_result() {
        let result_signature = signature(9);
        let stale_panel_signature = signature(10);
        assert_eq!(
            inactive_rebuild_decision(
                7,
                &result_signature,
                7,
                &stale_panel_signature,
                false,
                true,
                false,
            ),
            InactiveRebuildDecision::Retry
        );
        assert_eq!(
            inactive_rebuild_decision(
                7,
                &result_signature,
                8,
                &result_signature,
                false,
                true,
                false,
            ),
            InactiveRebuildDecision::Discard
        );
        assert_eq!(
            inactive_rebuild_decision(
                7,
                &result_signature,
                7,
                &result_signature,
                true,
                true,
                false,
            ),
            InactiveRebuildDecision::Cancel
        );
    }

    #[test]
    fn old_generation_triggers_latest_pending_retry() {
        let old_signature = signature(9);
        let latest_signature = signature(10);

        assert_eq!(
            inactive_rebuild_decision(6, &old_signature, 7, &latest_signature, false, true, true,),
            InactiveRebuildDecision::Retry
        );
    }

    #[test]
    fn inactive_rebuild_drain_policy_enforces_cap_and_budget() {
        let budget = Duration::from_millis(2);
        assert!(rebuild_drain_allows_next(
            3,
            Duration::from_millis(1),
            4,
            budget
        ));
        assert!(!rebuild_drain_allows_next(
            4,
            Duration::from_millis(1),
            4,
            budget
        ));
        assert!(!rebuild_drain_allows_next(0, budget, 4, budget));
    }

    #[test]
    fn inactive_result_is_deferred_by_tab_and_panel_until_tab_returns() {
        let mut deferred = VecDeque::new();
        let live_tabs = [11, 22];

        assert!(defer_inactive_rebuild_result(
            &mut deferred,
            result(22, ActivePanel::Right, 9),
            &live_tabs,
        ));
        assert!(defer_inactive_rebuild_result(
            &mut deferred,
            result(22, ActivePanel::Right, 10),
            &live_tabs,
        ));
        let mut older = result(22, ActivePanel::Right, 11);
        older.generation = 6;
        assert!(defer_inactive_rebuild_result(
            &mut deferred,
            older,
            &live_tabs,
        ));

        assert!(take_deferred_rebuild_results_for_tab(&mut deferred, 11).is_empty());
        let ready = take_deferred_rebuild_results_for_tab(&mut deferred, 22);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].signature.items_revision, 10);
        assert_eq!(ready[0].generation, 7);
        assert!(deferred.is_empty());
    }
}
