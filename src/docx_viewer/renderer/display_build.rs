use docx_edit::EngineSession;

use crate::infrastructure::diagnostic_logger::docx_trace::{OpenTrace, Phase};

pub(super) fn build_frame(
    engine: &EngineSession,
    extras: &str,
    trace: OpenTrace,
) -> Result<(), String> {
    if !trace.is_active() {
        return engine.build_display_list_frame(extras, 0).map(|_| ());
    }

    // Child stages of the inclusive build_display_list timing. This is the
    // first/full frame of a fresh engine: exactly three observer callbacks.
    // No timers or callbacks are added to normal, non-diagnostic openings.
    let mut stages = trace.stages(Phase::DisplayInput);
    let mut next = [
        Phase::DisplayPrimitives,
        Phase::DisplayFinalize,
        Phase::EncodeFrame,
    ]
    .into_iter();
    engine
        .build_display_list_frame_observed(extras, 0, &mut || {
            if let Some(phase) = next.next() {
                stages.lap(phase);
            }
        })
        .map(|_| ())
}
