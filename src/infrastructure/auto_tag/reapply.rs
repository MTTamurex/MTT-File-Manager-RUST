//! Copy-destination rechecks run off the UI and skip unmatched paths before I/O.
use super::{AutoTagContext, AutoTagEvent};
use crate::domain::auto_tag_rule::{matching_rules, AutoTagRule};
use crate::domain::file_tag::FileTag;
use rustc_hash::FxHashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

pub(super) fn apply(
    context: &AutoTagContext,
    rules: &[AutoTagRule],
    defs: &FxHashMap<i64, FileTag>,
    paths: Vec<PathBuf>,
) {
    for chunk in paths.chunks(128) {
        if context.shutdown.load(Ordering::Acquire) {
            return;
        }
        let mut by_tag: FxHashMap<i64, Vec<PathBuf>> = FxHashMap::default();
        for path in chunk {
            let matching = matching_rules(rules, path);
            if matching.is_empty() {
                continue;
            }
            let Ok(metadata) = path.metadata() else {
                continue;
            };
            for rule in matching {
                if defs.contains_key(&rule.tag_id)
                    && (rule.include_subfolders || !metadata.is_dir())
                {
                    by_tag.entry(rule.tag_id).or_default().push(path.clone());
                }
            }
        }
        let mut assignments = Vec::new();
        for (tag, paths) in by_tag {
            let ids = crate::domain::tag_tree::tags_to_assign(defs, tag);
            if context.app_state_db.assign_tags_batch(&paths, &ids) {
                assignments.extend(paths.into_iter().map(|path| (path, tag)));
            }
        }
        if !assignments.is_empty() {
            let _ = context
                .event_sender
                .send(AutoTagEvent::Applied { assignments });
        }
    }
}

#[cfg(not(feature = "notify-watcher"))]
pub(super) fn run_manual(
    receiver: std::sync::mpsc::Receiver<super::AutoTagCommand>,
    context: AutoTagContext,
    mut rules: Vec<AutoTagRule>,
    defs: Vec<FileTag>,
    ctx: eframe::egui::Context,
) {
    let mut defs = defs.into_iter().map(|tag| (tag.id, tag)).collect();
    while let Ok(command) = receiver.recv() {
        match command {
            super::AutoTagCommand::RulesUpdated {
                rules: updated,
                defs: updated_defs,
            } => {
                rules = updated;
                defs = updated_defs.into_iter().map(|tag| (tag.id, tag)).collect();
            }
            super::AutoTagCommand::ReapplyPaths(paths) => {
                apply(&context, &rules, &defs, paths);
                ctx.request_repaint();
            }
            super::AutoTagCommand::Shutdown => break,
            super::AutoTagCommand::ApplyExisting { .. } => {}
        }
    }
}
