//! Navigation-scoped interest in column reads, independent for the two physical panes.
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Default)]
pub(super) struct DisplaySources {
    panes: [Option<Source>; 2],
    pub current: Arc<AtomicBool>,
}

struct Source {
    generation: usize,
    path: PathBuf,
    cancelled: Arc<AtomicBool>,
}

impl DisplaySources {
    pub fn set(&mut self, pane: usize, source: Option<(usize, &Path)>) {
        if let (Some(previous), Some((generation, path))) = (&self.panes[pane], source) {
            if previous.generation == generation && previous.path == path {
                self.current = previous.cancelled.clone();
                return;
            }
        }
        if let Some(previous) = self.panes[pane].take() {
            previous.cancelled.store(true, Ordering::Release);
        }
        self.current = Arc::new(AtomicBool::new(source.is_none()));
        if let Some((generation, path)) = source {
            self.panes[pane] = Some(Source {
                generation,
                path: path.to_owned(),
                cancelled: self.current.clone(),
            });
        }
    }
}
