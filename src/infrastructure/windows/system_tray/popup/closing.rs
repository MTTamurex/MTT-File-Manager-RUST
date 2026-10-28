//! Individual exits are non-dismissing commands, unlike activation/global exit.
use super::super::{commands, model::PopupContinuation};
use super::{handoff, menu::Command, State, WindowRef};
use windows::Win32::Foundation::HWND;

pub(super) struct Plan {
    owner: HWND,
    target: WindowRef,
    continuation: Option<PopupContinuation>,
}

pub(super) enum Result {
    Closed(WindowRef),
    Transferred,
    Failed,
}

pub(super) fn take_plan(state: &mut State) -> Option<Plan> {
    if state.done || !matches!(state.command, Some(Command::Exit(_))) {
        return None;
    }
    let Some(Command::Exit(target)) = state.command.take() else {
        return None;
    };
    let continuation = (target.hwnd == state.owner
        && state
            .model
            .windows
            .iter()
            .any(|(window, _)| *window != target && window.is_current()))
    .then(|| handoff::capture(state));
    state.pressed = None;
    state.hot_close = None;
    Some(Plan {
        owner: state.owner,
        target,
        continuation,
    })
}

pub(super) fn execute(plan: Plan) -> Result {
    if let Some(continuation) = plan.continuation {
        if handoff::send(plan.owner, continuation) {
            return Result::Transferred;
        }
    } else if !plan.target.is_current() || commands::exit_window(plan.owner, plan.target) {
        return Result::Closed(plan.target);
    }
    log::warn!("[TRAY] Individual close was not acknowledged; retaining the menu entry");
    Result::Failed
}

pub(super) fn apply(state: &mut State, result: Result) {
    match result {
        Result::Closed(window) => {
            state.model.remove_window(window);
            state.resize = true;
            state.done |= state.model.windows.is_empty();
        }
        Result::Transferred => state.done = true,
        Result::Failed => {}
    }
}

#[cfg(test)]
mod tests;
