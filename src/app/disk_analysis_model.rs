//! Immutable tree model for the disk usage analyzer.
//!
//! Built once (off the UI thread) from a [`DiskAnalysisSnapshot`] and shared
//! with the view as `Arc<DiskAnalysisModel>`. All tree invariants are
//! established here: orphan attachment, cycle exclusion, reparse non-descent
//! and bottom-up aggregation.
//!
//! Memory layout is compact by design (the model can hold millions of
//! nodes): each [`AnalysisNode`] keeps only steady-state data and child
//! links live in one flat CSR array ([`DiskAnalysisModel::child_links`])
//! instead of a heap `Vec` per node. Counts, depths and per-category
//! rollups exist only as build-time scratch and are dropped before the
//! model is returned.

use mtt_search_protocol::DiskAnalysisSnapshot;
use std::time::{Duration, Instant};

/// Coarse file-type categories used for coloring and the sidebar legend.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum FileCategory {
    Video,
    Images,
    Audio,
    Archives,
    Code,
    Documents,
    System,
    Other,
}

impl FileCategory {
    pub const ALL: [FileCategory; 8] = [
        FileCategory::Video,
        FileCategory::Images,
        FileCategory::Audio,
        FileCategory::Archives,
        FileCategory::Code,
        FileCategory::Documents,
        FileCategory::System,
        FileCategory::Other,
    ];

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|c| *c == self).unwrap_or(7)
    }

    /// Classify a file by its extension. Directories are classified by their
    /// dominant content category instead (see `AnalysisNode.category`).
    pub fn from_file_name(name: &str) -> FileCategory {
        let ext = name.rsplit('.').next().unwrap_or("");
        if ext.len() == name.len() {
            return FileCategory::Other;
        }
        let ext = ext.to_ascii_lowercase();
        match ext.as_str() {
            "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" | "webm" | "m4v" | "mpg" | "mpeg"
            | "ts" | "vob" => FileCategory::Video,
            "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "svg" | "ico" | "heic" | "heif"
            | "tif" | "tiff" | "raw" | "cr2" | "nef" | "arw" | "avif" => FileCategory::Images,
            "mp3" | "flac" | "wav" | "aac" | "ogg" | "m4a" | "wma" | "opus" => FileCategory::Audio,
            "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "zst" | "iso" | "cab" => {
                FileCategory::Archives
            }
            "rs" | "js" | "jsx" | "tsx" | "py" | "c" | "cpp" | "h" | "hpp" | "cs" | "java"
            | "go" | "rb" | "php" | "html" | "css" | "json" | "toml" | "yml" | "yaml" | "xml"
            | "sql" | "sh" | "ps1" | "lua" => FileCategory::Code,
            "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" | "md" | "rtf"
            | "odt" | "csv" | "epub" => FileCategory::Documents,
            "sys" | "dll" | "exe" | "msi" | "drv" | "efi" | "mui" | "inf" | "cat" | "msu" => {
                FileCategory::System
            }
            _ => FileCategory::Other,
        }
    }
}

/// One node of the analyzed volume tree.
///
/// Compact steady-state layout: child links live in the flat CSR array
/// [`DiskAnalysisModel::child_links`] (see `child_start`/`child_len`);
/// counts, depths and per-category byte totals are build-only scratch.
pub struct AnalysisNode {
    pub name: String,
    /// Index of the parent node; the root is its own parent.
    pub parent: u32,
    /// Start of this node's child slice in `DiskAnalysisModel::child_links`.
    pub child_start: u32,
    /// Number of children (0 for files and leaves).
    pub child_len: u32,
    /// Own size in bytes (0 for directories).
    pub size: u64,
    /// Physical allocation of all streams owned by this file.
    pub allocated_size: u64,
    /// Aggregated logical size of the subtree. Reparse directories do not descend.
    pub subtree_size: u64,
    /// Aggregated physical allocation of the subtree.
    pub subtree_allocated_size: u64,
    /// Number of files in this subtree; a file counts itself once.
    pub subtree_files: u64,
    pub is_dir: bool,
    pub is_reparse: bool,
    /// Files: own category. Directories: dominant category by subtree bytes.
    pub category: FileCategory,
}

