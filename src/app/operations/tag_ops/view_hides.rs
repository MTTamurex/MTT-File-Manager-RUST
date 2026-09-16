use crate::app::state::ImageViewerApp;
use crate::domain::special_paths::tag_id_from_view_path;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::VecDeque;
use std::path::PathBuf;

fn append_unique_paths(target: &mut VecDeque<PathBuf>, paths: &[PathBuf]) {
    let mut queued = FxHashSet::default();
    queued.extend(target.iter().map(|path| super::normalize_path_text(path)));
    for path in paths {
        if queued.insert(super::normalize_path_text(path)) {
            target.push_back(path.clone());
        }
    }
}

impl ImageViewerApp {
    pub(crate) fn queue_tag_view_hides_for_generation(
        &mut self,
        generation: usize,
        paths: &[PathBuf],
    ) {
        if paths.is_empty() || !self.tag_view_generation_is_open(generation) {
            return;
        }

        let pending = self.pending_tag_view_hides.entry(generation).or_default();
        append_unique_paths(pending, paths);
    }

    fn tag_view_generation_is_open(&self, generation: usize) -> bool {
        if self.generation == generation
            && tag_id_from_view_path(&self.navigation_state.current_path).is_some()
        {
            return true;
        }
        if self
            .dual_panel_inactive_state
            .as_ref()
            .is_some_and(|snapshot| {
                snapshot.generation == generation && tag_id_from_view_path(&snapshot.path).is_some()
            })
        {
            return true;
        }

        self.tab_manager.tabs.iter().any(|tab| {
            (tab.generation == generation && tag_id_from_view_path(&tab.path).is_some())
                || tab
                    .dual_panel_inactive_state
                    .as_ref()
                    .is_some_and(|snapshot| {
                        snapshot.generation == generation
                            && tag_id_from_view_path(&snapshot.path).is_some()
                    })
        })
    }

    pub(crate) fn queue_tag_view_hides_for_all_views(&mut self, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }

        let mut generations = FxHashSet::default();
        if tag_id_from_view_path(&self.navigation_state.current_path).is_some() {
            generations.insert(self.generation);
        }
        if let Some(snapshot) = self.dual_panel_inactive_state.as_ref() {
            if tag_id_from_view_path(&snapshot.path).is_some() {
                generations.insert(snapshot.generation);
            }
        }

        let active_tab = self.tab_manager.active_tab;
        for (index, tab) in self.tab_manager.tabs.iter().enumerate() {
            if index != active_tab && tag_id_from_view_path(&tab.path).is_some() {
                generations.insert(tab.generation);
            }
            if let Some(snapshot) = tab.dual_panel_inactive_state.as_ref() {
                if tag_id_from_view_path(&snapshot.path).is_some() {
                    generations.insert(snapshot.generation);
                }
            }
        }

        for generation in generations {
            append_unique_paths(
                self.pending_tag_view_hides.entry(generation).or_default(),
                paths,
            );
        }
    }

    pub(crate) fn hide_unavailable_paths_from_tag_views(&mut self, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }

        self.prune_paths_from_loaded_tag_views(paths);
        self.ui_ctx.request_repaint();
    }

    pub(crate) fn apply_ready_tag_view_hides(&mut self) {
        const MAX_HIDES_PER_FRAME: usize = 32;

        let mut generations: FxHashMap<usize, bool> = FxHashMap::default();
        generations.insert(self.generation, !self.is_loading_folder);
        if let Some(snapshot) = self.dual_panel_inactive_state.as_ref() {
            generations
                .entry(snapshot.generation)
                .and_modify(|ready| *ready |= !snapshot.is_loading_folder)
                .or_insert(!snapshot.is_loading_folder);
        }
        let active_tab = self.tab_manager.active_tab;
        for (index, tab) in self.tab_manager.tabs.iter().enumerate() {
            if index != active_tab {
                generations
                    .entry(tab.generation)
                    .and_modify(|ready| *ready = true)
                    .or_insert(true);
            }
            if let Some(snapshot) = tab.dual_panel_inactive_state.as_ref() {
                generations
                    .entry(snapshot.generation)
                    .and_modify(|ready| *ready |= !snapshot.is_loading_folder)
                    .or_insert(!snapshot.is_loading_folder);
            }
        }

        self.pending_tag_view_hides
            .retain(|generation, paths| generations.contains_key(generation) && !paths.is_empty());
        // A result from a superseded generation is no longer safe to apply:
        // its path may have been remounted or recreated in the meantime. The
        // newer tag load or the next background purge will produce a fresh
        // result if the path is still unavailable.

        let ready_generations: Vec<usize> = generations
            .iter()
            .filter_map(|(generation, ready)| ready.then_some(*generation))
            .collect();
        let mut candidates = Vec::new();
        let mut made_progress = true;
        while candidates.len() < MAX_HIDES_PER_FRAME && made_progress {
            made_progress = false;
            for generation in &ready_generations {
                if candidates.len() >= MAX_HIDES_PER_FRAME {
                    break;
                }
                let Some(paths) = self.pending_tag_view_hides.get_mut(generation) else {
                    continue;
                };
                if let Some(path) = paths.pop_front() {
                    candidates.push(path);
                    made_progress = true;
                }
            }
        }
        self.pending_tag_view_hides
            .retain(|_, paths| !paths.is_empty());

        // Every candidate was already checked by its producing worker (or came
        // from persisted tag-assignment removal). Do not repeat filesystem
        // probes on the UI thread; a later remount is reconciled by the next
        // tag-view load or purge pass.
        self.hide_unavailable_paths_from_tag_views(&candidates);

        if self
            .pending_tag_view_hides
            .keys()
            .any(|generation| generations.get(generation).copied().unwrap_or(false))
        {
            self.ui_ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::append_unique_paths;
    use std::collections::VecDeque;
    use std::path::PathBuf;

    #[test]
    fn hide_queue_deduplicates_paths_without_reordering_existing_entries() {
        let mut queued = VecDeque::from([PathBuf::from(r"C:\Data\first.txt")]);
        let incoming = [
            PathBuf::from(r"c:/data/first.txt"),
            PathBuf::from(r"C:\Data\second.txt"),
        ];

        append_unique_paths(&mut queued, &incoming);

        assert_eq!(
            queued.into_iter().collect::<Vec<_>>(),
            vec![
                PathBuf::from(r"C:\Data\first.txt"),
                PathBuf::from(r"C:\Data\second.txt"),
            ]
        );
    }
}
