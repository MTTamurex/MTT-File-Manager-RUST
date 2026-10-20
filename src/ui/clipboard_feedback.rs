//! Cached clipboard feedback shared by all file views in a viewport.
use crate::application::{ClipboardManager, ClipboardOp};
use crate::infrastructure::windows_clipboard::{self, snapshot::FileSnapshot, ClipboardFileOp};
use eframe::egui::{self, Color32};
use rustc_hash::FxHashSet;
use std::path::Path;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(300);

#[derive(Default)]
struct Feedback {
    paths: FxHashSet<String>,
    operation: Option<ClipboardOp>,
}

#[derive(Default)]
struct Monitor {
    sequence: Option<u32>,
    internal_operation: Option<(ClipboardOp, u64)>,
    loaded: bool,
    checked: Option<Instant>,
    pending: Option<mpsc::Receiver<Option<FileSnapshot>>>,
}

impl Monitor {
    fn observe(
        &mut self,
        ctx: &egui::Context,
        sequence: Option<u32>,
        internal_operation: Option<(ClipboardOp, u64)>,
    ) {
        if sequence != self.sequence || internal_operation != self.internal_operation {
            self.sequence = sequence;
            self.internal_operation = internal_operation;
            self.loaded = false;
            self.checked = None;
            // Hide stale markers immediately, without waiting for the worker.
            publish(ctx, &[], None);
        }
    }

    fn apply_external(&mut self, ctx: &egui::Context, snapshot: FileSnapshot) {
        if Some(snapshot.sequence) == self.sequence && self.internal_operation.is_none() {
            let operation = match snapshot.operation {
                ClipboardFileOp::Copy => ClipboardOp::Copy,
                ClipboardFileOp::Move => ClipboardOp::Move,
            };
            publish(ctx, &snapshot.paths, Some(operation));
            self.loaded = true;
        }
    }
}

fn path_key(path: &Path) -> String {
    let text = path.to_string_lossy().replace('/', "\\").to_lowercase();
    let text = if let Some(unc) = text.strip_prefix(r"\\?\unc\") {
        format!(r"\\{unc}")
    } else {
        text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
    };
    text.trim_end_matches('\\').to_owned()
}

fn publish(ctx: &egui::Context, paths: &[std::path::PathBuf], operation: Option<ClipboardOp>) {
    let feedback = Feedback {
        paths: paths.iter().map(|p| path_key(p)).collect(),
        operation,
    };
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("clipboard_feedback"), Arc::new(feedback)));
    ctx.request_repaint();
}

pub(crate) fn update(ctx: &egui::Context, clipboard: &ClipboardManager) {
    let sequence = windows_clipboard::clipboard_sequence_number();
    update_for_sequence(ctx, sequence, clipboard.visual_state(sequence));
}

fn update_for_sequence(
    ctx: &egui::Context,
    sequence: Option<u32>,
    internal: Option<(&[std::path::PathBuf], ClipboardOp, u64)>,
) {
    let shared = ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<Arc<Mutex<Monitor>>>(egui::Id::new("clipboard_monitor"))
            .clone()
    });
    let Ok(mut monitor) = shared.lock() else {
        return;
    };
    let internal_operation = internal.map(|(_, op, revision)| (op, revision));
    monitor.observe(ctx, sequence, internal_operation);
    if let Some((paths, op, _)) = internal {
        if !monitor.loaded {
            publish(ctx, paths, Some(op));
            monitor.loaded = true;
        }
    }
    if let Some(receiver) = &monitor.pending {
        match receiver.try_recv() {
            Ok(snapshot) => {
                if let Some(snapshot) = snapshot {
                    monitor.apply_external(ctx, snapshot);
                }
                monitor.pending = None;
            }
            Err(mpsc::TryRecvError::Disconnected) => monitor.pending = None,
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }
    let due = monitor
        .checked
        .is_none_or(|last| last.elapsed() >= POLL_INTERVAL);
    if !monitor.loaded && monitor.pending.is_none() && due {
        monitor.checked = Some(Instant::now());
        if let Some(sequence) = sequence {
            let (sender, receiver) = mpsc::channel();
            let wakeup = ctx.clone();
            if std::thread::Builder::new()
                .name("clipboard-feedback".into())
                .spawn(move || {
                    let _ = sender.send(windows_clipboard::snapshot::read(sequence));
                    wakeup.request_repaint();
                })
                .is_ok()
            {
                monitor.pending = Some(receiver);
            }
        }
    }
    // Also observe changes while the pointer and keyboard are idle. No payload
    // is read again until the sequence changes (except bounded busy retries).
    ctx.request_repaint_after(POLL_INTERVAL);
}

pub(crate) fn operation(ctx: &egui::Context, path: &Path) -> Option<ClipboardOp> {
    let feedback =
        ctx.data(|data| data.get_temp::<Arc<Feedback>>(egui::Id::new("clipboard_feedback")))?;
    if feedback.paths.is_empty() {
        return None;
    }
    // Path normalization allocates; keep it outside egui's shared memory lock.
    feedback
        .paths
        .contains(&path_key(path))
        .then_some(feedback.operation)
        .flatten()
}

pub(crate) fn icon_opacity(ctx: &egui::Context, path: &Path) -> f32 {
    if operation(ctx, path) == Some(ClipboardOp::Move) {
        0.5
    } else {
        1.0
    }
}

pub(crate) fn paint(ui: &egui::Ui, rect: egui::Rect, path: &Path) {
    let Some(op) = operation(ui.ctx(), path) else {
        return;
    };
    let dark = ui.visuals().dark_mode;
    let color = match (op, dark) {
        (ClipboardOp::Copy, false) => Color32::from_rgb(25, 125, 65),
        (ClipboardOp::Copy, true) => Color32::from_rgb(95, 205, 135),
        (ClipboardOp::Move, false) => Color32::from_rgb(170, 105, 0),
        (ClipboardOp::Move, true) => Color32::from_rgb(235, 180, 65),
    };
    // Inset from selection/focus and drop-target borders so they remain visible.
    ui.painter().rect_stroke(
        rect.shrink(3.0),
        3.0,
        egui::Stroke::new(1.0, color),
        egui::StrokeKind::Inside,
    );
}

#[cfg(test)]
mod tests;
