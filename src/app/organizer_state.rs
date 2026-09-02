use crate::app::organizer_conflict_state::OrganizerConflictUiState;
use crate::app::organizer_history_state::OrganizerHistoryUiState;
use crate::domain::organizer_operation::OrganizerOperationId;
use crate::domain::organizer_rule::{OrganizerConflictPolicy, OrganizerRule};
use crate::infrastructure::organizer::{
    OrganizerCommandError, OrganizerCommandId, OrganizerCommandResult, OrganizerManager,
    OrganizerRuleStatus,
};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

const NOTIFICATION_IDLE_DELAY: Duration = Duration::from_millis(1500);
const MAX_ISSUE_DETAILS: usize = 3;

#[derive(Default)]
pub struct OrganizerNotificationBatch {
    moved: usize,
    skipped: usize,
    failed: usize,
    issue_details: Vec<String>,
    additional_issues: usize,
    last_event_at: Option<Instant>,
}

pub struct OrganizerNotificationSummary {
    pub moved: usize,
    pub skipped: usize,
    pub failed: usize,
    pub issue_details: Vec<String>,
    pub additional_issues: usize,
}

pub struct OrganizerPreviewResult {
    pub rule_id: i64,
    pub count: usize,
}

impl OrganizerNotificationBatch {
    pub fn record_moved(&mut self) {
        self.record_at(Instant::now(), |batch| batch.moved += 1);
    }

    pub fn record_skipped(&mut self, detail: String) {
        self.record_at(Instant::now(), |batch| {
            batch.skipped += 1;
            batch.record_issue(detail);
        });
    }

    pub fn record_failed(&mut self, detail: String) {
        self.record_at(Instant::now(), |batch| {
            batch.failed += 1;
            batch.record_issue(detail);
        });
    }

    pub fn take_if_idle(&mut self, now: Instant) -> Option<OrganizerNotificationSummary> {
        let last_event_at = self.last_event_at?;
        if now.duration_since(last_event_at) < NOTIFICATION_IDLE_DELAY {
            return None;
        }

        let summary = OrganizerNotificationSummary {
            moved: self.moved,
            skipped: self.skipped,
            failed: self.failed,
            issue_details: std::mem::take(&mut self.issue_details),
            additional_issues: self.additional_issues,
        };
        *self = Self::default();
        Some(summary)
    }

    fn record_at(&mut self, now: Instant, update: impl FnOnce(&mut Self)) {
        update(self);
        self.last_event_at = Some(now);
    }

    fn record_issue(&mut self, detail: String) {
        if self.issue_details.len() < MAX_ISSUE_DETAILS {
            self.issue_details.push(detail);
        } else {
            self.additional_issues += 1;
        }
    }
}

pub struct OrganizerState {
    pub manager: OrganizerManager,
    pub rules: Vec<OrganizerRule>,
    pub rule_statuses: HashMap<i64, OrganizerRuleStatus>,
    pub active_operation_ids: HashSet<OrganizerOperationId>,
    pub(crate) conflict_state: OrganizerConflictUiState,
    pub(crate) history_state: OrganizerHistoryUiState,
    pub source_input: String,
    pub destination_input: String,
    pub extensions_input: String,
    pub conflict_folder_input: String,
    pub editing_rule_id: Option<i64>,
    pub form_enabled: bool,
    pub conflict_policy: OrganizerConflictPolicy,
    pub notification_batch: OrganizerNotificationBatch,
    preview_sender: Sender<OrganizerPreviewResult>,
    pub preview_receiver: Receiver<OrganizerPreviewResult>,
    previewing_rule_ids: HashSet<i64>,
    pub folder_creation_confirmation: Option<OrganizerFolderCreationRequest>,
    pending_rule_sets: HashMap<OrganizerCommandId, Vec<OrganizerRule>>,
}

#[derive(Clone)]
pub struct OrganizerFolderCreationRequest {
    pub rule_id: i64,
    pub source: bool,
}