/// Per-category rollups over all files in the volume, kept per size basis
/// so every metric (allocated / logical / file count) can chart without a
/// rebuild. Indexed by `FileCategory::index()`.
#[derive(Clone, Copy, Debug, Default)]
pub struct CategoryTotals {
    pub logical: [u64; 8],
    pub allocated: [u64; 8],
    pub files: [u64; 8],
}

impl CategoryTotals {
    /// Totals for the given metric (charts and legends read through this).
    pub fn slice_for(&self, metric: super::disk_analysis_query::SizeMetric) -> &[u64; 8] {
        match metric {
            super::disk_analysis_query::SizeMetric::Allocated => &self.allocated,
            super::disk_analysis_query::SizeMetric::Logical => &self.logical,
            super::disk_analysis_query::SizeMetric::FileCount => &self.files,
        }
    }
}

/// Fully aggregated, read-only analysis of one volume.
pub struct DiskAnalysisModel {
    pub drive_letter: char,
    pub nodes: Vec<AnalysisNode>,
    /// Flat CSR child-link array; a node's children are
    /// `child_links[child_start..child_start + child_len]`.
    pub child_links: Vec<u32>,
    /// Index of the synthetic volume root ("C:").
    pub root: u32,
    pub total_size: u64,
    pub total_allocated_size: u64,
    pub total_files: u64,
    pub total_folders: u64,
    /// Maximum tree depth reached from the root.
    pub deepest_path: u32,
    /// Per-category totals over all files.
    pub category_totals: CategoryTotals,
    pub build_elapsed: Duration,
}

impl DiskAnalysisModel {
    /// Children of node `idx` in record order (empty slice for leaves).
    pub fn children(&self, idx: u32) -> &[u32] {
        let node = &self.nodes[idx as usize];
        let start = node.child_start as usize;
        &self.child_links[start..start + node.child_len as usize]
    }

    /// Build the tree model from a raw snapshot. O(n) passes; cycle-safe.
    pub fn build(snapshot: DiskAnalysisSnapshot) -> Self {
        Self::build_cancellable(snapshot, || false).expect("non-cancellable build")
    }

