use super::reader::ValidatedHash;
use super::*;
use crate::app::disk_analysis_model::DiskAnalysisModel;
use std::collections::{HashMap, HashSet};

pub(super) fn remove_member(report: &mut DuplicateReport, idx: u32) {
    for group in &mut report.groups {
        group.members.retain(|m| m.idx != idx);
        let entries: Vec<_> = group
            .members
            .iter()
            .map(|m| (m.physical_identity(), m.allocated_size))
            .collect();
        (group.recoverable, group.has_hardlinks) = compute_recoverable(&entries);
    }
    report.groups.retain(|g| {
        g.members.len() > 1
            && g.members
                .iter()
                .any(|m| m.physical_identity() != g.members[0].physical_identity())
    });
    report.total_recoverable = report
        .groups
        .iter()
        .fold(0u64, |sum, g| sum.saturating_add(g.recoverable));
}

pub fn compute_recoverable(entries: &[(FileIdentity, u64)]) -> (u64, bool) {
    let mut allocations: HashMap<FileIdentity, u64> = HashMap::new();
    for &(identity, bytes) in entries {
        let value = allocations.entry(identity).or_default();
        *value = (*value).max(bytes);
    }
    let linked = allocations.len() < entries.len();
    if allocations.len() < 2 {
        return (0, linked);
    }
    let keep = *allocations
        .iter()
        .max_by_key(|(_, bytes)| **bytes)
        .unwrap()
        .0;
    // Sum removed copies directly; subtracting after a saturated total loses bytes.
    let recoverable = allocations
        .iter()
        .filter(|(id, _)| **id != keep)
        .fold(0u64, |sum, (_, bytes)| sum.saturating_add(*bytes));
    (recoverable, linked)
}

pub(super) fn build(
    model: &DiskAnalysisModel,
    groups: &[(u64, Vec<u32>)],
    hashes: &HashMap<u32, ValidatedHash>,
    progress: &DuplicateProgress,
    cancelled: impl Fn() -> bool,
) -> Option<DuplicateReport> {
    let mut output = Vec::new();
    let mut hardlinks = 0;
    for (size, members) in groups {
        if cancelled() {
            return None;
        }
        let mut keyed: HashMap<[u8; 32], Vec<u32>> = HashMap::new();
        for &idx in members {
            if cancelled() {
                return None;
            }
            if let Some(hash) = hashes.get(&idx) {
                keyed.entry(hash.digest).or_default().push(idx);
            }
        }
        for mut members in keyed.into_values().filter(|m| m.len() > 1) {
            members.sort_unstable();
            let entries: Vec<_> = members
                .iter()
                .map(|idx| {
                    (
                        hashes[idx].stamp.identity,
                        model.nodes[*idx as usize].allocated_size,
                    )
                })
                .collect();
            let distinct: HashSet<_> = entries.iter().map(|e| e.0).collect();
            if distinct.len() < 2 {
                hardlinks += 1;
                continue;
            }
            let (recoverable, has_hardlinks) = compute_recoverable(&entries);
            output.push(DuplicateGroup {
                logical_size: *size,
                recoverable,
                has_hardlinks,
                members: members
                    .into_iter()
                    .map(|idx| {
                        let node = &model.nodes[idx as usize];
                        DuplicateMember {
                            idx,
                            logical_size: node.size,
                            allocated_size: node.allocated_size,
                            stamp: hashes[&idx].stamp,
                        }
                    })
                    .collect(),
            });
        }
    }
    output.sort_by(|a, b| {
        b.recoverable
            .cmp(&a.recoverable)
            .then(b.logical_size.cmp(&a.logical_size))
            .then(a.members[0].idx.cmp(&b.members[0].idx))
    });
    Some(DuplicateReport {
        total_recoverable: output
            .iter()
            .fold(0u64, |sum, group| sum.saturating_add(group.recoverable)),
        groups: output,
        stats: DuplicateStats {
            total_candidates: progress.total_candidates,
            processed_files: progress.processed_files,
            hashed_bytes: progress.hashed_bytes,
            partial_rejected: progress.partial_rejected,
            full_candidates: progress.full_candidates,
            inaccessible: progress.inaccessible,
            changed_during_read: progress.changed_during_read,
            skipped_unavailable: progress.skipped_unavailable,
            skipped_by_limit: progress.skipped_by_limit,
            hardlink_only_groups: hardlinks,
        },
    })
}
