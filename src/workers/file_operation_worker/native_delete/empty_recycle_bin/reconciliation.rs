//! Reconcile the original selection against one fresh, read-only Shell enumeration.
use super::{identity, tracking::Tracking};
use crate::infrastructure::windows::recycle_bin::{self, RecycleBinShellItem};
use std::collections::HashSet;
use windows::core::Result;

pub(super) struct Snapshot {
    // Parsing names identify namespace entries, not their original display names.
    // Keep them only in memory; never log names or paths.
    names: Vec<Option<String>>,
}

impl Snapshot {
    pub(super) fn capture(items: &[RecycleBinShellItem]) -> Self {
        Self {
            names: items
                .iter()
                .map(|item| identity::parsing_name(&item.item))
                .collect(),
        }
    }

    pub(super) fn reconcile(&self, tracking: &Tracking) {
        self.reconcile_with(tracking, || {
            // Verification is read-only and must also run after cancellation to
            // count removals made before the operation stopped.
            recycle_bin::enumerate_recycle_bin_shell_items(|| false)
                .map(|items| Self::capture(&items))
        });
    }

    fn reconcile_with(&self, tracking: &Tracking, enumerate: impl FnOnce() -> Result<Self>) {
        let remaining = match enumerate() {
            Ok(remaining) => remaining,
            Err(error) => {
                // Failed/partial enumeration is not evidence that items vanished.
                log::warn!(
                    "[RecycleBin] Final verification unavailable: hresult=0x{:08X}",
                    error.code().0 as u32
                );
                return;
            }
        };
        let complete_identities = remaining.names.iter().all(Option::is_some);
        let remaining_names: HashSet<_> =
            remaining.names.iter().filter_map(Option::as_ref).collect();
        for (index, name) in self.names.iter().enumerate() {
            let present = name.as_ref().map(|name| remaining_names.contains(name));
            if present == Some(true) {
                tracking.reconcile_item(index, true);
            } else if remaining.names.is_empty() || (complete_identities && present == Some(false))
            {
                // New, unrelated items do not affect the selected roots. Unknown
                // remaining identities prevent inferring absence from a mismatch.
                tracking.reconcile_item(index, false);
            }
        }
    }
}

#[cfg(test)]
mod tests;
