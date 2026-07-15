use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::viewer_app::{DocumentStatus, PdfPageLayout, PdfViewerApp, ZoomMode};

#[derive(Default, Serialize, Deserialize)]
struct PdfViewerStateFile {
    documents: HashMap<String, PdfDocumentState>,
}

#[derive(Clone, Serialize, Deserialize)]
struct PdfDocumentState {
    page: u32,
    zoom: f32,
    zoom_mode: String,
    view_mode: String,
    rotation: u16,
}

impl PdfViewerApp {
    pub(super) fn restore_document_state(&mut self) {
        if self.total_pages == 0 {
            return;
        }

        let Some(state) = load_state_for(&self.worker_path) else {
            return;
        };

        self.current_page = state.page.min(self.total_pages.saturating_sub(1));
        self.page_input = format!("{}", self.current_page + 1);
        self.scroll_to_page = Some(self.current_page);
        self.zoom = state.zoom.clamp(0.1, 5.0);
        self.zoom_mode = zoom_mode_from_str(&state.zoom_mode);
        self.page_layout = page_layout_from_str(&state.view_mode);
        self.rotation = state.rotation % 360;
        if !self.rotation.is_multiple_of(90) {
            self.rotation = 0;
        }
        self.on_view_changed();
    }

    pub(super) fn save_document_state(&self) {
        if !matches!(
            self.document_status,
            DocumentStatus::LoadingMetadata | DocumentStatus::Ready
        ) || self.total_pages == 0
        {
            return;
        }

        let Some(path) = state_file_path() else {
            return;
        };

        let mut state_file = read_state_file(&path).unwrap_or_default();
        state_file.documents.insert(
            document_key(&self.worker_path),
            PdfDocumentState {
                page: self.current_page.min(self.total_pages.saturating_sub(1)),
                zoom: self.zoom,
                zoom_mode: zoom_mode_to_str(self.zoom_mode).to_string(),
                view_mode: page_layout_to_str(self.page_layout).to_string(),
                rotation: self.rotation,
            },
        );

        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&state_file) {
            let _ = std::fs::write(path, json);
        }
    }
}

fn load_state_for(document_path: &Path) -> Option<PdfDocumentState> {
    let path = state_file_path()?;
    let state_file = read_state_file(&path)?;
    state_file
        .documents
        .get(&document_key(document_path))
        .cloned()
}

fn read_state_file(path: &Path) -> Option<PdfViewerStateFile> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn state_file_path() -> Option<PathBuf> {
    Some(
        dirs::data_local_dir()?
            .join("MTT-File-Manager")
            .join("state")
            .join("pdf_viewer_state.json"),
    )
}

fn document_key(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}

fn zoom_mode_to_str(mode: ZoomMode) -> &'static str {
    match mode {
        ZoomMode::FitWidth => "fit_width",
        ZoomMode::FitPage => "fit_page",
        ZoomMode::Custom => "custom",
    }
}

fn zoom_mode_from_str(value: &str) -> ZoomMode {
    match value {
        "fit_page" => ZoomMode::FitPage,
        "custom" => ZoomMode::Custom,
        _ => ZoomMode::FitWidth,
    }
}

fn page_layout_to_str(layout: PdfPageLayout) -> &'static str {
    match layout {
        PdfPageLayout::OnePage => "one_page",
        PdfPageLayout::TwoPage => "two_page",
    }
}

fn page_layout_from_str(value: &str) -> PdfPageLayout {
    match value {
        "two_page" => PdfPageLayout::TwoPage,
        _ => PdfPageLayout::OnePage,
    }
}