impl OrganizerState {
    pub(crate) fn new(
        file_operation_sender: crossbeam_channel::Sender<
            crate::workers::file_operation_worker::FileOperationRequest,
        >,
        app_state_db: std::sync::Arc<crate::infrastructure::app_state_db::AppStateDb>,
        rules: Vec<OrganizerRule>,
        ui_ctx: eframe::egui::Context,
    ) -> Self {
        let (preview_sender, preview_receiver) = mpsc::channel();
        if let Err(error) = app_state_db.reconcile_organizer_conflict_resolutions() {
            log::warn!("[ORGANIZER] Failed to reconcile conflict resolutions: {error}");
        }
        if let Err(error) = app_state_db.reconcile_started_organizer_operations() {
            log::warn!("[ORGANIZER] Failed to reconcile interrupted operations: {error}");
        }
        let conflict_state = OrganizerConflictUiState::load(&app_state_db);
        let history_state = OrganizerHistoryUiState::load(&app_state_db);
        let rule_statuses = rules
            .iter()
            .map(|rule| (rule.id, OrganizerRuleStatus::Starting))
            .collect();
        Self {
            manager: OrganizerManager::start(
                file_operation_sender,
                std::sync::Arc::clone(&app_state_db),
                rules.clone(),
                ui_ctx,
            ),
            rules,
            rule_statuses,
            active_operation_ids: HashSet::new(),
            conflict_state,
            history_state,
            source_input: String::new(),
            destination_input: String::new(),
            extensions_input: String::new(),
            conflict_folder_input: String::new(),
            editing_rule_id: None,
            form_enabled: true,
            conflict_policy: OrganizerConflictPolicy::default(),
            notification_batch: OrganizerNotificationBatch::default(),
            preview_sender,
            preview_receiver,
            previewing_rule_ids: HashSet::new(),
            folder_creation_confirmation: None,
            pending_rule_sets: HashMap::new(),
        }
    }

    pub fn reset_form(&mut self) {
        self.source_input.clear();
        self.destination_input.clear();
        self.extensions_input.clear();
        self.conflict_folder_input.clear();
        self.editing_rule_id = None;
        self.form_enabled = true;
        self.conflict_policy = OrganizerConflictPolicy::default();
    }

    pub fn replace_rules(
        &mut self,
        rules: Vec<OrganizerRule>,
    ) -> Result<OrganizerCommandId, OrganizerCommandError> {
        let command_id = self.manager.set_rules(rules.clone())?;
        self.pending_rule_sets.insert(command_id, rules);
        Ok(command_id)
    }

    pub fn command_succeeded(
        &mut self,
        command_id: OrganizerCommandId,
        result: &OrganizerCommandResult,
    ) {
        self.conflict_state.command_finished(command_id);
        self.history_state.command_finished(command_id);
        match result {
            OrganizerCommandResult::ConflictResolved { conflict_id, .. }
            | OrganizerCommandResult::ConflictCancelled { conflict_id } => {
                self.conflict_state.remove(*conflict_id);
            }
            _ => {}
        }
        let Some(rules) = self.pending_rule_sets.remove(&command_id) else {
            return;
        };
        if !matches!(result, OrganizerCommandResult::RulesUpdated { .. }) {
            return;
        }
        self.rule_statuses
            .retain(|rule_id, _| rules.iter().any(|rule| rule.id == *rule_id));
        for rule in &rules {
            self.rule_statuses
                .entry(rule.id)
                .or_insert(OrganizerRuleStatus::Starting);
        }
        self.rules = rules;
    }

    pub fn command_failed(&mut self, command_id: OrganizerCommandId) -> bool {
        self.pending_rule_sets.remove(&command_id);
        self.history_state.command_finished(command_id);
        self.conflict_state.command_finished(command_id)
    }

    pub fn reload_conflicts(
        &mut self,
        db: &crate::infrastructure::app_state_db::AppStateDb,
    ) -> Result<(), crate::infrastructure::app_state_db::OrganizerConflictDbError> {
        self.conflict_state.reload(db)
    }

    pub fn reload_history(
        &mut self,
        db: &crate::infrastructure::app_state_db::AppStateDb,
    ) -> Result<(), crate::infrastructure::app_state_db::OrganizerOperationDbError> {
        self.history_state.reload(db)
    }

    pub fn reload_history_if_dirty(
        &mut self,
        db: &crate::infrastructure::app_state_db::AppStateDb,
    ) {
        self.history_state.reload_if_dirty(db);
    }

