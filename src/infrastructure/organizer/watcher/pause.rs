use super::commands::CommandContext;
use super::queue::queue_rule_paths;
use super::*;
use crate::infrastructure::app_state_db::AppStateDb;

pub(super) fn effective_paused_rules(
    rules: &[OrganizerRule],
    globally_paused: bool,
) -> HashSet<i64> {
    if globally_paused {
        rules
            .iter()
            .filter(|rule| rule.enabled)
            .map(|rule| rule.id)
            .collect()
    } else {
        HashSet::new()
    }
}

pub(super) fn load_pause_state(db: &AppStateDb, events: &Sender<OrganizerEvent>) -> bool {
    match db.organizer_globally_paused() {
        Ok(global) => global,
        Err(error) => {
            let reason = error.to_string();
            let _ = events.send(OrganizerEvent::Error {
                message: OrganizerCommandError::PausePersistenceFailed { reason }.to_string(),
            });
            // Never start organizing when the persisted pause state cannot be read.
            true
        }
    }
}

impl CommandContext<'_> {
    pub(super) fn refresh_paused_rules(&mut self) {
        *self.paused_rules = effective_paused_rules(self.rules, *self.globally_paused);
    }

    pub(super) fn set_global_pause(
        &mut self,
        command_id: OrganizerCommandId,
        paused: bool,
    ) -> bool {
        if let Err(error) = self.app_state_db.set_organizer_globally_paused(paused) {
            return self.pause_write_failed(command_id, error);
        }
        *self.globally_paused = paused;
        self.refresh_paused_rules();
        // Do not invalidate activation tokens: an already-running move must finish.
        // The file worker checks the persisted global pause before starting each move.
        if !paused {
            for rule in self
                .rules
                .iter()
                .filter(|rule| rule.enabled && !self.paused_rules.contains(&rule.id))
            {
                queue_rule_paths(
                    rule,
                    self.rules,
                    self.activation_flags,
                    self.paused_rules,
                    self.pending,
                );
            }
        }
        self.respond(
            command_id,
            Ok(OrganizerCommandResult::GlobalPauseSet { paused }),
        )
    }

    fn pause_write_failed(
        &self,
        command_id: OrganizerCommandId,
        error: crate::infrastructure::app_state_db::AppStateWriteError,
    ) -> bool {
        self.respond(
            command_id,
            Err(OrganizerCommandError::PausePersistenceFailed {
                reason: error.to_string(),
            }),
        )
    }
}

#[cfg(test)]
#[path = "pause_tests.rs"]
mod tests;
