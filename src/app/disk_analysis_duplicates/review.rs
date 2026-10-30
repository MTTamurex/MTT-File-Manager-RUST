//! Comparison state. Only two visible copies are loaded; previews never
//! survive report replacement.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{
    mpsc::{TryRecvError, TrySendError},
    Weak,
};

mod loader;
mod worker;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyStatus {
    Current,
    Changed,
    Missing,
    Unavailable,
    Inaccessible,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewStatus {
    Ready,
    NotImage,
    TooLarge,
    Unavailable,
}

pub struct PreviewImage {
    pub size: [usize; 2],
    pub rgba: Vec<u8>,
}

pub struct CopyInfo {
    pub status: CopyStatus,
    pub size: Option<u64>,
    pub created: Option<u64>,
    pub modified: Option<u64>,
    pub preview_status: PreviewStatus,
    pub image: Option<PreviewImage>,
}

impl CopyInfo {
    fn unavailable(status: CopyStatus) -> Self {
        Self {
            status,
            size: None,
            created: None,
            modified: None,
            preview_status: PreviewStatus::Unavailable,
            image: None,
        }
    }
}

pub struct ReviewEntry {
    pub idx: u32,
    requested: bool,
    pub info: Option<CopyInfo>,
    pub texture: Option<eframe::egui::TextureHandle>,
}

pub struct DuplicateReview {
    report: Weak<DuplicateReport>,
    pub active_group: Option<usize>,
    pub entries: [Option<ReviewEntry>; 2],
    generation: Arc<AtomicU64>,
    worker: Option<worker::Worker>,
    worker_failed: bool,
}

impl Default for DuplicateReview {
    fn default() -> Self {
        Self {
            report: Weak::new(),
            active_group: None,
            entries: [None, None],
            generation: Arc::new(AtomicU64::new(0)),
            worker: None,
            worker_failed: false,
        }
    }
}

impl DuplicateReview {
    pub(super) fn matches_report(&self, report: &Arc<DuplicateReport>) -> bool {
        self.report.ptr_eq(&Arc::downgrade(report))
    }

    pub fn bind_report(&mut self, report: &Arc<DuplicateReport>) {
        if !self.report.ptr_eq(&Arc::downgrade(report)) {
            self.reset();
            self.report = Arc::downgrade(report);
        }
    }

    pub fn reset(&mut self) {
        self.close();
        self.report = Weak::new();
    }

    pub fn close(&mut self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.active_group = None;
        self.entries = [None, None];
    }

    pub fn open_group(&mut self, report: &Arc<DuplicateReport>, group: usize) -> bool {
        self.bind_report(report);
        let Some(members) = report.groups.get(group).map(|g| &g.members) else {
            return false;
        };
        if members.len() < 2 {
            return false;
        }
        self.close();
        self.active_group = Some(group);
        self.entries = [
            Some(Self::entry(members[0].idx)),
            Some(Self::entry(members[1].idx)),
        ];
        true
    }

    fn entry(idx: u32) -> ReviewEntry {
        ReviewEntry {
            idx,
            requested: false,
            info: None,
            texture: None,
        }
    }

    pub fn select_copy(&mut self, slot: usize, idx: u32) -> bool {
        if slot > 1 {
            return false;
        }
        let Some(report) = self.report.upgrade() else {
            return false;
        };
        let Some(group) = self.active_group.and_then(|g| report.groups.get(g)) else {
            return false;
        };
        if !group.members.iter().any(|m| m.idx == idx) {
            return false;
        }
        let old = self.entries[slot].as_ref().map(|e| e.idx);
        if old == Some(idx) {
            return false;
        }
        if self.entries[1 - slot]
            .as_ref()
            .is_some_and(|e| e.idx == idx)
        {
            self.entries[1 - slot] = old.map(Self::entry);
        }
        self.entries[slot] = Some(Self::entry(idx));
        self.refresh();
        true
    }

    pub fn refresh(&mut self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        for entry in self.entries.iter_mut().flatten() {
            *entry = Self::entry(entry.idx);
        }
        if self.worker_failed {
            self.worker = None;
            self.worker_failed = false;
        }
    }

    pub fn current(&self, idx: u32) -> bool {
        self.entries.iter().flatten().any(|e| {
            e.idx == idx
                && e.info
                    .as_ref()
                    .is_some_and(|i| i.status == CopyStatus::Current)
        })
    }

    pub fn queue_visible(&mut self, model: &crate::app::disk_analysis_model::DiskAnalysisModel) {
        let Some(report) = self.report.upgrade() else {
            self.reset();
            return;
        };
        let Some(group) = self.active_group.and_then(|g| report.groups.get(g)) else {
            return;
        };
        if self.worker.is_none() && !self.worker_failed {
            match worker::Worker::spawn(self.generation.clone()) {
                Ok(worker) => self.worker = Some(worker),
                Err(_) => self.worker_failed = true,
            }
        }
        for (slot, entry) in self.entries.iter_mut().enumerate() {
            let Some(entry) = entry else {
                continue;
            };
            if entry.requested {
                continue;
            }
            if self.worker_failed {
                break;
            }
            let Some(member) = group.members.iter().find(|m| m.idx == entry.idx) else {
                continue;
            };
            let request = worker::Request {
                generation: self.generation.load(Ordering::Acquire),
                slot,
                member: member.clone(),
                path: PathBuf::from(model.path_of(entry.idx)),
            };
            match self.worker.as_ref().unwrap().requests.try_send(request) {
                Ok(()) => entry.requested = true,
                Err(TrySendError::Full(_)) => break,
                Err(TrySendError::Disconnected(_)) => {
                    self.worker_failed = true;
                }
            }
        }
        if self.worker_failed {
            self.fail_pending();
        }
    }

    fn fail_pending(&mut self) -> bool {
        let mut changed = false;
        for entry in self
            .entries
            .iter_mut()
            .flatten()
            .filter(|e| e.info.is_none())
        {
            entry.info = Some(CopyInfo::unavailable(CopyStatus::Inaccessible));
            entry.requested = true;
            changed = true;
        }
        changed
    }

    pub fn is_pending(&self) -> bool {
        self.active_group.is_some() && self.entries.iter().flatten().any(|e| e.info.is_none())
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        loop {
            let Some(worker) = &self.worker else {
                break;
            };
            match worker.responses.try_recv() {
                Ok(response) => {
                    if response.generation != self.generation.load(Ordering::Acquire) {
                        continue;
                    }
                    let idx = response.idx;
                    if let Some(entry) = self
                        .entries
                        .get_mut(response.slot)
                        .and_then(Option::as_mut)
                        .filter(|e| e.idx == idx)
                    {
                        entry.info = Some(response.info);
                        entry.texture = None;
                        changed = true;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.worker_failed = true;
                    changed |= self.fail_pending();
                    break;
                }
            }
        }
        changed
    }
}

impl Drop for DuplicateReview {
    fn drop(&mut self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests;