    pub fn retry_operation(
        &mut self,
        operation_id: OrganizerOperationId,
    ) -> Result<OrganizerCommandId, OrganizerCommandError> {
        let command_id = self.manager.retry_operation(operation_id)?;
        self.history_state.command_started(command_id, operation_id);
        Ok(command_id)
    }

    pub fn undo_operation(
        &mut self,
        operation_id: OrganizerOperationId,
    ) -> Result<OrganizerCommandId, OrganizerCommandError> {
        let command_id = self.manager.undo_operation(operation_id)?;
        self.history_state.command_started(command_id, operation_id);
        Ok(command_id)
    }

    pub fn is_history_operation_pending(&self, operation_id: OrganizerOperationId) -> bool {
        self.history_state.is_pending(operation_id)
    }

    pub fn resolve_conflict(
        &mut self,
        conflict_id: crate::domain::organizer_conflict::OrganizerConflictId,
        resolution: crate::infrastructure::organizer::OrganizerConflictResolution,
    ) -> Result<OrganizerCommandId, OrganizerCommandError> {
        self.conflict_state
            .resolve(&self.manager, conflict_id, resolution)
    }

    pub fn is_conflict_command_pending(
        &self,
        conflict_id: crate::domain::organizer_conflict::OrganizerConflictId,
    ) -> bool {
        self.conflict_state.is_pending(conflict_id)
    }

    pub fn rule_status(&self, rule_id: i64) -> OrganizerRuleStatus {
        self.rule_statuses
            .get(&rule_id)
            .copied()
            .unwrap_or(OrganizerRuleStatus::Starting)
    }

    pub fn set_rule_status(&mut self, rule_id: i64, status: OrganizerRuleStatus) {
        if self.rules.iter().any(|rule| rule.id == rule_id) {
            self.rule_statuses.insert(rule_id, status);
        }
    }

    pub fn operation_started(&mut self, operation_id: OrganizerOperationId) -> bool {
        let inserted = self.active_operation_ids.insert(operation_id);
        if inserted {
            self.history_state.mark_dirty();
        }
        inserted
    }

    pub fn operation_finished(&mut self, operation_id: OrganizerOperationId) -> bool {
        let removed = self.active_operation_ids.remove(&operation_id);
        if removed {
            self.history_state.mark_dirty();
        }
        removed
    }

    pub fn is_previewing(&self, rule_id: i64) -> bool {
        self.previewing_rule_ids.contains(&rule_id)
    }

    pub fn start_preview(
        &mut self,
        rule: OrganizerRule,
        ui_ctx: eframe::egui::Context,
    ) -> Result<bool, String> {
        if !self.previewing_rule_ids.insert(rule.id) {
            return Ok(false);
        }

        let sender = self.preview_sender.clone();
        let rule_id = rule.id;
        if let Err(error) = crate::spawn_named("organizer-preview", move || {
            let count = crate::domain::organizer_rule::preview_rule(&rule).len();
            let _ = sender.send(OrganizerPreviewResult { rule_id, count });
            ui_ctx.request_repaint();
        }) {
            self.previewing_rule_ids.remove(&rule_id);
            return Err(error.to_string());
        }
        Ok(true)
    }

