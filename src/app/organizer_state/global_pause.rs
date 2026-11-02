use crate::infrastructure::app_state_db::AppStateDb;
use crate::infrastructure::organizer::{
    OrganizerCommandError, OrganizerCommandId, OrganizerCommandResult, OrganizerManager,
};

pub struct OrganizerGlobalPauseUiState {
    pub paused: bool,
    pending: Option<OrganizerCommandId>,
}

impl OrganizerGlobalPauseUiState {
    pub(super) fn load(db: &AppStateDb) -> Self {
        let paused = db.organizer_globally_paused().unwrap_or(true);
        Self {
            paused,
            pending: None,
        }
    }

    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub fn individual_toggles_enabled(&self) -> bool {
        !self.paused && !self.is_pending()
    }

    pub fn toggle(&mut self, manager: &OrganizerManager) -> Result<(), OrganizerCommandError> {
        if self.pending.is_none() {
            self.pending = Some(manager.set_global_pause(!self.paused)?);
        }
        Ok(())
    }

    pub(super) fn command_succeeded(
        &mut self,
        command_id: OrganizerCommandId,
        result: &OrganizerCommandResult,
    ) {
        if let OrganizerCommandResult::GlobalPauseSet { paused } = result {
            self.paused = *paused;
        }
        self.command_failed(command_id);
    }

    pub(super) fn command_failed(&mut self, command_id: OrganizerCommandId) {
        if self.pending == Some(command_id) {
            self.pending = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn organizer_pause_ui_restores_saved_state_and_fails_closed() {
        let db = AppStateDb::new_in_memory().unwrap();
        assert!(!OrganizerGlobalPauseUiState::load(&db).paused);
        db.set_organizer_globally_paused(true).unwrap();
        let state = OrganizerGlobalPauseUiState::load(&db);
        assert!(state.paused);
        assert!(!state.is_pending());
        db.set_organizer_globally_paused(false).unwrap();
        assert!(!OrganizerGlobalPauseUiState::load(&db).paused);
        db.set_preference("organizer_globally_paused", "invalid")
            .unwrap();
        assert!(OrganizerGlobalPauseUiState::load(&db).paused);
    }

    #[cfg(feature = "notify-watcher")]
    #[test]
    fn organizer_pause_ui_waits_for_confirmation_and_recovers_from_failure() {
        let db = std::sync::Arc::new(AppStateDb::new_in_memory().unwrap());
        let (sender, _receiver) = crossbeam_channel::unbounded();
        let manager = OrganizerManager::start(
            sender,
            std::sync::Arc::clone(&db),
            Vec::new(),
            eframe::egui::Context::default(),
        );
        let mut state = OrganizerGlobalPauseUiState::load(&db);
        state.toggle(&manager).unwrap();
        let command_id = state.pending.unwrap();
        assert!(!state.paused);
        assert!(!state.individual_toggles_enabled());
        assert!(state.is_pending());
        state.toggle(&manager).unwrap();
        assert_eq!(state.pending, Some(command_id));
        state.command_succeeded(
            command_id,
            &OrganizerCommandResult::GlobalPauseSet { paused: true },
        );
        assert!(state.paused);
        assert!(!state.is_pending());
        assert!(!state.individual_toggles_enabled());
        state.toggle(&manager).unwrap();
        let command_id = state.pending.unwrap();
        state.command_failed(command_id);
        assert!(state.paused);
        assert!(!state.is_pending());
    }
}
