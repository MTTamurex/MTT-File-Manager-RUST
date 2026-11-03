use super::*;
use crate::app::state::ImageViewerApp;
use crate::domain::auto_tag_rule::{matching_rules, remap_rule_folders};
use crate::domain::tag_tree::tags_to_assign;
use rustc_hash::FxHashSet;
use std::path::Path;
use std::sync::Arc;

impl ImageViewerApp {
    /// Reloads rules from the database and pushes them to the worker. Called
    /// on startup and whenever a rule or tag hierarchy mutation can affect
    /// the worker's snapshot.
    pub fn reload_auto_tag_rules(&mut self) {
        self.auto_tag_rules = self.app_state_db.get_auto_tag_rules();
        self.auto_tag_status.retain(|id, _| {
            self.auto_tag_rules
                .iter()
                .any(|rule| rule.id == *id && rule.enabled)
        });
        self.sync_auto_tag_worker();
    }

    /// Sends the current rules and tag definitions to the worker thread.
    pub(crate) fn sync_auto_tag_worker(&self) {
        let defs = self.tag_definitions.values().cloned().collect();
        self.auto_tag_manager
            .send_rules_updated(self.auto_tag_rules.clone(), defs);
    }

    /// Mirrors worker-applied assignments into the in-memory state and heals
    /// any ancestry staleness in the database (the worker expands chains from
    /// its own, possibly outdated, definitions snapshot).
    pub fn apply_auto_tag_assignments(&mut self, assignments: &[(PathBuf, i64)]) {
        if assignments.is_empty() {
            return;
        }

        // Group by resolved ancestor chain so each distinct chain becomes a
        // single batch write.
        let mut chains: Vec<(Vec<i64>, Vec<PathBuf>)> = Vec::new();
        let mut changed_tags = FxHashSet::default();
        for (path, rule_tag) in assignments {
            if !self.tag_definitions.contains_key(rule_tag) {
                continue; // tag deleted while the event was in flight
            }
            let chain = tags_to_assign(&self.tag_definitions, *rule_tag);
            changed_tags.extend(chain.iter().copied());
            if let Some(group) = chains.iter_mut().find(|(existing, _)| *existing == chain) {
                group.1.push(path.clone());
            } else {
                chains.push((chain, vec![path.clone()]));
            }
        }
        if chains.is_empty() {
            return;
        }

        chains.retain(|(chain, paths)| self.app_state_db.assign_tags_batch(paths, chain));
        if chains.is_empty() {
            return;
        }

        let assignments_map = Arc::make_mut(&mut self.tag_assignments);
        for (chain, paths) in &chains {
            for path in paths {
                let assignment_key = tag_assignment_key_for_path(assignments_map, path)
                    .cloned()
                    .unwrap_or_else(|| path.clone());
                let ids = assignments_map.entry(assignment_key).or_default();
                for id in chain {
                    if !ids.contains(id) {
                        ids.push(*id);
                    }
                }
            }
        }

        self.recompute_tag_counts_from_assignments();
        self.sync_tag_assignments_normalized();
        self.invalidate_cached_tag_views_for_tags(&changed_tags);
        self.refresh_visible_items_after_tag_change();
        self.ui_ctx.request_repaint();
    }

    /// Re-applies matching rules to the given paths. Called after a copy's
    /// destination tags are cleared, so a rule tag applied by the watcher
    /// while the copy ran is not wiped by the clear.
    pub fn reapply_auto_tag_rules_for_paths(&mut self, paths: &[PathBuf]) {
        if self.auto_tag_rules.is_empty() || paths.is_empty() {
            return;
        }
        let mut assignments = Vec::new();
        for path in paths {
            let Ok(metadata) = path.metadata() else {
                continue;
            };
            let is_dir = metadata.is_dir();
            for matched in matching_rules(&self.auto_tag_rules, path) {
                // Rules that exclude subfolders tag files only.
                if !matched.include_subfolders && is_dir {
                    continue;
                }
                assignments.push((path.clone(), matched.tag_id));
            }
        }
        self.apply_auto_tag_assignments(&assignments);
    }

    /// Follows folder renames/moves so watched folders keep working after the
    /// rename (DB + in-memory + worker snapshot).
    pub fn remap_auto_tag_rule_folders(&mut self, old_root: &Path, new_root: &Path) {
        if self.auto_tag_rules.is_empty() {
            return;
        }
        let remaps = remap_rule_folders(&self.auto_tag_rules, old_root, new_root);
        if remaps.is_empty() {
            return;
        }
        for (id, new_folder) in &remaps {
            if self
                .app_state_db
                .rename_auto_tag_rule_folder(*id, new_folder)
            {
                if let Some(rule) = self.auto_tag_rules.iter_mut().find(|rule| rule.id == *id) {
                    rule.folder_path = new_folder.clone();
                }
            }
        }
        self.sync_auto_tag_worker();
    }
}