    pub fn finish_preview(&mut self, rule_id: i64) {
        self.previewing_rule_ids.remove(&rule_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::organizer::{OrganizerCommandResult, OrganizerEvent};

    #[test]
    fn notification_batch_emits_one_summary_after_the_idle_delay() {
        let now = Instant::now();
        let mut batch = OrganizerNotificationBatch::default();
        batch.record_at(now, |batch| batch.moved += 2);
        batch.record_at(now + Duration::from_millis(500), |batch| {
            batch.skipped += 1;
            batch.record_issue("conflict".to_string());
        });

        assert!(batch
            .take_if_idle(now + Duration::from_millis(1500))
            .is_none());

        let summary = batch
            .take_if_idle(now + Duration::from_millis(2000))
            .expect("summary after idle delay");
        assert_eq!(summary.moved, 2);
        assert_eq!(summary.skipped, 1);
        assert_eq!(summary.failed, 0);
        assert_eq!(summary.issue_details, vec!["conflict"]);
        assert!(batch
            .take_if_idle(now + Duration::from_millis(4000))
            .is_none());
    }

    #[test]
    fn notification_batch_limits_issue_details() {
        let now = Instant::now();
        let mut batch = OrganizerNotificationBatch::default();
        for index in 0..(MAX_ISSUE_DETAILS + 2) {
            batch.record_at(now, |batch| {
                batch.failed += 1;
                batch.record_issue(format!("issue {index}"));
            });
        }

        let summary = batch
            .take_if_idle(now + NOTIFICATION_IDLE_DELAY)
            .expect("summary after idle delay");
        assert_eq!(summary.issue_details.len(), MAX_ISSUE_DETAILS);
        assert_eq!(summary.additional_issues, 2);
    }

    #[cfg(feature = "notify-watcher")]
    #[test]
    fn replacement_rules_are_applied_only_after_typed_success() {
        let root = tempfile::tempdir().expect("create test directory");
        let source = root.path().join("source");
        let first_destination = root.path().join("first-destination");
        let second_destination = root.path().join("second-destination");
        std::fs::create_dir(&source).expect("create source directory");
        std::fs::create_dir(&first_destination).expect("create first destination");
        std::fs::create_dir(&second_destination).expect("create second destination");
        let initial = OrganizerRule::new(
            1,
            source.clone(),
            first_destination,
            vec!["txt".to_string()],
            false,
        )
        .expect("create initial rule");
        let replacement = OrganizerRule::new(
            1,
            source,
            second_destination,
            vec!["txt".to_string()],
            false,
        )
        .expect("create replacement rule");
        let (file_operation_sender, _file_operation_receiver) = crossbeam_channel::unbounded();
        let mut state = OrganizerState::new(
            file_operation_sender,
            std::sync::Arc::new(
                crate::infrastructure::app_state_db::AppStateDb::new_in_memory().expect("database"),
            ),
            vec![initial.clone()],
            eframe::egui::Context::default(),
        );

        let command_id = state
            .replace_rules(vec![replacement.clone()])
            .expect("queue replacement");
        assert_eq!(state.rules, vec![initial]);

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match state.manager.try_recv_event() {
                Ok(OrganizerEvent::CommandResult {
                    command_id: received_id,
                    result: Ok(result @ OrganizerCommandResult::RulesUpdated { .. }),
                }) if received_id == command_id => {
                    state.command_succeeded(received_id, &result);
                    break;
                }
                Ok(_) | Err(std::sync::mpsc::TryRecvError::Empty) if Instant::now() < deadline => {
                    std::thread::yield_now();
                }
                _ => panic!("missing rules confirmation"),
            }
        }
        assert_eq!(state.rules, vec![replacement]);
    }

    #[cfg(feature = "notify-watcher")]
    #[test]
    fn rejected_rules_never_replace_visible_state() {
        let root = tempfile::tempdir().expect("create test directory");
        let first = root.path().join("first");
        let second = root.path().join("second");
        std::fs::create_dir(&first).expect("create first directory");
        std::fs::create_dir(&second).expect("create second directory");
        let initial = OrganizerRule::new(
            1,
            first.clone(),
            second.clone(),
            vec!["pdf".to_string()],
            false,
        )
        .expect("create initial rule");
        let forward = OrganizerRule::new(
            2,
            first.clone(),
            second.clone(),
            vec!["txt".to_string()],
            true,
        )
        .expect("create forward rule");
        let reverse = OrganizerRule::new(3, second, first, vec!["txt".to_string()], true)
            .expect("create reverse rule");
        let (file_operation_sender, _file_operation_receiver) = crossbeam_channel::unbounded();
        let mut state = OrganizerState::new(
            file_operation_sender,
            std::sync::Arc::new(
                crate::infrastructure::app_state_db::AppStateDb::new_in_memory().expect("database"),
            ),
            vec![initial.clone()],
            eframe::egui::Context::default(),
        );

        let command_id = state
            .replace_rules(vec![forward, reverse])
            .expect("queue invalid replacement");
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match state.manager.try_recv_event() {
                Ok(OrganizerEvent::CommandResult {
                    command_id: received_id,
                    result: Err(_),
                }) if received_id == command_id => {
                    state.command_failed(received_id);
                    break;
                }
                Ok(_) | Err(std::sync::mpsc::TryRecvError::Empty) if Instant::now() < deadline => {
                    std::thread::yield_now();
                }
                _ => panic!("missing rules rejection"),
            }
        }
        assert_eq!(state.rules, vec![initial]);
    }
}