    pub fn build_cancellable<F>(snapshot: DiskAnalysisSnapshot, is_cancelled: F) -> Option<Self>
    where
        F: Fn() -> bool,
    {
        let started = Instant::now();
        let drive_letter = snapshot.drive_letter;
        let records = snapshot.records;
        let record_count = records.len();

        let mut nodes: Vec<AnalysisNode> = Vec::with_capacity(record_count + 1);
        // Synthetic root for the whole volume.
        nodes.push(AnalysisNode {
            name: format!("{}:", drive_letter.to_ascii_uppercase()),
            parent: 0,
            child_start: 0,
            child_len: 0,
            size: 0,
            allocated_size: 0,
            subtree_size: 0,
            subtree_allocated_size: 0,
            subtree_files: 0,
            is_dir: true,
            is_reparse: false,
            category: FileCategory::Other,
        });

        // Pass 1: create one node per record, moving names out of the
        // snapshot (no cloning). Only the parent FRNs and the FRN lookup
        // pairs survive this pass, so the record buffer and its string
        // heap are released before child linking allocates.
        let mut parent_frns: Vec<u64> = Vec::with_capacity(record_count);
        let mut self_parents: Vec<bool> = Vec::with_capacity(record_count);
        let mut frn_pairs: Vec<(u64, u32)> = Vec::with_capacity(record_count);
        let mut volume_root_idx: Option<u32> = None;
        for (record_pos, record) in records.into_iter().enumerate() {
            if record_pos % 4096 == 0 && is_cancelled() {
                return None;
            }
            let idx = nodes.len() as u32;
            let self_parent = record.parent_frn == record.frn;
            if volume_root_idx.is_none() && self_parent && record.is_dir {
                volume_root_idx = Some(idx);
            }
            frn_pairs.push((record.frn, idx));
            parent_frns.push(record.parent_frn);
            self_parents.push(self_parent);
            let category = if record.is_dir {
                FileCategory::Other
            } else {
                FileCategory::from_file_name(&record.name)
            };
            nodes.push(AnalysisNode {
                name: record.name,
                parent: 0,
                child_start: 0,
                child_len: 0,
                size: record.size,
                allocated_size: record.allocated_size,
                subtree_size: record.size,
                subtree_allocated_size: record.allocated_size,
                subtree_files: u64::from(!record.is_dir),
                is_dir: record.is_dir,
                is_reparse: record.is_reparse,
                category,
            });
        }

        if is_cancelled() {
            return None;
        }

        // FRN -> node index lookup as a sorted vector (roughly half the
        // memory of a HashMap at millions of entries). Duplicate FRNs keep
        // the highest index, matching HashMap last-write-wins.
        frn_pairs.sort_unstable();
        if is_cancelled() {
            return None;
        }
        let mut frn_len = 0usize;
        for i in 0..frn_pairs.len() {
            if frn_len > 0 && frn_pairs[frn_len - 1].0 == frn_pairs[i].0 {
                frn_pairs[frn_len - 1] = frn_pairs[i];
            } else {
                frn_pairs[frn_len] = frn_pairs[i];
                frn_len += 1;
            }
        }
        frn_pairs.truncate(frn_len);
        frn_pairs.shrink_to_fit();
        // Pass 2: resolve parents and build the flat CSR child array.
        // Orphans (missing parent, self-parent, or a file recorded as
        // parent) attach to the tree root. The volume root record
        // (self-parent directory) becomes the tree root itself.
        let root = volume_root_idx.unwrap_or(0);
        if let Some(vr) = volume_root_idx {
            let node = &mut nodes[vr as usize];
            node.parent = vr;
            node.name = format!("{}:", drive_letter.to_ascii_uppercase());
        }
        let node_count = nodes.len();
        let mut resolved_parents: Vec<u32> = Vec::with_capacity(record_count);
        let mut resolved_idxs: Vec<u32> = Vec::with_capacity(record_count);
        let mut child_counts: Vec<u32> = vec![0; node_count];
        for (i, &parent_frn) in parent_frns.iter().enumerate() {
            if i % 4096 == 0 && is_cancelled() {
                return None;
            }
            let idx = (i + 1) as u32;
            if Some(idx) == volume_root_idx {
                continue;
            }
            let found = if self_parents[i] {
                None
            } else {
                frn_pairs
                    .binary_search_by_key(&parent_frn, |(frn, _)| *frn)
                    .ok()
                    .map(|pos| frn_pairs[pos].1)
            };
            let parent_idx = match found {
                Some(p) if p != idx && nodes[p as usize].is_dir => p,
                _ => root,
            };
            nodes[idx as usize].parent = parent_idx;
            resolved_parents.push(parent_idx);
            resolved_idxs.push(idx);
            child_counts[parent_idx as usize] += 1;
        }
        drop(parent_frns);
        drop(self_parents);
        drop(frn_pairs);

        // Prefix sums give each node its CSR slice. Children stay in
        // record order because parents are filled in ascending idx order.
        let mut start = 0u32;
        let mut cursors: Vec<u32> = Vec::with_capacity(node_count);
        for (idx, &count) in child_counts.iter().enumerate() {
            nodes[idx].child_start = start;
            nodes[idx].child_len = count;
            cursors.push(start);
            start += count;
        }
        let mut child_links: Vec<u32> = vec![0; start as usize];
        for (i, &parent_idx) in resolved_parents.iter().enumerate() {
            let slot = cursors[parent_idx as usize];
            child_links[slot as usize] = resolved_idxs[i];
            cursors[parent_idx as usize] = slot + 1;
        }
        drop(child_counts);
        drop(cursors);
        drop(resolved_parents);
        drop(resolved_idxs);

        // Pass 3: iterative post-order aggregation from the root. The child
        // structure is a forest (one parent per node), so cycles are simply
        // unreachable components and can never loop this traversal. Reparse
        // directories are leaves: their subtree is not descended into.
        // Counts and per-category totals are build-only scratch.
        let mut category_totals = CategoryTotals::default();
        let mut folder_counts: Vec<u32> = vec![0; node_count];
        for (idx, node) in nodes.iter().enumerate() {
            folder_counts[idx] = u32::from(node.is_dir);
        }
        let mut cat_slot: Vec<u32> = vec![u32::MAX; node_count];
        // Per-directory rollups: [allocated(8) | logical(8) | file count(8)].
        let mut dir_cat: Vec<[u64; 24]> = Vec::new();
        for (idx, node) in nodes.iter().enumerate() {
            if node.is_dir {
                cat_slot[idx] = dir_cat.len() as u32;
                dir_cat.push([0u64; 24]);
            }
        }
        fn child_range(node: &AnalysisNode) -> std::ops::Range<usize> {
            let start = node.child_start as usize;
            start..start + node.child_len as usize
        }
        let mut stack: Vec<(u32, bool)> = vec![(root, false)];
        let mut aggregation_steps = 0usize;
        while let Some((idx, visited)) = stack.pop() {
            aggregation_steps += 1;
            if aggregation_steps.is_multiple_of(4096) && is_cancelled() {
                return None;
            }
            let node = &nodes[idx as usize];
            if visited {
                let descend = node.is_dir && !node.is_reparse;
                if descend {
                    let mut totals = [0u64; 24];
                    let mut subtree = node.size;
                    let mut subtree_allocated = node.allocated_size;
                    let mut subtree_files = 0u64;
                    let mut folders = if node.is_dir { 1 } else { 0 };
                    for &child in &child_links[child_range(node)] {
                        subtree = subtree.saturating_add(nodes[child as usize].subtree_size);
                        subtree_allocated = subtree_allocated
                            .saturating_add(nodes[child as usize].subtree_allocated_size);
                        subtree_files =
                            subtree_files.saturating_add(nodes[child as usize].subtree_files);
                        folders += u64::from(folder_counts[child as usize]);
                        let slot = cat_slot[child as usize];
                        if slot != u32::MAX {
                            let cb = &dir_cat[slot as usize];
                            for i in 0..24 {
                                totals[i] = totals[i].saturating_add(cb[i]);
                            }
                        } else {
                            let c = &nodes[child as usize];
                            let ci = c.category.index();
                            totals[ci] = totals[ci].saturating_add(c.allocated_size);
                            totals[8 + ci] = totals[8 + ci].saturating_add(c.size);
                            totals[16 + ci] = totals[16 + ci].saturating_add(1);
                        }
                    }
                    let dominant = totals
                        .iter()
                        .take(8)
                        .enumerate()
                        .max_by_key(|(_, v)| **v)
                        .map(|(i, _)| FileCategory::ALL[i])
                        .unwrap_or(FileCategory::Other);
                    folder_counts[idx as usize] = folders as u32;
                    let n = &mut nodes[idx as usize];
                    n.subtree_size = subtree;
                    n.subtree_allocated_size = subtree_allocated;
                    n.subtree_files = subtree_files;
                    n.category = dominant;
                    dir_cat[cat_slot[idx as usize] as usize] = totals;
                } else if node.is_dir {
                    // Reparse dir: count itself, no descent.
                    let n = &mut nodes[idx as usize];
                    n.subtree_size = n.size;
                    n.subtree_allocated_size = n.allocated_size;
                    folder_counts[idx as usize] = 1;
                } else {
                    let ci = node.category.index();
                    category_totals.allocated[ci] =
                        category_totals.allocated[ci].saturating_add(node.allocated_size);
                    category_totals.logical[ci] =
                        category_totals.logical[ci].saturating_add(node.size);
                    category_totals.files[ci] = category_totals.files[ci].saturating_add(1);
                }
                continue;
            }
            stack.push((idx, true));
            if node.is_dir && !node.is_reparse {
                for &child in &child_links[child_range(node)] {
                    stack.push((child, false));
                }
            }
        }
        drop(cat_slot);
        drop(dir_cat);

        // Depths: build-only scratch assigned via a top-down walk (parent
        // depth known). Reparse directories are leaves: depth under a
        // junction is not meaningful. Unreachable nodes keep the initial 1.
        let mut depths: Vec<u32> = vec![1; node_count];
        depths[0] = 0;
        if let Some(vr) = volume_root_idx {
            depths[vr as usize] = 0;
        }
        let mut stack = vec![root];
        let mut depth_steps = 0usize;
        while let Some(idx) = stack.pop() {
            depth_steps += 1;
            if depth_steps.is_multiple_of(4096) && is_cancelled() {
                return None;
            }
            if nodes[idx as usize].is_reparse {
                continue;
            }
            let depth = depths[idx as usize];
            let range = child_range(&nodes[idx as usize]);
            for &child in &child_links[range] {
                if depths[child as usize] == 1 && child != root {
                    depths[child as usize] = depth + 1;
                }
                stack.push(child);
            }
        }
        let deepest_path = depths.iter().copied().max().unwrap_or(0);

        let total_size = nodes[root as usize].subtree_size;
        let total_allocated_size = nodes[root as usize].subtree_allocated_size;
        let total_files = nodes[root as usize].subtree_files;
        let total_folders = u64::from(folder_counts[root as usize]).saturating_sub(1);
        drop(depths);
        drop(folder_counts);

        nodes.shrink_to_fit();
        child_links.shrink_to_fit();
        Some(Self {
            drive_letter,
            nodes,
            child_links,
            root,
            total_size,
            total_allocated_size,
            total_files,
            total_folders,
            deepest_path,
            category_totals,
            build_elapsed: started.elapsed(),
        })
    }

