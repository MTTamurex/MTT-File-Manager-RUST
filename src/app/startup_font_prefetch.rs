//! Starts the main window's font loading before graphics initialization.
//!
//! Loading system fonts takes a few hundred milliseconds and only depends on
//! the saved font preference, so `main` starts it while the GPU device and
//! window are being created. App initialization then takes the receiver.

use super::init_workers::spawn_async_font_loader;
use crate::ui::fonts::selection::LoadedFonts;
use std::sync::mpsc::Receiver;
use std::sync::Mutex;

struct Prefetched {
    selected: String,
    receiver: Receiver<LoadedFonts>,
}

static PREFETCHED: Mutex<Option<Prefetched>> = Mutex::new(None);

/// Preference key holding the selected UI font family.
pub const PREFERENCE_KEY: &str = crate::ui::fonts::selection::PREFERENCE_KEY;

/// Starts loading fonts for `selected` on a background thread.
pub fn start(selected: String) {
    let receiver = spawn_async_font_loader(selected.clone());
    if let Ok(mut slot) = PREFETCHED.lock() {
        *slot = Some(Prefetched { selected, receiver });
    }
}

/// Returns the prefetched loader for `selected`, or starts a new one when
/// nothing was prefetched or the preference differs.
pub(crate) fn take_or_spawn(selected: String) -> Receiver<LoadedFonts> {
    let prefetched = PREFETCHED.lock().ok().and_then(|mut slot| slot.take());
    match prefetched {
        Some(prefetched) if prefetched.selected == selected => prefetched.receiver,
        _ => spawn_async_font_loader(selected),
    }
}
