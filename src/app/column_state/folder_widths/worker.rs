//! SQLite work never runs on the caller or holds any UI/cache locks.
use super::{Change, WidthLocation};
use crate::domain::list_columns::ColumnId;
use crate::infrastructure::app_state_db::AppStateDb;
use crossbeam_channel::{bounded, unbounded, Receiver, Sender};
use eframe::egui;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub(super) enum Request {
    Load(WidthLocation, egui::Context),
    Save(Vec<Change>, egui::Context),
}
pub(super) enum Response {
    Loaded(WidthLocation, Result<Vec<(ColumnId, f32)>, ()>),
    Saved(bool),
}
pub(super) struct Worker {
    pub requests: Sender<Request>,
    pub results: Receiver<Response>,
    pub running: Arc<AtomicBool>,
}
struct Lifetime(Arc<AtomicBool>);
impl Drop for Lifetime {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl Worker {
    pub fn start(db: Arc<AppStateDb>) -> std::io::Result<Self> {
        let (sender, requests) = bounded(32);
        let (results, receiver) = unbounded();
        let running = Arc::new(AtomicBool::new(true));
        let lifetime = Lifetime(running.clone());
        crate::spawn_named("folder-column-widths", move || {
            let _lifetime = lifetime;
            while let Ok(request) = requests.recv() {
                let (result, ctx) = match request {
                    Request::Load(location, ctx) => {
                        let result = db.load_column_widths(&location).map_err(|_| ());
                        (Response::Loaded(location, result), ctx)
                    }
                    Request::Save(changes, ctx) => {
                        let values: Vec<_> = changes
                            .into_iter()
                            .map(|change| (change.location, change.column, change.width))
                            .collect();
                        (Response::Saved(db.save_column_widths(&values).is_ok()), ctx)
                    }
                };
                if results.send(result).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        })?;
        Ok(Self {
            requests: sender,
            results: receiver,
            running,
        })
    }
}
