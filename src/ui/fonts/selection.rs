//! UI font preference and asynchronous switching. The original definitions are
//! retained so resetting never loses icon fonts or multilingual fallbacks.

use super::catalog::Catalog;
use super::sizing::{self, FontSizes};
use eframe::egui;
use std::sync::{mpsc, Arc, Mutex};

pub(crate) const PREFERENCE_KEY: &str = "ui_font_family";

pub(crate) struct LoadedFonts {
    base: egui::FontDefinitions,
    fonts: egui::FontDefinitions,
    catalog: Arc<Catalog>,
    selected: String,
    unavailable: bool,
}

impl LoadedFonts {
    pub fn into_definitions(self) -> egui::FontDefinitions {
        self.fonts
    }

    pub fn load(base: egui::FontDefinitions, selected: String) -> Self {
        let catalog = Arc::new(Catalog::discover());
        Self::with_catalog(base, selected, catalog)
    }

    fn with_catalog(base: egui::FontDefinitions, selected: String, catalog: Arc<Catalog>) -> Self {
        let mut fonts = base.clone();
        let unavailable = !catalog.apply(&mut fonts, &selected);
        Self {
            base,
            fonts,
            catalog,
            selected,
            unavailable,
        }
    }
}

struct Pending {
    selected: String,
    receiver: mpsc::Receiver<Option<egui::FontDefinitions>>,
}

struct Request {
    selected: String,
    commit: bool,
}

struct State {
    base: egui::FontDefinitions,
    catalog: Arc<Catalog>,
    selected: String,
    committed_fonts: egui::FontDefinitions,
    displayed_family: String,
    queued_fonts: Option<egui::FontDefinitions>,
    committed_unavailable: bool,
    sizes: FontSizes,
    pending: Option<Pending>,
    desired: Option<Request>,
    preview_pass: Option<u64>,
    unavailable: bool,
    save_failed: bool,
    metrics_changed_at: Option<u64>,
}

fn state(ctx: &egui::Context) -> Option<Arc<Mutex<State>>> {
    ctx.data(|data| data.get_temp(egui::Id::new(PREFERENCE_KEY)))
}

pub(crate) fn install(ctx: &egui::Context, loaded: LoadedFonts, sizes: FontSizes) {
    ctx.data_mut(|data| {
        data.insert_temp(
            egui::Id::new(PREFERENCE_KEY),
            Arc::new(Mutex::new(State {
                base: loaded.base,
                catalog: loaded.catalog,
                displayed_family: loaded.selected.clone(),
                queued_fonts: Some(loaded.fonts.clone()),
                selected: loaded.selected,
                committed_fonts: loaded.fonts,
                committed_unavailable: loaded.unavailable,
                sizes,
                pending: None,
                desired: None,
                preview_pass: None,
                unavailable: loaded.unavailable,
                save_failed: false,
                metrics_changed_at: None,
            })),
        )
    });
    ctx.request_repaint();
}

pub(crate) struct Snapshot {
    pub catalog: Arc<Catalog>,
    pub selected: String,
    pub loading: bool,
    pub committing: bool,
    pub unavailable: bool,
    pub save_failed: bool,
    pub scale: f32,
}

pub(crate) fn snapshot(ctx: &egui::Context) -> Option<Snapshot> {
    let state = state(ctx)?;
    let state = state.lock().ok()?;
    Some(Snapshot {
        catalog: Arc::clone(&state.catalog),
        selected: state.selected.clone(),
        loading: state.desired.is_some(),
        committing: state.desired.as_ref().is_some_and(|request| request.commit),
        unavailable: state.unavailable,
        save_failed: state.save_failed,
        scale: state.sizes.get(&state.displayed_family),
    })
}

pub(crate) fn request(ctx: &egui::Context, selected: String) {
    let Some(state) = state(ctx) else { return };
    let Ok(mut state) = state.lock() else { return };
    state.preview_pass = None;
    state.desired = Some(Request {
        selected,
        commit: true,
    });
    ctx.request_repaint();
}

/// Called only while the dropdown is rendered. Previewing never writes a
/// preference; only an explicit confirmation promotes a request to a commit.
pub(crate) fn preview_frame(ctx: &egui::Context) {
    let Some(state) = state(ctx) else { return };
    let Ok(mut state) = state.lock() else { return };
    state.preview_pass = Some(ctx.cumulative_pass_nr());
}

pub(crate) fn preview(ctx: &egui::Context, selected: String) {
    let Some(state) = state(ctx) else { return };
    let Ok(mut state) = state.lock() else { return };
    state.desired = Some(Request {
        selected,
        commit: false,
    });
    ctx.request_repaint();
}

pub(crate) fn cancel_preview(ctx: &egui::Context) {
    let Some(state) = state(ctx) else { return };
    let Ok(mut state) = state.lock() else { return };
    if state.preview_pass.take().is_some() {
        state.desired = None;
        state.unavailable = state.committed_unavailable;
        let fonts = state.committed_fonts.clone();
        let selected = state.selected.clone();
        apply(ctx, &mut state, fonts, &selected);
    }
}

