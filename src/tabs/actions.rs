//! Tab policies shared by the workspace and physical pane tab strips.
use super::{PanelTabSet, TabManager};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabAction {
    Close,
    Duplicate,
    CloseLeft,
    CloseRight,
    TogglePin,
    PinQuickAccess,
    UnpinQuickAccess,
    ExpandFolderTree,
    CollapseFolderTree,
}

pub(crate) fn close_candidates<T>(
    tabs: &[T],
    target: usize,
    action: TabAction,
    pinned: impl Fn(&T) -> bool,
) -> Vec<usize> {
    if target >= tabs.len() {
        return Vec::new();
    }
    (0..tabs.len())
        .rev()
        .filter(|&index| {
            !pinned(&tabs[index])
                && match action {
                    TabAction::CloseLeft => index < target,
                    TabAction::CloseRight => index > target,
                    _ => false,
                }
        })
        .collect()
}

pub(crate) fn valid_move<T>(
    tabs: &[T],
    from: usize,
    gap: usize,
    pinned: impl Fn(&T) -> bool,
) -> bool {
    let Some(tab) = tabs.get(from) else {
        return false;
    };
    let boundary = tabs.iter().take_while(|tab| pinned(tab)).count();
    gap <= tabs.len()
        && if pinned(tab) {
            gap <= boundary
        } else {
            gap >= boundary
        }
}

fn toggle_pin<T>(
    tabs: &mut Vec<T>,
    active: &mut usize,
    index: usize,
    pinned: impl Fn(&T) -> bool,
    set: impl Fn(&mut T, bool),
) -> bool {
    let Some(tab) = tabs.get(index) else {
        return false;
    };
    let was_pinned = pinned(tab);
    let boundary = tabs.iter().take_while(|tab| pinned(tab)).count();
    set(&mut tabs[index], !was_pinned);
    // Move before enforcing the new boundary. The active identity is preserved.
    super::reorder::move_to_gap(tabs, active, index, boundary);
    true
}

impl TabManager {
    pub(crate) fn toggle_pin(&mut self, index: usize) -> bool {
        toggle_pin(
            &mut self.tabs,
            &mut self.active_tab,
            index,
            |tab| tab.pinned,
            |tab, value| tab.pinned = value,
        )
    }

    pub(crate) fn duplicate_at(&mut self, index: usize) -> bool {
        if !self.can_add_tab() || index >= self.tabs.len() {
            return false;
        }
        let mut tab = self.tabs[index].clone();
        tab.id = self.next_id;
        tab.pinned = false;
        tab.selected_thumbnail = None;
        tab.selected_gif = None;
        if let Some(snapshot) = tab.dual_panel_inactive_state.as_mut() {
            detach_snapshot(snapshot);
        }
        if let Some(tabs) = tab.dual_panel_tabs.as_mut() {
            for inner in tabs.left.tabs.iter_mut().chain(&mut tabs.right.tabs) {
                detach_snapshot(&mut inner.snapshot);
            }
        }
        self.next_id += 1;
        let boundary = self.tabs.iter().take_while(|tab| tab.pinned).count();
        let insert = (index + 1).max(boundary);
        self.tabs.insert(insert, tab);
        self.active_tab = insert;
        true
    }
}

impl PanelTabSet {
    pub(crate) fn toggle_pin(&mut self, index: usize) -> bool {
        toggle_pin(
            &mut self.tabs,
            &mut self.active_tab,
            index,
            |tab| tab.pinned,
            |tab, value| tab.pinned = value,
        )
    }
}

