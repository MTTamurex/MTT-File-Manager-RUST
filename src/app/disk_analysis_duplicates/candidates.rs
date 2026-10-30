use crate::app::disk_analysis_model::DiskAnalysisModel;
use std::collections::HashMap;

type SizeGroups = Vec<(u64, Vec<u32>)>;

fn is_recycle_root(model: &DiskAnalysisModel, idx: u32) -> bool {
    let node = &model.nodes[idx as usize];
    idx != model.root
        && node.parent == model.root
        && node.is_dir
        && node.name.eq_ignore_ascii_case("$RECYCLE.BIN")
}

pub(super) fn is_in_recycle_bin(model: &DiskAnalysisModel, mut idx: u32) -> bool {
    for _ in 0..model.nodes.len() {
        if is_recycle_root(model, idx) {
            return true;
        }
        if idx == model.root {
            break;
        }
        let parent = model.nodes[idx as usize].parent;
        if parent == idx {
            break;
        }
        idx = parent;
    }
    false
}

pub(super) fn is_recycle_path(path: &std::path::Path) -> bool {
    path.ancestors().any(|part| {
        part.file_name().is_some_and(|name| {
            name.as_encoded_bytes()
                .eq_ignore_ascii_case(b"$RECYCLE.BIN")
        }) && part
            .parent()
            .is_some_and(|parent| parent.has_root() && parent.file_name().is_none())
    })
}

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
    if cancelled() {
        return None;
    }
    if is_in_recycle_bin(model, root) {
        return Some((Vec::new(), 0));
    }
    let mut stack = vec![root];
    let mut candidates = Vec::new();
    let mut skipped = 0;
    while let Some(idx) = stack.pop() {
        if cancelled() {
            return None;
        }
        let node = &model.nodes[idx as usize];
        if node.is_dir {
            if !node.is_reparse && !is_recycle_root(model, idx) {
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
