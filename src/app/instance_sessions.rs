//! Explicit restart policy: last individual close, or the whole global-exit set.
mod record;
mod startup;
pub(crate) use record::save;
pub use record::KEY;
pub(crate) use record::{exit_order, ExitOrder};
pub(crate) use startup::{confirm_startup, poll_restore_failure, restoring, startup_session};
pub use startup::{prepare_startup, receive_startup};

pub(crate) fn save_individual(app: &crate::app::ImageViewerApp, order: ExitOrder) -> bool {
    if !startup::ready()
        || crate::infrastructure::windows::system_tray::session_exit::is_group_exit()
    {
        return true;
    }
    matches!(
        record::save_at(
            &app.app_state_db,
            vec![crate::tabs::TabManagerSession::from_tab_manager(
                &app.tab_manager,
            )],
            order,
        ),
        record::SaveOutcome::Saved | record::SaveOutcome::Superseded
    )
}
