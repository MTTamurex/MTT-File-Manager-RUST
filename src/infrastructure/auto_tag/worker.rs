use super::{AutoTagCommand, AutoTagContext, AutoTagEvent, AutoTagRuleStatus};
use crate::domain::auto_tag_rule::{matching_rules, AutoTagRule};
use crate::domain::file_tag::FileTag;
use crate::domain::tag_tree::tags_to_assign;
use crate::infrastructure::removal_aware_watcher::RemovalAwareWatcher;
use notify::RecursiveMode;
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant};

const STABILITY_DELAY: Duration = Duration::from_secs(2);
const HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(1);
const INITIAL_WATCHER_RETRY_DELAY: Duration = Duration::from_secs(1);
const MAX_WATCHER_RETRY_DELAY: Duration = Duration::from_secs(30);
const EVENT_QUEUE_CAPACITY: usize = 1024;
const MAX_EVENTS_PER_TICK: usize = 256;

mod scan;
use scan::{collect_paths, scan_and_apply};

#[cfg(test)]
mod tests;

pub(crate) fn run_auto_tag(
    command_receiver: Receiver<AutoTagCommand>,
    context: AutoTagContext,
    mut rules: Vec<AutoTagRule>,
    defs: Vec<FileTag>,
    ui_ctx: eframe::egui::Context,
) {
    let mut defs_by_id: FxHashMap<i64, FileTag> =
        defs.into_iter().map(|tag| (tag.id, tag)).collect();
    let mut pending: FxHashMap<PathBuf, Instant> = FxHashMap::default();
    let (event_tx, event_rx) =
        std::sync::mpsc::sync_channel::<notify::Result<notify::Event>>(EVENT_QUEUE_CAPACITY);
    let overflowed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut runtime = WatcherRuntime::new(event_tx, event_rx, Arc::clone(&overflowed));
    let mut next_health_check = Instant::now();
    let mut retry_delay = INITIAL_WATCHER_RETRY_DELAY;
    let mut reported_status: FxHashMap<i64, AutoTagRuleStatus> = FxHashMap::default();

    // Register before scanning so arrivals during the scan are not lost.
    reconcile_watcher(
        &rules,
        &mut runtime,
        &context,
        &mut reported_status,
        &mut retry_delay,
    );
    // Startup scan: tag everything already inside rule folders (user choice —
    // this also re-tags items the user unchecked manually).
    scan_and_apply(&context, &rules, &defs_by_id, None);
    ui_ctx.request_repaint();

    loop {
        if context.shutdown.load(Ordering::Acquire) {
            break;
        }
        if Instant::now() >= next_health_check {
            if reconcile_watcher(
                &rules,
                &mut runtime,
                &context,
                &mut reported_status,
                &mut retry_delay,
            ) {
                ui_ctx.request_repaint();
            }
            next_health_check = Instant::now() + HEALTH_CHECK_INTERVAL;
        }

        for _ in 0..MAX_EVENTS_PER_TICK {
            let Ok(event) = runtime.event_receiver.try_recv() else {
                break;
            };
            match event {
                Ok(event) => {
                    if event.need_rescan() {
                        runtime.overflowed.store(true, Ordering::Release);
                    }
                    handle_notify_event(&event, &rules, &mut pending);
                }
                Err(error) => {
                    runtime.ready = false;
                    runtime.next_retry = Instant::now();
                    let _ = context.event_sender.send(AutoTagEvent::WatcherError {
                        message: error.to_string(),
                    });
                    ui_ctx.request_repaint();
                }
            }
        }
        if runtime.overflowed.swap(false, Ordering::AcqRel) {
            // The event queue dropped notifications; a full re-scan restores
            // coverage because assignments are idempotent.
            scan_and_apply(&context, &rules, &defs_by_id, None);
            ui_ctx.request_repaint();
        }

        let next_command = match command_receiver.recv_timeout(Duration::from_millis(250)) {
            Ok(command) => Some(command),
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => None,
        };
        if context.shutdown.load(Ordering::Acquire) {
            break;
        }
        match next_command {
            Some(AutoTagCommand::RulesUpdated {
                rules: new_rules,
                defs: new_defs,
            }) => {
                rules = new_rules;
                defs_by_id = new_defs.into_iter().map(|tag| (tag.id, tag)).collect();
                pending.retain(|path, _| !matching_rules(&rules, path).is_empty());
                next_health_check = Instant::now();
            }
            Some(AutoTagCommand::ApplyExisting { rule_id }) => {
                if let Some(rule) = rules.iter().find(|rule| rule.id == rule_id) {
                    scan_and_apply(
                        &context,
                        std::slice::from_ref(rule),
                        &defs_by_id,
                        Some(rule_id),
                    );
                    ui_ctx.request_repaint();
                }
            }
            Some(AutoTagCommand::Shutdown) => break,
            None => {}
        }

        if process_stable_files(&context, &rules, &defs_by_id, &mut pending) {
            ui_ctx.request_repaint();
        }
    }
}

