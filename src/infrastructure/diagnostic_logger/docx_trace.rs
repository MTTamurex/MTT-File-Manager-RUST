//! Opening timings only. No paths, document text, errors or font names.
//! Disabled capture does not allocate buffers or read the clock. The origin is
//! viewer construction, not process launch. UI drawing is not GPU presentation.

use super::{capture_session, field_duration_ms, field_label, field_u64, writer, LOGGER};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT_OPEN_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    ReadFile,
    ParsePackage,
    SeedEngine,
    FontRequirements,
    LoadFonts,
    LayoutRequest,
    LayoutDocument,
    BuildDisplayList,
    DisplayInput,
    DisplayPrimitives,
    DisplayFinalize,
    EncodeFrame,
    CloneDisplayList,
    PageSizes,
    LoadImages,
    FinalizeOpen,
    SearchIndex,
    ScalePage,
    RasterizePage,
    DecodePagePixels,
    PrepareTexture,
}

impl Phase {
    fn label(self) -> &'static str {
        match self {
            Self::ReadFile => "read_file",
            Self::ParsePackage => "parse_package",
            Self::SeedEngine => "seed_engine",
            Self::FontRequirements => "font_requirements",
            Self::LoadFonts => "load_fonts",
            Self::LayoutRequest => "layout_request",
            Self::LayoutDocument => "layout_document",
            Self::BuildDisplayList => "build_display_list",
            Self::DisplayInput => "display_input",
            Self::DisplayPrimitives => "display_primitives",
            Self::DisplayFinalize => "display_finalize",
            Self::EncodeFrame => "encode_frame",
            Self::CloneDisplayList => "clone_display_list",
            Self::PageSizes => "page_sizes",
            Self::LoadImages => "load_images",
            Self::FinalizeOpen => "finalize_open",
            Self::SearchIndex => "search_index",
            Self::ScalePage => "scale_page",
            Self::RasterizePage => "rasterize_page",
            Self::DecodePagePixels => "decode_page_pixels",
            Self::PrepareTexture => "prepare_texture",
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Milestone {
    RendererReady,
    ReadyForPages,
    FirstPageRasterized,
    FirstPageTextureReady,
    FirstPageUiDrawn,
    OpenFailed,
    FirstPageFailed,
}

impl Milestone {
    fn label(self) -> &'static str {
        match self {
            Self::RendererReady => "renderer_ready",
            Self::ReadyForPages => "ready_for_pages",
            Self::FirstPageRasterized => "first_page_rasterized",
            Self::FirstPageTextureReady => "first_page_texture_ready",
            Self::FirstPageUiDrawn => "first_page_ui_drawn",
            Self::OpenFailed => "open_failed",
            Self::FirstPageFailed => "first_page_failed",
        }
    }
}

#[derive(Clone, Copy)]
struct Capture {
    session: u64,
    id: u64,
    started: Instant,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct OpenTrace(Option<Capture>);

impl OpenTrace {
    pub(crate) fn is_active(self) -> bool {
        self.0
            .is_some_and(|capture| capture_session() == Some(capture.session))
    }

    fn new() -> Self {
        Self(capture_session().map(|session| Capture {
            session,
            id: NEXT_OPEN_ID.fetch_add(1, Ordering::Relaxed),
            started: Instant::now(),
        }))
    }

    pub(crate) fn stages(self, phase: Phase) -> Stages {
        let capture = self
            .0
            .filter(|capture| capture_session() == Some(capture.session));
        Stages {
            capture,
            recording: capture.map(|_| Recording {
                phase,
                started: Instant::now(),
                samples: Vec::with_capacity(18),
            }),
        }
    }

    pub(crate) fn mark(self, milestone: Milestone) {
        if let Some(capture) = self.0 {
            emit(
                capture,
                "opening_milestone",
                milestone.label(),
                capture.started.elapsed(),
            );
        }
    }
}

struct Recording {
    phase: Phase,
    started: Instant,
    samples: Vec<(Phase, Duration)>,
}

impl Recording {
    fn lap_at(&mut self, next: Phase, now: Instant) {
        self.samples
            .push((self.phase, now.duration_since(self.started)));
        self.phase = next;
        self.started = now;
    }
}

pub(crate) struct Stages {
    capture: Option<Capture>,
    recording: Option<Recording>,
}

impl Stages {
    pub(crate) fn lap(&mut self, next: Phase) {
        if let Some(recording) = &mut self.recording {
            recording.lap_at(next, Instant::now());
        }
    }
}

impl Drop for Stages {
    fn drop(&mut self) {
        let (Some(capture), Some(recording)) = (self.capture, &mut self.recording) else {
            return;
        };
        // Buffer until the timed scope ends so logging is not timed as work.
        recording.lap_at(recording.phase, Instant::now());
        for &(phase, elapsed) in &recording.samples {
            emit(capture, "opening_stage", phase.label(), elapsed);
        }
    }
}

fn fields(capture: Capture, label: &'static str, elapsed: Duration) -> Vec<super::DiagnosticField> {
    vec![
        field_u64("process_id", u64::from(std::process::id())),
        field_u64("open_id", capture.id),
        field_label("phase", label),
        field_duration_ms("duration", elapsed),
    ]
}

fn emit(capture: Capture, code: &'static str, label: &'static str, elapsed: Duration) {
    if let Some(logger) = LOGGER.get() {
        emit_to(logger, capture, code, label, elapsed);
    }
}

fn emit_to(
    logger: &super::DiagnosticLogger,
    capture: Capture,
    code: &'static str,
    label: &'static str,
    elapsed: Duration,
) {
    if logger.capture_session() != Some(capture.session) {
        return;
    }
    // Pin the writer event to the original diagnostic session, even if
    // diagnostic mode is toggled between this check and enqueueing it.
    logger.writer.events(
        capture.session,
        vec![writer::Event::new(
            log::Level::Info,
            "docx_viewer",
            code,
            fields(capture, label, elapsed),
        )],
    );
}

/// UI-only state: texture preparation and the first visible page's paint
/// command are captured once, never on later scrolling or cache refills.
pub(crate) struct FirstPageTrace {
    trace: OpenTrace,
    texture_recorded: bool,
    drawn: bool,
}

impl FirstPageTrace {
    pub(crate) fn new() -> Self {
        Self {
            trace: OpenTrace::new(),
            texture_recorded: false,
            drawn: false,
        }
    }

    pub(crate) fn trace(&self) -> OpenTrace {
        self.trace
    }

    pub(crate) fn texture_stages(&self) -> Stages {
        let trace = if self.texture_recorded {
            OpenTrace::default()
        } else {
            self.trace
        };
        trace.stages(Phase::PrepareTexture)
    }

    pub(crate) fn texture_ready(&mut self) {
        if !self.texture_recorded {
            self.texture_recorded = true;
            self.trace.mark(Milestone::FirstPageTextureReady);
        }
    }

    pub(crate) fn page_drawn(&mut self) {
        if !self.drawn {
            self.drawn = true;
            self.trace.mark(Milestone::FirstPageUiDrawn);
        }
    }
}

#[cfg(test)]
mod tests;
