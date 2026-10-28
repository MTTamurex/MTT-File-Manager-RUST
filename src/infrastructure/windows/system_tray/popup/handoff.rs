//! Continue a popup in a surviving process before its current host exits.
use super::super::{
    commands, coordinator,
    model::{Packet, PopupContinuation, PopupInstance},
    protocol, POPUP_CONTINUE_MESSAGE,
};
use super::{Model, State, WindowRef, ACTIVE};
use std::{cell::RefCell, collections::HashSet};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, PostMessageW};

struct Pending {
    source: WindowRef,
    destination: WindowRef,
    view: PopupContinuation,
}
thread_local! { static PENDING: RefCell<Option<Pending>> = const { RefCell::new(None) }; }

pub(super) fn capture(state: &State) -> PopupContinuation {
    PopupContinuation {
        anchor: [state.anchor.x, state.anchor.y],
        scroll: state.model.scroll,
        windows: state
            .model
            .windows
            .iter()
            .filter(|(window, _)| window.is_current())
            .map(|(window, snapshot)| PopupInstance {
                handle: window.hwnd.0 as usize,
                process_id: window.process_id(),
                snapshot: snapshot.clone(),
                expanded: state.model.expanded.contains(&(window.hwnd.0 as usize)),
            })
            .collect(),
    }
}

fn reference(instance: &PopupInstance) -> Option<WindowRef> {
    WindowRef::capture(HWND(instance.handle as *mut _))
        .filter(|window| window.process_id() == instance.process_id)
}

pub(super) fn send(owner: HWND, view: PopupContinuation) -> bool {
    let Some(destination) = view
        .windows
        .iter()
        .filter_map(reference)
        .find(|window| window.hwnd != owner)
    else {
        return false;
    };
    // A timed-out send may already have been delivered. Never start a second
    // continuation in a different process for the same close request.
    let packet = Packet::ContinuePopup(view);
    unsafe {
        let _ = AllowSetForegroundWindow(destination.process_id());
    }
    protocol::send(owner, destination.hwnd, &packet)
}

pub(in super::super) fn receive(sender: HWND, view: PopupContinuation) -> bool {
    let Some(source) = WindowRef::capture(sender) else {
        return false;
    };
    let Some(destination) = coordinator::local_window().and_then(WindowRef::capture) else {
        return false;
    };
    if source == destination
        || ACTIVE.with(|active| active.get())
        || view.windows.len() > 128
        || PENDING.with(|pending| pending.borrow().is_some())
    {
        return false;
    }
    let identities: HashSet<_> = view
        .windows
        .iter()
        .filter_map(reference)
        .map(|window| (window.hwnd.0 as usize, window.process_id()))
        .collect();
    if identities.len() != view.windows.len()
        || !identities.contains(&(sender.0 as usize, source.process_id()))
        || !identities.contains(&(destination.hwnd.0 as usize, destination.process_id()))
    {
        return false;
    }
    PENDING.with(|pending| {
        *pending.borrow_mut() = Some(Pending {
            source,
            destination,
            view,
        })
    });
    if unsafe {
        PostMessageW(
            Some(destination.hwnd),
            POPUP_CONTINUE_MESSAGE,
            WPARAM(0),
            LPARAM(0),
        )
    }
    .is_err()
    {
        PENDING.with(|pending| pending.borrow_mut().take());
        return false;
    }
    true
}

fn take_model(hwnd: HWND) -> Option<(Model, POINT)> {
    let pending = PENDING.with(|pending| pending.borrow_mut().take())?;
    if pending.destination.hwnd != hwnd || !pending.destination.is_current() {
        return None;
    }
    // Only this accepted continuation may close the old host. A failed handoff
    // never terminates it, and HWND reuse must not target an unrelated process.
    if pending.source.is_current() && !commands::exit_window(hwnd, pending.source) {
        log::warn!("[TRAY] Could not close the previous popup host");
    }
    let expanded = pending
        .view
        .windows
        .iter()
        .filter(|instance| instance.expanded)
        .filter_map(reference)
        .map(|window| window.hwnd.0 as usize)
        .collect();
    let windows = pending
        .view
        .windows
        .into_iter()
        .filter_map(|instance| reference(&instance).map(|window| (window, instance.snapshot)))
        .collect();
    let mut model = Model::new(windows);
    model.expanded = expanded;
    model.scroll = pending.view.scroll.max(0);
    model.rebuild();
    Some((
        model,
        POINT {
            x: pending.view.anchor[0],
            y: pending.view.anchor[1],
        },
    ))
}

pub(in super::super) fn show_pending(hwnd: HWND) {
    if let Some((model, anchor)) = take_model(hwnd) {
        if !super::show_at(hwnd, model, anchor) {
            super::menu::show(hwnd);
        }
    }
}

#[cfg(test)]
mod tests;
