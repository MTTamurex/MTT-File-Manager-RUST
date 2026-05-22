use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct PendingDragMoveConfirmation {
    pub paths: Vec<PathBuf>,
    pub dest_folder: PathBuf,
    pub source_folder: Option<PathBuf>,
}

impl PendingDragMoveConfirmation {
    pub fn new(paths: Vec<PathBuf>, dest_folder: PathBuf, source_folder: Option<PathBuf>) -> Self {
        Self {
            paths,
            dest_folder,
            source_folder,
        }
    }
}
