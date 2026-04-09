//! PDF page rendering and text extraction using Pdfium.

use std::path::{Path, PathBuf};

use once_cell::sync::OnceCell;
use pdfium_render::prelude::*;

static PDFIUM_READY: OnceCell<()> = OnceCell::new();

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PdfTextBounds {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

impl PdfTextBounds {
    pub fn from_points(left: f32, right: f32, top: f32, bottom: f32) -> Self {
        Self {
            left: left.min(right),
            right: left.max(right),
            top: top.max(bottom),
            bottom: top.min(bottom),
        }
    }

    pub fn width(&self) -> f32 {
        self.right - self.left
    }

    pub fn height(&self) -> f32 {
        self.top - self.bottom
    }

    pub fn overlaps(&self, other: &Self) -> bool {
        self.left < other.right
            && self.right > other.left
            && self.bottom < other.top
            && self.top > other.bottom
    }
}

#[derive(Clone, Debug)]
pub struct PdfTextSegment {
    pub bounds: PdfTextBounds,
}

/// A loaded PDF document ready for page rendering.
pub struct PdfRenderer {
    page_sizes: Vec<(f32, f32)>,
}

/// A rendered PDF page as raw RGBA pixels.
pub struct RenderedPage {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl PdfRenderer {
    /// Open a PDF file from disk.
    pub fn open(path: &Path) -> Result<Self, String> {
        let pdfium = pdfium()?;
        let document = pdfium
            .load_pdf_from_file(path, None)
            .map_err(|e| format!("LoadPdf: {e}"))?;

        let page_count = document.pages().len();
        let mut page_sizes = Vec::with_capacity(page_count as usize);

        for index in 0..page_count {
            let page = document
                .pages()
                .get(index as PdfPageIndex)
                .map_err(|e| e.to_string())?;
            page_sizes.push((page.width().value, page.height().value));
        }

        Ok(Self {
            page_sizes,
        })
    }

    pub fn page_sizes(&self) -> &[(f32, f32)] {
        &self.page_sizes
    }
}

/// Execute an operation with a timeout. Spawns a worker thread and waits
/// up to `timeout` for the result. Returns an error on timeout.
#[allow(dead_code)]
pub fn with_timeout<T: Send + 'static>(
    timeout: std::time::Duration,
    op: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("pdf-op-timeout".into())
        .spawn(move || {
            let _ = tx.send(op());
        })
        .map_err(|e| format!("Failed to spawn timeout thread: {e}"))?;

    rx.recv_timeout(timeout)
        .map_err(|_| format!("PDF operation timed out after {}s", timeout.as_secs()))?
}

pub(super) fn pdfium() -> Result<Pdfium, String> {
    if PDFIUM_READY.get().is_some() {
        return Ok(Pdfium::default());
    }

    bind_pdfium()
}

fn bind_pdfium() -> Result<Pdfium, String> {
    for candidate in pdfium_library_candidates() {
        match Pdfium::bind_to_library(&candidate) {
            Ok(bindings) => {
                let pdfium = Pdfium::new(bindings);
                let _ = PDFIUM_READY.set(());
                return Ok(pdfium);
            }
            Err(PdfiumError::LoadLibraryError(_)) => continue,
            Err(err) => {
                return Err(format!(
                    "Failed to load Pdfium from {}: {err}",
                    candidate.display()
                ));
            }
        }
    }

    match Pdfium::bind_to_system_library() {
        Ok(bindings) => {
            let pdfium = Pdfium::new(bindings);
            let _ = PDFIUM_READY.set(());
            Ok(pdfium)
        }
        Err(err) => Err(format!(
            "Failed to load pdfium.dll. Place a compatible Pdfium runtime next to the executable or install it system-wide. Details: {err}"
        )),
    }
}

fn pdfium_library_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(Pdfium::pdfium_platform_library_name_at_path(dir));
        }
    }

    // SEC: Do NOT search the current working directory ("./") for pdfium.dll.
    // A malicious DLL in CWD would be loaded and executed (DLL planting attack).
    candidates
}

pub(super) fn pdfium_rect_to_bounds(rect: PdfRect) -> PdfTextBounds {
    PdfTextBounds::from_points(
        rect.left().value,
        rect.right().value,
        rect.top().value,
        rect.bottom().value,
    )
}