/// Duplicates must never share a folder-load cancellation token or media player.
pub(crate) fn detach_snapshot(snapshot: &mut crate::app::dual_panel::PanelSnapshot) {
    snapshot.folder_load_generation =
        std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(snapshot.generation));
    snapshot.selected_gif = None;
    snapshot.selected_thumbnail = None;
    snapshot.renaming_state = None;
    snapshot.focus_rename = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_preserve_active_identity_and_enforce_boundary() {
        let mut manager = TabManager::new();
        manager.new_tab_at("C:\\");
        manager.new_tab_at("D:\\");
        let active = manager.active().id;
        manager.toggle_pin(1);
        assert!(manager.tabs[0].pinned);
        assert_eq!(manager.active().id, active);
        assert!(!manager.close_tab(0));
        assert_eq!(manager.tabs.len(), 3);
        assert!(!manager.move_tab_to_gap(0, 3));
        assert!(!manager.move_tab_to_gap(2, 0));
        manager.switch_to(0);
        assert!(!manager.move_tab_right());
        manager.switch_to(1);
        assert!(!manager.move_tab_left());
        manager.switch_to(2);
        manager.toggle_pin(0);
        assert!(!manager.tabs[0].pinned);
        assert_eq!(manager.active().id, active);
    }

    #[test]
    fn batch_close_excludes_only_pinned_and_target_tabs() {
        let pins = [true, false, false, false, false];
        assert_eq!(
            close_candidates(&pins, 4, TabAction::CloseLeft, |p| *p),
            vec![3, 2, 1]
        );
        assert_eq!(
            close_candidates(&pins, 0, TabAction::CloseRight, |p| *p),
            vec![4, 3, 2, 1]
        );
    }

    #[test]
    fn batch_close_keeps_target_focus_when_active_tab_is_closed() {
        for action in [TabAction::CloseLeft, TabAction::CloseRight] {
            for pinned in [false, true] {
                for target in 0..5 {
                    for selected in 0..5 {
                        let mut manager = TabManager::new();
                        for _ in 1..5 {
                            manager.new_tab();
                        }
                        manager.tabs[0].pinned = pinned;
                        manager.switch_to(selected);
                        let candidates =
                            close_candidates(&manager.tabs, target, action, |t| t.pinned);
                        let closing_active = candidates.contains(&selected);
                        let expected_active =
                            manager.tabs[if closing_active { target } else { selected }].id;
                        let closed_ids: Vec<_> =
                            candidates.iter().map(|&i| manager.tabs[i].id).collect();
                        let target_id = manager.tabs[target].id;
                        if closing_active {
                            manager.switch_to(target);
                        }
                        for index in candidates {
                            assert!(!manager.close_tab(index));
                        }
                        assert_eq!(manager.active().id, expected_active);
                        assert!(manager.tabs.iter().any(|t| t.id == target_id));
                        assert!(manager.tabs.iter().all(|t| !closed_ids.contains(&t.id)));
                        if pinned {
                            assert!(manager.tabs[0].pinned);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn duplicate_is_unpinned_and_keeps_virtual_view_and_workspace_state() {
        let mut manager =
            TabManager::new_at_path(crate::domain::special_paths::RECYCLE_BIN_VIEW_ID);
        manager.tabs[0].dual_panel_enabled = true;
        manager.toggle_pin(0);
        assert!(manager.duplicate_at(0));
        assert_eq!(manager.active_tab, 1);
        assert!(!manager.active().pinned);
        assert!(manager.active().is_recycle_bin_view);
        assert!(manager.active().dual_panel_enabled);
        assert_ne!(manager.tabs[0].id, manager.tabs[1].id);
    }

    #[test]
    fn last_pinned_tab_never_requests_application_close() {
        let mut manager = TabManager::new();
        manager.toggle_pin(0);
        assert!(!manager.close_active_tab());
        assert_eq!(manager.tabs.len(), 1);
        manager.toggle_pin(0);
        assert!(manager.close_active_tab());
    }

    #[test]
    fn reopening_and_duplicating_never_split_the_pinned_prefix() {
        let mut manager = TabManager::new();
        manager.new_tab_at("C:\\");
        manager.new_tab_at("D:\\");
        manager.close_tab(2);
        manager.toggle_pin(0);
        manager.toggle_pin(1);
        manager.switch_to(0);
        assert!(manager.reopen_closed_tab());
        assert_eq!(manager.active_tab, 2);
        assert!(manager.duplicate_at(0));
        assert_eq!(manager.active_tab, 2);
        assert!(manager.tabs[..2].iter().all(|tab| tab.pinned));
        assert!(manager.tabs[2..].iter().all(|tab| !tab.pinned));
    }

    #[test]
    fn duplication_respects_cap_without_changing_active_tab() {
        let mut manager = TabManager::new();
        while manager.can_add_tab() {
            manager.new_tab();
        }
        let active = manager.active().id;
        assert!(!manager.duplicate_at(0));
        assert_eq!(manager.tabs.len(), super::super::MAX_TABS);
        assert_eq!(manager.active().id, active);
    }

    #[test]
    fn pin_toggle_keeps_active_identity_and_partition_for_every_tab() {
        for selected in 0..4 {
            for target in 0..4 {
                let mut manager = TabManager::new();
                for _ in 1..4 {
                    manager.new_tab();
                }
                manager.toggle_pin(1);
                manager.switch_to(selected);
                let active = manager.active().id;
                manager.toggle_pin(target);
                let boundary = manager.tabs.iter().take_while(|tab| tab.pinned).count();
                assert!(manager.tabs[boundary..].iter().all(|tab| !tab.pinned));
                assert_eq!(manager.active().id, active);
            }
        }
    }
}