struct WatcherRuntime {
    watcher: Option<RemovalAwareWatcher>,
    signature: Option<Vec<String>>,
    ready: bool,
    registered_folders: FxHashSet<String>,
    next_retry: Instant,
    event_receiver: Receiver<notify::Result<notify::Event>>,
    event_sender: SyncSender<notify::Result<notify::Event>>,
    overflowed: Arc<std::sync::atomic::AtomicBool>,
}

impl WatcherRuntime {
    fn new(
        event_sender: SyncSender<notify::Result<notify::Event>>,
        event_receiver: Receiver<notify::Result<notify::Event>>,
        overflowed: Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        Self {
            watcher: None,
            signature: None,
            ready: false,
            registered_folders: FxHashSet::default(),
            next_retry: Instant::now(),
            event_receiver,
            event_sender,
            overflowed,
        }
    }
}

fn watch_signature(rules: &[AutoTagRule]) -> Vec<String> {
    let mut entries: Vec<String> = rules
        .iter()
        .filter(|rule| rule.enabled)
        .map(|rule| {
            format!(
                "{}:{}",
                crate::domain::file_tag::normalize_tag_path_key(&rule.folder_path),
                rule.include_subfolders
            )
        })
        .collect();
    entries.sort();
    entries.dedup();
    entries
}

fn configure_watcher(
    rules: &[AutoTagRule],
    event_sender: SyncSender<notify::Result<notify::Event>>,
    overflowed: Arc<std::sync::atomic::AtomicBool>,
) -> (Option<RemovalAwareWatcher>, bool, FxHashSet<String>) {
    let mut registered = FxHashSet::default();
    let handler = move |event: notify::Result<notify::Event>| {
        // try_send never blocks the notify thread; a full queue drops the
        // event and flags an overflow so the worker re-scans (assignments are
        // idempotent, so a scan restores coverage).
        if event_sender.try_send(event).is_err() {
            overflowed.store(true, Ordering::Release);
        }
    };
    let Ok(mut watcher) = RemovalAwareWatcher::new(handler) else {
        return (None, false, registered);
    };
    let mut ready = true;
    for (path, recursive) in watch_targets(rules) {
        let mode = if recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        match watcher.watch(&path, mode) {
            Ok(()) => {
                registered.insert(crate::domain::file_tag::normalize_tag_path_key(&path));
            }
            Err(error) => {
                log::warn!("[AUTO-TAG] Failed to register folder watcher: {error}");
                ready = false;
            }
        }
    }
    (Some(watcher), ready && !registered.is_empty(), registered)
}

/// One native watch per normalized folder, with the widest requested scope.
/// RemovalAwareWatcher keeps the first mode registered for any given path.
fn watch_targets(rules: &[AutoTagRule]) -> Vec<(PathBuf, bool)> {
    let mut targets: FxHashMap<String, (PathBuf, bool)> = FxHashMap::default();
    for rule in rules.iter().filter(|rule| rule.enabled) {
        targets
            .entry(crate::domain::file_tag::normalize_tag_path_key(
                &rule.folder_path,
            ))
            .and_modify(|(_, recursive)| *recursive |= rule.include_subfolders)
            .or_insert((rule.folder_path.clone(), rule.include_subfolders));
    }
    targets.into_values().collect()
}

fn reconcile_watcher(
    rules: &[AutoTagRule],
    runtime: &mut WatcherRuntime,
    context: &AutoTagContext,
    reported_status: &mut FxHashMap<i64, AutoTagRuleStatus>,
    retry_delay: &mut Duration,
) -> bool {
    let signature = watch_signature(rules);
    let now = Instant::now();
    let signature_changed = runtime.signature.as_ref() != Some(&signature);
    let watcher_released = runtime
        .watcher
        .as_ref()
        .is_some_and(|watcher| watcher.needs_refresh());
    if rules.iter().filter(|rule| rule.enabled).any(|rule| {
        runtime
            .registered_folders
            .contains(&crate::domain::file_tag::normalize_tag_path_key(
                &rule.folder_path,
            ))
            && !rule.folder_path.is_dir()
    }) {
        runtime.ready = false;
        runtime.next_retry = now;
    }
    if signature_changed || watcher_released || (!runtime.ready && now >= runtime.next_retry) {
        let (watcher, ready, registered) = configure_watcher(
            rules,
            runtime.event_sender.clone(),
            Arc::clone(&runtime.overflowed),
        );
        runtime.watcher = watcher;
        runtime.ready = ready;
        runtime.registered_folders = registered;
        runtime.signature = Some(signature);
        if ready {
            *retry_delay = INITIAL_WATCHER_RETRY_DELAY;
            runtime.next_retry = now + HEALTH_CHECK_INTERVAL;
        } else {
            runtime.next_retry = now + *retry_delay;
            *retry_delay = (*retry_delay * 2).min(MAX_WATCHER_RETRY_DELAY);
        }
    }

    let mut changed = false;
    reported_status.retain(|id, _| rules.iter().any(|rule| rule.id == *id && rule.enabled));
    for rule in rules.iter().filter(|rule| rule.enabled) {
        let status = if runtime.registered_folders.contains(
            &crate::domain::file_tag::normalize_tag_path_key(&rule.folder_path),
        ) {
            AutoTagRuleStatus::Watching
        } else {
            AutoTagRuleStatus::FolderMissing
        };
        if reported_status.insert(rule.id, status) != Some(status) {
            changed = true;
            let _ = context.event_sender.send(AutoTagEvent::Status {
                rule_id: rule.id,
                status,
            });
        }
    }
    changed
}

