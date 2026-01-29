use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::infrastructure::usn_journal::{get_file_reference_number, UsnJournal, UsnRecord};

#[derive(Debug, Clone)]
pub enum FsEvent {
    Created(PathBuf),
    Deleted(PathBuf),
    Modified(PathBuf),
    Renamed(PathBuf, PathBuf),
}

pub struct UsnWatcherState {
    monitored_dirs: Mutex<HashSet<(PathBuf, u64)>>,
    last_usn: Mutex<std::collections::HashMap<char, i64>>,
}

impl UsnWatcherState {
    pub fn new() -> Self {
        Self {
            monitored_dirs: Mutex::new(HashSet::new()),
            last_usn: Mutex::new(std::collections::HashMap::new()),
        }
    }

    pub fn watch(&self, path: PathBuf) {
        if let Some(file_ref) = get_file_reference_number(&path) {
            if let Ok(mut dirs) = self.monitored_dirs.lock() {
                dirs.insert((path, file_ref));
            }
        }
    }

    pub fn unwatch(&self, path: &PathBuf) {
        if let Ok(mut dirs) = self.monitored_dirs.lock() {
            dirs.retain(|(p, _)| p != path);
        }
    }

    pub fn set_single_watch(&self, path: PathBuf) {
        if let Ok(mut dirs) = self.monitored_dirs.lock() {
            dirs.clear();
            if let Some(file_ref) = get_file_reference_number(&path) {
                dirs.insert((path, file_ref));
            }
        }
    }

    fn get_monitored_refs(&self, drive: char) -> HashSet<u64> {
        if let Ok(dirs) = self.monitored_dirs.lock() {
            dirs.iter()
                .filter(|(p, _)| {
                    p.to_string_lossy()
                        .chars()
                        .next()
                        .map(|c| c.to_ascii_uppercase() == drive)
                        .unwrap_or(false)
                })
                .map(|(_, ref_num)| *ref_num)
                .collect()
        } else {
            HashSet::new()
        }
    }

    fn get_path_for_ref(&self, file_ref: u64) -> Option<PathBuf> {
        if let Ok(dirs) = self.monitored_dirs.lock() {
            dirs.iter()
                .find(|(_, r)| *r == file_ref)
                .map(|(p, _)| p.clone())
        } else {
            None
        }
    }
}

pub fn spawn_usn_watcher(
    state: Arc<UsnWatcherState>,
    event_sender: Sender<FsEvent>,
    poll_interval: Duration,
) {
    std::thread::spawn(move || {
        let mut journals: std::collections::HashMap<char, UsnJournal> =
            std::collections::HashMap::new();

        loop {
            let drives: HashSet<char> = if let Ok(dirs) = state.monitored_dirs.lock() {
                dirs.iter()
                    .filter_map(|(p, _)| {
                        p.to_string_lossy()
                            .chars()
                            .next()
                            .map(|c| c.to_ascii_uppercase())
                    })
                    .collect()
            } else {
                HashSet::new()
            };

            for drive in drives {
                if !journals.contains_key(&drive) {
                    if let Ok(journal) = UsnJournal::open(drive) {
                        if let Ok(mut last_usn) = state.last_usn.lock() {
                            last_usn.entry(drive).or_insert(journal.current_usn());
                        }
                        journals.insert(drive, journal);
                    }
                }

                if let Some(journal) = journals.get(&drive) {
                    let start_usn = state
                        .last_usn
                        .lock()
                        .ok()
                        .and_then(|m| m.get(&drive).copied())
                        .unwrap_or(journal.current_usn());

                    if let Ok((records, new_usn)) = journal.read_changes(start_usn) {
                        if let Ok(mut last_usn) = state.last_usn.lock() {
                            last_usn.insert(drive, new_usn);
                        }

                        let monitored_refs = state.get_monitored_refs(drive);
                        let relevant: Vec<UsnRecord> = records
                            .into_iter()
                            .filter(|r| monitored_refs.contains(&r.parent_reference_number))
                            .collect();

                        for record in relevant {
                            if let Some(parent_path) =
                                state.get_path_for_ref(record.parent_reference_number)
                            {
                                let file_path = parent_path.join(&record.file_name);

                                if record.reason.is_close() {
                                    if record.reason.is_create() {
                                        let _ = event_sender.send(FsEvent::Created(file_path));
                                    } else if record.reason.is_delete() {
                                        let _ = event_sender.send(FsEvent::Deleted(file_path));
                                    } else if record.reason.is_modify() {
                                        let _ = event_sender.send(FsEvent::Modified(file_path));
                                    }
                                }
                            }
                        }
                    }
                }
            }

            std::thread::sleep(poll_interval);
        }
    });
}
