//! Shared labels for deletion cards and window titles, including mixed results.
use crate::infrastructure::delete_engine::DeleteProgress;
use rust_i18n::t;
use std::sync::atomic::Ordering;

#[derive(Default)]
pub(super) struct Counts {
    pub recycled: u64,
    pub permanent: u64,
    pub has_recycle: bool,
    pub has_permanent: bool,
}

impl Counts {
    pub fn include(&mut self, progress: &DeleteProgress) {
        let recycled = progress.recycled.load(Ordering::Relaxed);
        let permanent = progress.permanently_removed.load(Ordering::Relaxed);
        self.recycled = self.recycled.saturating_add(recycled);
        self.permanent = self.permanent.saturating_add(permanent);
        self.has_recycle |= !progress.permanently || recycled > 0;
        self.has_permanent |= progress.permanently
            || progress.used_permanent_fallback.load(Ordering::Relaxed)
            || permanent > 0;
    }

    pub fn label(&self) -> String {
        if self.has_recycle && self.has_permanent {
            t!(
                "delete_engine.header_mixed",
                recycled = self.recycled,
                permanent = self.permanent
            )
            .to_string()
        } else if self.has_permanent {
            t!("delete_engine.header_permanent", count = self.permanent).to_string()
        } else {
            t!("delete_engine.header_recycled", count = self.recycled).to_string()
        }
    }
}
