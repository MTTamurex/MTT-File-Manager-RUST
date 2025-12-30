//! Main application struct - orchestrates UI components and application state
//! Follows .cursorrules: separation of UI and business logic, < 300 lines

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

use eframe::egui;
use notify::RecommendedWatcher;

use crate::application::state::AppState;
use crate::domain::file_entry::FileEntry;
use crate::domain::thumbnail::ThumbnailData;
use crate::ui::cache::CacheManager;

// Worker management (to be extracted further)
pub struct WorkerManager {
    pub thumbnail_req_sender: Sender<(PathBuf, usize)>,
    pub image_receiver: Receiver<ThumbnailData>,
    pub file_entry_receiver: Receiver<(usize, Vec<FileEntry>)>,
    pub file_entry_sender: Sender<(usize, Vec<FileEntry>)>,
    pub cover_worker_sender: Sender<PathBuf>,
    pub cover_worker_receiver: Receiver<(PathBuf, Option<PathBuf>)>,
    pub fs_event_receiver: Receiver<notify::Result<notify::Event>>,
    pub fs_event_sender: Sender<notify::Result<notify::Event>>,
    pub watcher: Option<RecommendedWatcher>,
}

/// Main application struct
pub struct ImageViewerApp {
    pub state: AppState,
    pub cache: CacheManager,
    pub workers: WorkerManager,
    pub ui_ctx: egui::Context,
}

impl ImageViewerApp {
    /// Creates a new application instance
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = cc.egui_ctx.clone();
        
        // Initialize state
        let state = AppState::new("C:\\".to_string());
        
        // Initialize cache
        let cache = CacheManager::new();
        
        // Initialize workers (simplified for now - will be properly extracted)
        let workers = WorkerManager::new();
        
        Self {
            state,
            cache,
            workers,
            ui_ctx: ctx,
        }
    }
    
    /// Renders the application UI
    pub fn render_ui(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Simple UI for now
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("MTT File Manager - Refactoring in Progress");
            ui.separator();
            ui.label("Sprint 2: Refactoring large files into modules");
            ui.label(format!("Current path: {}", self.state.current_path));
            ui.label(format!("Items: {}", self.state.total_items));
            
            if self.state.is_loading_folder {
                ui.spinner();
                ui.label("Loading...");
            }
        });
    }
}

impl WorkerManager {
    /// Creates a new worker manager
    pub fn new() -> Self {
        use std::sync::mpsc;
        
        // Create channels (dummy implementations for now)
        let (file_entry_sender, file_entry_receiver) = mpsc::channel::<(usize, Vec<FileEntry>)>();
        let (cover_req_tx, _cover_req_rx) = mpsc::channel::<PathBuf>();
        let (_cover_res_tx, cover_res_rx) = mpsc::channel();
        let (_fs_tx, fs_rx) = mpsc::channel();
        let (_img_tx, img_rx) = mpsc::channel();
        let (req_tx, _req_rx) = mpsc::channel::<(PathBuf, usize)>();
        
        Self {
            thumbnail_req_sender: req_tx,
            image_receiver: img_rx,
            file_entry_receiver,
            file_entry_sender,
            cover_worker_sender: cover_req_tx,
            cover_worker_receiver: cover_res_rx,
            fs_event_receiver: fs_rx,
            fs_event_sender: _fs_tx,
            watcher: None,
        }
    }
}

impl eframe::App for ImageViewerApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.render_ui(ctx, frame);
    }
}
