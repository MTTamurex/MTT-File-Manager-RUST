use crate::domain::organizer_conflict::OrganizerConflictId;
use crate::infrastructure::app_state_db::{
    AppStateDb, OrganizerConflictDbError, OrganizerConflictRecord,
};
use crate::infrastructure::organizer::{
    OrganizerCommandError, OrganizerCommandId, OrganizerConflictResolution, OrganizerManager,
};
use std::collections::{HashMap, HashSet};
use std::path::Path;

const MAX_VISIBLE_CONFLICTS: usize = 200;

pub(crate) struct OrganizerConflictUiState {
    pub conflicts: Vec<OrganizerConflictRecord>,
    pub source_inputs: HashMap<OrganizerConflictId, String>,
    pub destination_inputs: HashMap<OrganizerConflictId, String>,
    pending_commands: HashMap<OrganizerCommandId, OrganizerConflictId>,
    pub load_error: Option<String>,
    pub destination_confirmation: Option<OrganizerConflictId>,
}

impl OrganizerConflictUiState {
    pub fn load(db: &AppStateDb) -> Self {
        let (conflicts, load_error) = match pending_conflicts(db) {
            Ok(conflicts) => (conflicts, None),
            Err(error) => {
                log::warn!("[ORGANIZER] Failed to load pending conflicts: {error}");
                (Vec::new(), Some(error.to_string()))
            }
        };
        Self {
            source_inputs: conflict_name_inputs(&conflicts, true),
            destination_inputs: conflict_name_inputs(&conflicts, false),
            conflicts,
            pending_commands: HashMap::new(),
            load_error,
            destination_confirmation: None,
        }
    }

    pub fn reload(&mut self, db: &AppStateDb) -> Result<(), OrganizerConflictDbError> {
        let conflicts = match pending_conflicts(db) {
            Ok(conflicts) => conflicts,
            Err(error) => {
                self.load_error = Some(error.to_string());
                return Err(error);
            }
        };
        self.load_error = None;
        self.conflicts = conflicts;
        let ids = self
            .conflicts
            .iter()
            .map(|conflict| conflict.conflict_id)
            .collect::<HashSet<_>>();
        self.source_inputs
            .retain(|conflict_id, _| ids.contains(conflict_id));
        self.destination_inputs
            .retain(|conflict_id, _| ids.contains(conflict_id));
        for conflict in &self.conflicts {
            self.source_inputs
                .entry(conflict.conflict_id)
                .or_insert_with(|| suggested_conflict_name(&conflict.source_path));
            self.destination_inputs
                .entry(conflict.conflict_id)
                .or_insert_with(|| suggested_conflict_name(&conflict.destination_path));
        }
        Ok(())
    }

    pub fn resolve(
        &mut self,
        manager: &OrganizerManager,
        conflict_id: OrganizerConflictId,
        resolution: OrganizerConflictResolution,
    ) -> Result<OrganizerCommandId, OrganizerCommandError> {
        let command_id = manager.resolve_conflict(conflict_id, resolution)?;
        self.pending_commands.insert(command_id, conflict_id);
        Ok(command_id)
    }

    pub fn command_finished(&mut self, command_id: OrganizerCommandId) -> bool {
        self.pending_commands.remove(&command_id).is_some()
    }

    pub fn remove(&mut self, conflict_id: OrganizerConflictId) {
        self.conflicts
            .retain(|conflict| conflict.conflict_id != conflict_id);
        self.source_inputs.remove(&conflict_id);
        self.destination_inputs.remove(&conflict_id);
        if self.destination_confirmation == Some(conflict_id) {
            self.destination_confirmation = None;
        }
    }

    pub fn is_pending(&self, conflict_id: OrganizerConflictId) -> bool {
        self.pending_commands
            .values()
            .any(|pending_id| *pending_id == conflict_id)
    }
}

fn pending_conflicts(
    db: &AppStateDb,
) -> Result<Vec<OrganizerConflictRecord>, OrganizerConflictDbError> {
    db.list_pending_organizer_conflicts(MAX_VISIBLE_CONFLICTS)
}

fn suggested_conflict_name(path: &Path) -> String {
    let Some(file_name) = path.file_name() else {
        return String::new();
    };
    let file_name = file_name.to_string_lossy();
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy())
        .unwrap_or_else(|| file_name.clone());
    let extension = path
        .extension()
        .map(|extension| extension.to_string_lossy().into_owned());
    for index in 1..=9999 {
        let suffix = format!(" ({index})");
        let extension_suffix = extension
            .as_deref()
            .map(|extension| format!(".{extension}"))
            .unwrap_or_default();
        let reserved_units =
            suffix.encode_utf16().count() + extension_suffix.encode_utf16().count();
        let stem_limit = 255usize.saturating_sub(reserved_units);
        let mut truncated_stem = String::new();
        let mut used_units = 0usize;
        for character in stem.chars() {
            let units = character.len_utf16();
            if used_units + units > stem_limit {
                break;
            }
            truncated_stem.push(character);
            used_units += units;
        }
        let candidate = format!("{truncated_stem}{suffix}{extension_suffix}");
        if path
            .parent()
            .is_none_or(|parent| !parent.join(&candidate).exists())
        {
            return candidate;
        }
    }
    format!("{file_name}.renamed")
}

fn conflict_name_inputs(
    conflicts: &[OrganizerConflictRecord],
    source: bool,
) -> HashMap<OrganizerConflictId, String> {
    conflicts
        .iter()
        .map(|conflict| {
            (
                conflict.conflict_id,
                suggested_conflict_name(if source {
                    &conflict.source_path
                } else {
                    &conflict.destination_path
                }),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::suggested_conflict_name;

    #[test]
    fn suggested_name_uses_the_first_available_suffix() {
        let root = tempfile::tempdir().expect("test directory");
        let original = root.path().join("report.txt");
        std::fs::write(root.path().join("report (1).txt"), b"occupied").expect("create collision");

        assert_eq!(suggested_conflict_name(&original), "report (2).txt");
    }

    #[test]
    fn suggested_name_respects_the_windows_component_limit() {
        let root = tempfile::tempdir().expect("test directory");
        let original = root.path().join(format!("{}.txt", "a".repeat(251)));
        let suggestion = suggested_conflict_name(&original);

        assert!(suggestion.encode_utf16().count() <= 255);
        assert!(suggestion.ends_with(" (1).txt"));
    }
}