/// Also cancel if a different settings page was selected or the widget vanished.
pub(crate) fn finish_preview_frame(ctx: &egui::Context) {
    let stale = state(ctx).is_some_and(|state| {
        state.lock().is_ok_and(|state| {
            state
                .preview_pass
                .is_some_and(|pass| pass != ctx.cumulative_pass_nr())
        })
    });
    if stale {
        cancel_preview(ctx);
    }
}

// Only one read is in flight. Fast navigation replaces `desired` rather than
// spawning a thread (and retaining another font file) for every hovered row.
fn start_request(ctx: &egui::Context, state: &mut State) {
    if state.pending.is_some() {
        return;
    }
    let Some(request) = state.desired.as_ref() else {
        return;
    };
    let selected = request.selected.clone();
    let commit = request.commit;
    let catalog = Arc::clone(&state.catalog);
    let mut fonts = state.base.clone();
    let requested = selected.clone();
    let (sender, receiver) = mpsc::channel();
    let repaint = ctx.clone();
    let spawn = std::thread::Builder::new()
        .name("ui-font-loader".into())
        .spawn(move || {
            let result = catalog.apply(&mut fonts, &requested).then_some(fonts);
            let _ = sender.send(result);
            repaint.request_repaint();
        });
    if spawn.is_ok() {
        state.pending = Some(Pending { selected, receiver });
    } else {
        state.desired = None;
        state.unavailable = true;
        if commit {
            let fonts = state.committed_fonts.clone();
            let selected = state.selected.clone();
            apply(ctx, state, fonts, &selected);
        }
    }
    ctx.request_repaint();
}

/// Persist only successfully loaded selections; a read failure leaves the
/// previously working font and its saved preference untouched.
pub(crate) fn poll(ctx: &egui::Context, db: &crate::infrastructure::app_state_db::AppStateDb) {
    let Some(state) = state(ctx) else { return };
    let Ok(mut state) = state.lock() else { return };
    // set_fonts takes effect on the next egui pass. Discard measurements made
    // in the intervening pass with the previous font as well.
    if state
        .metrics_changed_at
        .is_some_and(|pass| ctx.cumulative_pass_nr() > pass)
    {
        clear_metrics();
        state.metrics_changed_at = None;
    }
    let Some(pending) = state.pending.as_ref() else {
        start_request(ctx, &mut state);
        return;
    };
    let result = match pending.receiver.try_recv() {
        Ok(result) => result,
        Err(mpsc::TryRecvError::Empty) => {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
            return;
        }
        Err(mpsc::TryRecvError::Disconnected) => None,
    };
    let pending = state.pending.take().expect("pending font request");
    // A result from before cancellation or from a row already passed over must
    // never replace the latest preview or the confirmed font.
    if state
        .desired
        .as_ref()
        .is_none_or(|request| request.selected != pending.selected)
    {
        start_request(ctx, &mut state);
        return;
    }
    let request = state.desired.take().expect("desired font request");
    state.unavailable = result.is_none();
    if let Some(fonts) = result {
        if request.commit {
            state.save_failed = db
                .set_preference(PREFERENCE_KEY, &request.selected)
                .is_err();
            state.selected = request.selected.clone();
            state.committed_fonts = fonts.clone();
            state.committed_unavailable = false;
        }
        apply(ctx, &mut state, fonts, &request.selected);
    } else if request.commit {
        let fonts = state.committed_fonts.clone();
        let selected = state.selected.clone();
        apply(ctx, &mut state, fonts, &selected);
    }
}

fn apply(ctx: &egui::Context, state: &mut State, fonts: egui::FontDefinitions, family: &str) {
    state.displayed_family = family.to_owned();
    state.queued_fonts = Some(fonts);
    ctx.request_repaint();
}

/// Submit only the final choice of this UI pass. egui compares against the
/// currently active fonts, not its pending update: submitting B then restoring
/// A in one pass can otherwise leave B pending even though A was requested last.
pub(crate) fn flush_font_updates(ctx: &egui::Context) {
    let Some(state) = state(ctx) else { return };
    let Ok(mut state) = state.lock() else { return };
    let Some(mut fonts) = state.queued_fonts.take() else {
        return;
    };
    sizing::apply(&mut fonts, state.sizes.get(&state.displayed_family));
    ctx.set_fonts(fonts);
    clear_metrics();
    state.metrics_changed_at = Some(ctx.cumulative_pass_nr());
    ctx.request_repaint();
}

pub(crate) fn set_scale(
    ctx: &egui::Context,
    db: &crate::infrastructure::app_state_db::AppStateDb,
    scale: f32,
) {
    let Some(state) = state(ctx) else { return };
    let Ok(mut state) = state.lock() else { return };
    if state.preview_pass.is_some() || state.desired.is_some() {
        return;
    }
    let selected = state.selected.clone();
    let scale = sizing::normalize(scale);
    state.save_failed = db
        .set_preference(&sizing::preference_key(&selected), &scale.to_string())
        .is_err();
    state.sizes.set(&selected, scale);
    let fonts = state.committed_fonts.clone();
    apply(ctx, &mut state, fonts, &selected);
}

fn clear_metrics() {
    crate::ui::views::list_view::clear_font_metrics_cache();
}

#[cfg(test)]
mod size_tests;
#[cfg(test)]
mod tests;