fn handle_notify_event(
    event: &notify::Event,
    rules: &[AutoTagRule],
    pending: &mut FxHashMap<PathBuf, Instant>,
) {
    use notify::event::{EventKind, ModifyKind, RenameMode};
    let now = Instant::now();
    match &event.kind {
        EventKind::Create(_) => {
            for path in &event.paths {
                queue_path(pending, rules, path, now);
            }
        }
        EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Metadata(_) | ModifyKind::Any) => {
            for path in &event.paths {
                queue_path(pending, rules, path, now);
            }
        }
        EventKind::Modify(ModifyKind::Name(mode)) => {
            // RDCW renames usually arrive as `Any`/`Both` carrying both paths.
            if event.paths.len() >= 2 {
                let old = &event.paths[0];
                let new = &event.paths[1];
                pending.remove(old);
                if !matching_rules(rules, new).is_empty() {
                    pending.insert(new.clone(), now);
                }
            } else {
                match mode {
                    RenameMode::To | RenameMode::Any | RenameMode::Other => {
                        for path in &event.paths {
                            queue_path(pending, rules, path, now);
                        }
                    }
                    _ => {
                        for path in &event.paths {
                            pending.remove(path);
                        }
                    }
                }
            }
        }
        EventKind::Remove(_) => {
            for path in &event.paths {
                pending.remove(path);
            }
        }
        _ => {}
    }
}

fn queue_path(
    pending: &mut FxHashMap<PathBuf, Instant>,
    rules: &[AutoTagRule],
    path: &Path,
    now: Instant,
) {
    if matching_rules(rules, path).is_empty() {
        return;
    }
    pending.insert(path.to_path_buf(), now);
}

/// Applies every pending path that has been quiet for [`STABILITY_DELAY`].
/// Returns true when anything was assigned (so the UI repaints).
fn process_stable_files(
    context: &AutoTagContext,
    rules: &[AutoTagRule],
    defs_by_id: &FxHashMap<i64, FileTag>,
    pending: &mut FxHashMap<PathBuf, Instant>,
) -> bool {
    let now = Instant::now();
    let mut stable: Vec<PathBuf> = Vec::new();
    pending.retain(|path, first_seen| {
        if now.duration_since(*first_seen) >= STABILITY_DELAY {
            stable.push(path.clone());
            false
        } else {
            true
        }
    });
    if stable.is_empty() {
        return false;
    }

    // Group paths per rule tag, then write one batch per tag with the full
    // implicit-parent chain. INSERT OR IGNORE keeps re-applications cheap.
    let mut paths_by_tag: FxHashMap<i64, Vec<PathBuf>> = FxHashMap::default();
    let mut assignments: Vec<(PathBuf, i64)> = Vec::new();
    let recursive = stable.iter().any(|path| {
        matching_rules(rules, path)
            .iter()
            .any(|rule| rule.include_subfolders)
    });
    for path in collect_paths(context, stable, recursive) {
        let Ok(metadata) = path.metadata() else {
            continue; // vanished while pending; the GC cleans stale rows
        };
        let is_dir = metadata.is_dir();
        for matched in matching_rules(rules, &path) {
            // Rules that exclude subfolders tag files only — a folder is
            // never an arrival they cover.
            if (!matched.include_subfolders && is_dir)
                || crate::domain::file_tag::normalize_tag_path_key(&path)
                    == crate::domain::file_tag::normalize_tag_path_key(&matched.folder_path)
            {
                continue;
            }
            paths_by_tag
                .entry(matched.tag_id)
                .or_default()
                .push(path.clone());
        }
    }
    for (tag_id, paths) in &paths_by_tag {
        if !defs_by_id.contains_key(tag_id) {
            continue;
        }
        let chain = tags_to_assign(defs_by_id, *tag_id);
        if !context.app_state_db.assign_tags_batch(paths, &chain) {
            log::warn!(
                "[AUTO-TAG] Failed to assign tag {tag_id} to {} path(s)",
                paths.len()
            );
        } else {
            assignments.extend(paths.iter().map(|path| (path.clone(), *tag_id)));
        }
    }
    if assignments.is_empty() {
        return false;
    }
    let _ = context
        .event_sender
        .send(AutoTagEvent::Applied { assignments });
    true
}
