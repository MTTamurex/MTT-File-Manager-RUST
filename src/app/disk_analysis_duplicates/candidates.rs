use crate::app::disk_analysis_model::DiskAnalysisModel;
use std::collections::HashMap;

type SizeGroups = Vec<(u64, Vec<u32>)>;

pub fn group_by_size(candidates: Vec<(u32, u64)>) -> SizeGroups {
    group_cancellable(candidates, || false).expect("non-cancellable grouping")
}

fn group_cancellable(
    candidates: Vec<(u32, u64)>,
    cancelled: impl Fn() -> bool,
) -> Option<SizeGroups> {
    let mut sizes: HashMap<u64, Vec<u32>> = HashMap::new();
    for (idx, size) in candidates {
        if cancelled() {
            return None;
        }
        sizes.entry(size).or_default().push(idx);
    }
    let mut groups: Vec<_> = sizes
        .into_iter()
        .filter(|(_, members)| members.len() > 1)
        .collect();
    for (_, members) in &mut groups {
        members.sort_unstable();
    }
    groups.sort_by_key(|g| g.0);
    Some(groups)
}

pub(super) fn collect(
    model: &DiskAnalysisModel,
    root: u32,
    minimum: u64,
    cancelled: impl Fn() -> bool,
) -> Option<(SizeGroups, u64)> {
    let mut stack = vec![root];
    let mut candidates = Vec::new();
    let mut skipped = 0;
    while let Some(idx) = stack.pop() {
        if cancelled() {
            return None;
        }
        let node = &model.nodes[idx as usize];
        if node.is_dir {
            if !node.is_reparse {
                stack.extend_from_slice(model.children(idx));
            }
        } else if !node.is_reparse {
            if node.size > 0 && node.size >= minimum {
                candidates.push((idx, node.size));
            } else {
                skipped += 1;
            }
        }
    }
    Some((group_cancellable(candidates, cancelled)?, skipped))
}
