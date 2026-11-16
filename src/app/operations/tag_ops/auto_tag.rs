use super::persistence::TagWrite;
use super::*;
use crate::app::state::ImageViewerApp;
use crate::domain::auto_tag_rule::remap_rule_folders;
use crate::domain::tag_tree::tags_to_assign;
use std::path::Path;

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
        let mut chains: FxHashMap<Vec<i64>, Vec<PathBuf>> = FxHashMap::default();
        let mut resolved = FxHashMap::default();
        for (path, rule_tag) in assignments {
            if !self.tag_definitions.contains_key(rule_tag) {
                continue; // tag deleted while the event was in flight
            }
            let chain = resolved
                .entry(*rule_tag)
                .or_insert_with(|| tags_to_assign(&self.tag_definitions, *rule_tag));
            chains.entry(chain.clone()).or_default().push(path.clone());
        }
        if chains.is_empty() {
            return;
        }

        for (chain, paths) in chains {
            if !self.queue_tag_write(TagWrite::HealAuto {
                paths: paths.clone(),
                rule_tag: chain[0],
                ids: chain.clone(),
            }) {
                continue;
            }
            let mut changes: FxHashMap<String, (PathBuf, Vec<i64>)> = FxHashMap::default();
            for path in paths {
                let key = normalize_path_text(&path);
                let (_, ids) = changes.entry(key).or_insert_with(|| {
                    let path = self.tag_path_index.canonical(&path).unwrap_or(path);
                    let ids = self.tag_assignments.get(&path).cloned().unwrap_or_default();
                    (path, ids)
                });
                for id in &chain {
                    if !ids.contains(id) {
                        ids.push(*id);
                    }
                }
            }
            self.replace_tag_paths(changes.into_values().collect());
        }
    }

    /// Re-applies matching rules to the given paths. Called after a copy's
    /// destination tags are cleared, so a rule tag applied by the watcher
    /// while the copy ran is not wiped by the clear.
    pub fn reapply_auto_tag_rules_for_paths(&mut self, paths: &[PathBuf]) {
        if self.auto_tag_rules.is_empty() || paths.is_empty() {
            return;
        }
        self.tag_updates.reapply_paths.extend(paths.iter().cloned());
        self.ui_ctx.request_repaint();
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