    /// Chain of node indices from the volume root down to `idx` (both
    /// inclusive), i.e. the full ancestor path used by the breadcrumb trail.
    pub fn chain_to(&self, idx: u32) -> Vec<u32> {
        let mut chain = Vec::new();
        let mut current = idx;
        // Bounded walk: self-parent roots terminate; unreachable cycle
        // members (never placed by the treemap) cannot loop forever.
        while chain.len() <= self.nodes.len() {
            chain.push(current);
            let parent = self.nodes[current as usize].parent;
            if parent == current {
                break;
            }
            current = parent;
        }
        chain.reverse();
        chain
    }

    /// Full backslash-separated path of a node (for tooltips).
    pub fn path_of(&self, idx: u32) -> String {
        let mut parts: Vec<&str> = Vec::new();
        let mut current = idx;
        while current != self.root {
            parts.push(&self.nodes[current as usize].name);
            current = self.nodes[current as usize].parent;
        }
        let mut path = format!("{}:\\", self.drive_letter.to_ascii_uppercase());
        if parts.is_empty() {
            return path;
        }
        for part in parts.iter().rev() {
            path.push_str(part);
            path.push('\\');
        }
        path.pop();
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mtt_search_protocol::DiskAnalysisRecord;

    fn rec(frn: u64, parent: u64, name: &str, size: u64, is_dir: bool) -> DiskAnalysisRecord {
        DiskAnalysisRecord {
            frn,
            parent_frn: parent,
            name: name.to_string(),
            size,
            allocated_size: size,
            is_dir,
            is_reparse: false,
        }
    }

    fn snapshot(records: Vec<DiskAnalysisRecord>) -> DiskAnalysisSnapshot {
        DiskAnalysisSnapshot {
            drive_letter: 'C',
            records,
        }
    }

    #[test]
    fn builds_tree_with_orphan_and_cycle_safety() {
        let model = DiskAnalysisModel::build(snapshot(vec![
            rec(5, 5, "", 0, true),
            rec(10, 5, "dir", 0, true),
            rec(11, 10, "a.txt", 100, false),
            rec(12, 999, "orphan.bin", 50, false),
            // Cycle: unreachable component, must not hang nor aggregate.
            rec(20, 21, "cyc1", 0, true),
            rec(21, 20, "cyc2", 0, true),
        ]));

        assert_eq!(model.root, 1); // volume root record (frn 5)
                                   // root children: dir + orphan (cycle members stay unreachable).
        let root_children = model.children(model.root);
        assert_eq!(root_children.len(), 2);

        assert_eq!(model.total_files, 2);
        assert_eq!(model.total_size, 150);
        assert_eq!(model.deepest_path, 2);
    }

    #[test]
    fn reparse_dirs_are_not_descended() {
        let mut junction = rec(30, 5, "junction", 0, true);
        junction.is_reparse = true;
        let model = DiskAnalysisModel::build(snapshot(vec![
            rec(5, 5, "", 0, true),
            junction,
            rec(31, 30, "inside.txt", 5_000, false),
        ]));

        let junction_idx = model
            .nodes
            .iter()
            .position(|n| n.name == "junction")
            .unwrap() as u32;
        assert_eq!(model.nodes[junction_idx as usize].subtree_size, 0);
        assert_eq!(model.total_size, 0);
        assert_eq!(model.total_files, 0);
    }

    #[test]
    fn aggregates_subtrees_and_categories() {
        let model = DiskAnalysisModel::build(snapshot(vec![
            rec(5, 5, "", 0, true),
            rec(10, 5, "docs", 0, true),
            rec(11, 10, "a.txt", 100, false),
            rec(12, 10, "b.pdf", 300, false),
            rec(13, 5, "vid.mp4", 600, false),
        ]));

        let docs_idx = model.nodes.iter().position(|n| n.name == "docs").unwrap() as u32;
        assert_eq!(model.nodes[docs_idx as usize].subtree_size, 400);
        assert_eq!(
            model.nodes[docs_idx as usize].category,
            FileCategory::Documents
        );
        assert_eq!(model.total_folders, 1);
        assert_eq!(model.total_files, 3);

        let docs = FileCategory::Documents.index();
        let video = FileCategory::Video.index();
        assert_eq!(model.category_totals.allocated[docs], 400);
        assert_eq!(model.category_totals.files[docs], 2);
        assert_eq!(model.category_totals.logical[docs], 400);
        assert_eq!(model.category_totals.allocated[video], 600);
        assert_eq!(model.category_totals.files[video], 1);
    }

    #[test]
    fn keeps_logical_and_allocated_totals_independent() {
        let mut sparse = rec(11, 5, "sparse.bin", 1 << 40, false);
        sparse.allocated_size = 8_192;
        let model = DiskAnalysisModel::build(snapshot(vec![rec(5, 5, "", 0, true), sparse]));

        assert_eq!(model.total_size, 1 << 40);
        assert_eq!(model.total_allocated_size, 8_192);
        assert_eq!(
            model.category_totals.allocated[FileCategory::Other.index()],
            8_192
        );
        assert_eq!(
            model.category_totals.logical[FileCategory::Other.index()],
            1 << 40
        );
    }

    #[test]
    fn subtree_file_counts_are_kept_per_node() {
        let model = DiskAnalysisModel::build(snapshot(vec![
            rec(5, 5, "", 0, true),
            rec(10, 5, "docs", 0, true),
            rec(11, 10, "a.txt", 100, false),
            rec(12, 10, "sub", 0, true),
            rec(13, 12, "b.txt", 50, false),
        ]));

        let docs_idx = model.nodes.iter().position(|n| n.name == "docs").unwrap() as u32;
        let sub_idx = model.nodes.iter().position(|n| n.name == "sub").unwrap() as u32;
        assert_eq!(model.nodes[docs_idx as usize].subtree_files, 2);
        assert_eq!(model.nodes[sub_idx as usize].subtree_files, 1);
        assert_eq!(model.nodes[model.root as usize].subtree_files, 2);
        // Files count themselves once.
        let a = model.nodes.iter().position(|n| n.name == "a.txt").unwrap();
        assert_eq!(model.nodes[a].subtree_files, 1);
    }

    #[test]
    fn reparse_dirs_have_zero_file_count() {
        let mut junction = rec(30, 5, "junction", 0, true);
        junction.is_reparse = true;
        let model = DiskAnalysisModel::build(snapshot(vec![
            rec(5, 5, "", 0, true),
            junction,
            rec(31, 30, "inside.txt", 5_000, false),
        ]));
        assert_eq!(model.total_files, 0);
    }

    #[test]
    fn csr_children_preserve_record_order() {
        let model = DiskAnalysisModel::build(snapshot(vec![
            rec(5, 5, "", 0, true),
            rec(10, 5, "b.txt", 2, false),
            rec(11, 5, "a.txt", 1, false),
            rec(12, 5, "sub", 0, true),
            rec(13, 5, "c.txt", 3, false),
        ]));

        let names: Vec<&str> = model
            .children(model.root)
            .iter()
            .map(|&idx| model.nodes[idx as usize].name.as_str())
            .collect();
        assert_eq!(names, vec!["b.txt", "a.txt", "sub", "c.txt"]);
    }

    #[test]
    fn path_of_walks_parents() {
        let model = DiskAnalysisModel::build(snapshot(vec![
            rec(5, 5, "", 0, true),
            rec(10, 5, "dir", 0, true),
            rec(11, 10, "a.txt", 1, false),
        ]));
        let file_idx = model.nodes.iter().position(|n| n.name == "a.txt").unwrap() as u32;
        assert_eq!(model.path_of(file_idx), r"C:\dir\a.txt");
    }
}
