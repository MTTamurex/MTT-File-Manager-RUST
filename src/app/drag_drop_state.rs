use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutboundDragInputGuard {
    #[default]
    Inactive,
    WaitingForRelease,
    WaitingForNewPress,
}

impl OutboundDragInputGuard {
    pub fn armed(primary_down: bool) -> Self {
        if primary_down {
            Self::WaitingForRelease
        } else {
            Self::WaitingForNewPress
        }
    }

    pub fn update(self, primary_down: bool, primary_press_received_by_egui: bool) -> Self {
        match self {
            Self::WaitingForRelease if primary_press_received_by_egui => Self::Inactive,
            Self::WaitingForRelease if !primary_down => Self::WaitingForNewPress,
            Self::WaitingForNewPress if primary_down && primary_press_received_by_egui => {
                Self::Inactive
            }
            current => current,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PendingDragMoveConfirmation {
    pub paths: Vec<PathBuf>,
    pub dest_folder: PathBuf,
    pub source_folder: Option<PathBuf>,
}

impl PendingDragMoveConfirmation {
    pub fn new(paths: Vec<PathBuf>, dest_folder: PathBuf, source_folder: Option<PathBuf>) -> Self {
        Self {
            paths,
            dest_folder,
            source_folder,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::OutboundDragInputGuard;

    #[test]
    fn outbound_guard_requires_release_then_a_new_press() {
        let guard = OutboundDragInputGuard::armed(true);
        assert_eq!(
            guard.update(true, false),
            OutboundDragInputGuard::WaitingForRelease
        );

        let guard = guard.update(false, false);
        assert_eq!(guard, OutboundDragInputGuard::WaitingForNewPress);
        assert_eq!(
            guard.update(false, false),
            OutboundDragInputGuard::WaitingForNewPress
        );
        assert_eq!(
            guard.update(true, false),
            OutboundDragInputGuard::WaitingForNewPress
        );
        assert_eq!(guard.update(true, true), OutboundDragInputGuard::Inactive);
    }

    #[test]
    fn new_egui_press_proves_a_missed_release_occurred() {
        assert_eq!(
            OutboundDragInputGuard::WaitingForRelease.update(true, true),
            OutboundDragInputGuard::Inactive
        );
    }

    #[test]
    fn outbound_guard_arms_for_new_press_when_button_is_already_up() {
        assert_eq!(
            OutboundDragInputGuard::armed(false),
            OutboundDragInputGuard::WaitingForNewPress
        );
    }

    #[test]
    fn native_window_drag_does_not_release_outbound_guard() {
        let guard = OutboundDragInputGuard::WaitingForNewPress;

        // A non-client window drag changes the physical button state but does
        // not produce a primary PointerButton event for egui.
        assert_eq!(
            guard.update(true, false),
            OutboundDragInputGuard::WaitingForNewPress
        );
        assert_eq!(
            guard.update(false, false),
            OutboundDragInputGuard::WaitingForNewPress
        );
    }
}
