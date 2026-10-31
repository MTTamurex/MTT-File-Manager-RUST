//! Reopen lightweight closed-tab snapshots without retaining directory caches.
use super::TabManager;

impl TabManager {
    pub(crate) fn has_closed_tabs(&self) -> bool {
        !self.closed_tabs.is_empty()
    }

    pub(crate) fn can_reopen_closed_tab(&self) -> bool {
        self.has_closed_tabs() && self.can_add_tab()
    }

    pub fn reopen_closed_tab(&mut self) -> bool {
        // Check before popping: reaching the tab limit must not lose history.
        if !self.can_reopen_closed_tab() {
            return false;
        }
        let mut tab = self.closed_tabs.pop().unwrap();
        tab.id = self.next_id;
        tab.pinned = false;
        // Pending rebuilds refer to the discarded listing, not the fresh load.
        tab.pending_items_rebuild = false;
        tab.pending_items_count = 0;
        tab.inactive_final_items_rebuild_pending = false;
        self.next_id += 1;

        let boundary = self.tabs.iter().take_while(|tab| tab.pinned).count();
        let insert_pos = (self.active_tab + 1).max(boundary);
        self.tabs.insert(insert_pos, tab);
        self.active_tab = insert_pos;
        true
    }
}

#[cfg(test)]
mod tests;
