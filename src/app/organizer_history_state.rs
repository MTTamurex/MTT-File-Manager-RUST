use crate::domain::organizer_operation::OrganizerOperationId;
use crate::infrastructure::app_state_db::{
    AppStateDb, OrganizerOperationDbError, OrganizerOperationRecord,
};
use crate::infrastructure::organizer::OrganizerCommandId;
use std::collections::HashMap;

pub(crate) const MAX_VISIBLE_ORGANIZER_OPERATIONS: usize = 200;

pub(crate) struct OrganizerHistoryUiState {
    pub operations: Vec<OrganizerOperationRecord>,
    pub load_error: Option<String>,
    pub retention_days: i64,
    pub retention_input: String,
    pub retention_input_dirty: bool,
    pending_commands: HashMap<OrganizerCommandId, OrganizerOperationId>,
    dirty: bool,
    loaded_revision: u64,
}

impl OrganizerHistoryUiState {
    pub fn load(db: &AppStateDb) -> Self {
        let retention_days = db.organizer_history_retention_days();
        let mut state = Self {
            operations: Vec::new(),
            load_error: None,
            retention_days,
            retention_input: retention_days.to_string(),
            retention_input_dirty: false,
            pending_commands: HashMap::new(),
            dirty: false,
            loaded_revision: db.organizer_history_revision(),
        };
        let _ = state.reload(db);
        state
    }

    pub fn reload(&mut self, db: &AppStateDb) -> Result<(), OrganizerOperationDbError> {
        let (operations, retention_days, loaded_revision) = loop {
            let revision_before = db.organizer_history_revision();
            let operations = match db.list_organizer_operations(MAX_VISIBLE_ORGANIZER_OPERATIONS) {
                Ok(operations) => operations,
                Err(error) => {
                    self.load_error = Some(error.to_string());
                    self.dirty = false;
                    return Err(error);
                }
            };
            let retention_days = db.organizer_history_retention_days();
            let revision_after = db.organizer_history_revision();
            if revision_before == revision_after {
                break (operations, retention_days, revision_after);
            }
        };
        self.operations = operations;
        self.retention_days = retention_days;
        if !self.retention_input_dirty {
            self.retention_input = self.retention_days.to_string();
        }
        self.load_error = None;
        self.dirty = false;
        self.loaded_revision = loaded_revision;
        Ok(())
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn reload_if_dirty(&mut self, db: &AppStateDb) {
        if self.dirty || self.loaded_revision != db.organizer_history_revision() {
            let _ = self.reload(db);
        }
    }

    pub fn command_started(
        &mut self,
        command_id: OrganizerCommandId,
        operation_id: OrganizerOperationId,
    ) {
        self.pending_commands.insert(command_id, operation_id);
    }

    pub fn command_finished(&mut self, command_id: OrganizerCommandId) {
        if self.pending_commands.remove(&command_id).is_some() {
            self.mark_dirty();
        }
    }

    pub fn is_pending(&self, operation_id: OrganizerOperationId) -> bool {
        self.pending_commands
            .values()
            .any(|pending_id| *pending_id == operation_id)
    }
}
