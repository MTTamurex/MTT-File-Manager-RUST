use crate::domain::organizer_rule::OrganizerRule;
use crate::infrastructure::organizer::{OrganizerEvent, OrganizerRuleStatus};
use notify::{RecursiveMode, Watcher};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{Sender, SyncSender},
    Arc,
};
use std::time::{Duration, Instant};

use super::{
    normalize_watched_path, HEALTH_CHECK_INTERVAL, INITIAL_WATCHER_RETRY_DELAY,
    MAX_WATCHER_RETRY_DELAY,
};

pub(super) struct WatcherRuntime {
    pub(super) watcher: Option<notify::RecommendedWatcher>,
    pub(super) signature: Option<Vec<(String, bool)>>,
    pub(super) ready: bool,
    pub(super) registered_folders: HashSet<String>,
    pub(super) retry_delay: Duration,
    pub(super) next_retry: Instant,
    event_sender: SyncSender<notify::Result<notify::Event>>,
    overflowed: Arc<AtomicBool>,
}

impl WatcherRuntime {
    pub(super) fn new(
        event_sender: SyncSender<notify::Result<notify::Event>>,
        overflowed: Arc<AtomicBool>,
    ) -> Self {
        Self {
            watcher: None,
            signature: None,
            ready: false,
            registered_folders: HashSet::new(),
            retry_delay: INITIAL_WATCHER_RETRY_DELAY,
            next_retry: Instant::now(),
            event_sender,
            overflowed,
        }
    }
}

pub(super) fn reset_watcher(runtime: &mut WatcherRuntime) {
    runtime.watcher = None;
    runtime.signature = None;
    runtime.ready = false;
    runtime.registered_folders.clear();
    runtime.retry_delay = INITIAL_WATCHER_RETRY_DELAY;
    runtime.next_retry = Instant::now();
}

pub(super) fn reconcile_watcher(
    rules: &[OrganizerRule],
    paused_rules: &HashSet<i64>,
    runtime: &mut WatcherRuntime,
    ui_ctx: &eframe::egui::Context,
    event_sender: &Sender<OrganizerEvent>,
    reported_statuses: &mut HashMap<i64, OrganizerRuleStatus>,
) -> Vec<i64> {
    let signature = watch_signature(rules);
    let now = Instant::now();
    let signature_changed = runtime.signature.as_ref() != Some(&signature);
    if signature_changed || (!runtime.ready && now >= runtime.next_retry) {
        let (new_watcher, ready, new_registered_folders) = configure_watcher(
            rules,
            runtime.event_sender.clone(),
            Arc::clone(&runtime.overflowed),
            ui_ctx,
        );
        runtime.watcher = new_watcher;
        runtime.ready = ready;
        runtime.registered_folders = new_registered_folders;
        runtime.signature = Some(signature);
        if ready {
            runtime.retry_delay = INITIAL_WATCHER_RETRY_DELAY;
            runtime.next_retry = now + HEALTH_CHECK_INTERVAL;
        } else {
            runtime.next_retry = now + runtime.retry_delay;
            runtime.retry_delay = (runtime.retry_delay * 2).min(MAX_WATCHER_RETRY_DELAY);
        }
    }

    let mut recovered = Vec::new();
    for rule in rules {
        let status = status_for_rule(rule, paused_rules, &runtime.registered_folders);
        let previous = reported_statuses.insert(rule.id, status);
        if previous != Some(status) {
            let _ = event_sender.send(OrganizerEvent::Status {
                rule_id: rule.id,
                status,
            });
        }
        if status == OrganizerRuleStatus::Active
            && previous.is_some_and(|previous| {
                matches!(
                    previous,
                    OrganizerRuleStatus::SourceUnavailable
                        | OrganizerRuleStatus::DestinationUnavailable
                        | OrganizerRuleStatus::BothUnavailable
                        | OrganizerRuleStatus::Recovering
                )
            })
        {
            recovered.push(rule.id);
        }
    }
    reported_statuses.retain(|rule_id, _| rules.iter().any(|rule| rule.id == *rule_id));
    recovered
}

pub(super) fn status_for_rule(
    rule: &OrganizerRule,
    paused_rules: &HashSet<i64>,
    registered_folders: &HashSet<String>,
) -> OrganizerRuleStatus {
    if !rule.enabled {
        return OrganizerRuleStatus::Disabled;
    }
    if paused_rules.contains(&rule.id) {
        return OrganizerRuleStatus::Paused;
    }
    let source_available = rule.source_folder.is_dir();
    let destination_available = rule.destination_folder.is_dir();
    match (source_available, destination_available) {
        (false, false) => OrganizerRuleStatus::BothUnavailable,
        (false, true) => OrganizerRuleStatus::SourceUnavailable,
        (true, false) => OrganizerRuleStatus::DestinationUnavailable,
        (true, true)
            if rule_watch_folders(rule).all(|folder| {
                folder.is_dir() && registered_folders.contains(&normalize_watched_path(folder))
            }) =>
        {
            OrganizerRuleStatus::Active
        }
        (true, true) => OrganizerRuleStatus::Recovering,
    }
}

fn watch_signature(rules: &[OrganizerRule]) -> Vec<(String, bool)> {
    watched_folders(rules)
        .into_iter()
        .map(|folder| {
            let available = folder.is_dir();
            (normalize_watched_path(&folder), available)
        })
        .collect()
}

fn configure_watcher(
    rules: &[OrganizerRule],
    event_sender: SyncSender<notify::Result<notify::Event>>,
    watch_overflowed: Arc<AtomicBool>,
    ui_ctx: &eframe::egui::Context,
) -> (Option<notify::RecommendedWatcher>, bool, HashSet<String>) {
    let ui_ctx = ui_ctx.clone();
    let Ok(mut watcher) = notify::recommended_watcher(move |event| {
        if matches!(
            event_sender.try_send(event),
            Err(std::sync::mpsc::TrySendError::Full(_))
        ) {
            watch_overflowed.store(true, Ordering::Release);
        }
        ui_ctx.request_repaint();
    }) else {
        log::error!("[ORGANIZER] Failed to create watcher");
        return (None, false, HashSet::new());
    };

    let mut ready = true;
    let mut registered_folders = HashSet::new();
    for folder in watched_folders(rules) {
        if !folder.is_dir() {
            ready = false;
            continue;
        }
        if let Err(error) = watcher.watch(&folder, RecursiveMode::NonRecursive) {
            ready = false;
            log::warn!("[ORGANIZER] Failed to watch {}: {error}", folder.display());
        } else {
            registered_folders.insert(normalize_watched_path(&folder));
        }
    }
    (Some(watcher), ready, registered_folders)
}

pub(super) fn watched_folders(rules: &[OrganizerRule]) -> Vec<PathBuf> {
    let mut identities = HashSet::new();
    let mut folders = Vec::new();
    for rule in rules.iter().filter(|rule| rule.enabled) {
        for folder in rule_watch_folders(rule) {
            if identities.insert(normalize_watched_path(folder)) {
                folders.push(folder.to_path_buf());
            }
        }
    }
    folders
}

fn rule_watch_folders(rule: &OrganizerRule) -> impl Iterator<Item = &std::path::Path> {
    [
        Some(rule.source_folder.as_path()),
        Some(rule.destination_folder.as_path()),
        rule.conflict_policy.conflict_folder(),
    ]
    .into_iter()
    .flatten()
}
