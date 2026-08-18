//! Immutable tree model for the disk usage analyzer.
//!
//! Built once (off the UI thread) from a [`DiskAnalysisSnapshot`] and shared
//! with the view as `Arc<DiskAnalysisModel>`. All tree invariants are
//! established here: orphan attachment, cycle exclusion, reparse non-descent
//! and bottom-up aggregation.

use mtt_search_protocol::DiskAnalysisSnapshot;
use std::collections::HashMap;
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
            "rs" | "js" | "jsx" | "tsx" | "py" | "c" | "cpp" | "h" | "hpp" | "cs"
            | "java" | "go" | "rb" | "php" | "html" | "css" | "json" | "toml" | "yml" | "yaml"
            | "xml" | "sql" | "sh" | "ps1" | "lua" => FileCategory::Code,
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
pub struct AnalysisNode {
    pub name: String,
    /// Index of the parent node; the synthetic root is its own parent.
    pub parent: u32,
    pub children: Vec<u32>,
    /// Own size in bytes (0 for directories).
    pub size: u64,
    /// Aggregated size of the subtree. Reparse directories do not descend.
    pub subtree_size: u64,
    /// Subtree file count.
    pub file_count: u64,
    /// Subtree folder count (includes this node when it is a directory).
    pub folder_count: u64,
    /// Depth from the synthetic root (root = 0).
    pub depth: u32,
    pub is_dir: bool,
    pub is_reparse: bool,
    /// Files: own category. Directories: dominant category by subtree bytes.
    pub category: FileCategory,
    /// Directories only: per-category subtree byte totals.
    pub cat_bytes: Option<Box<[u64; 8]>>,
}

/// Fully aggregated, read-only analysis of one volume.
pub struct DiskAnalysisModel {
    pub drive_letter: char,
    pub nodes: Vec<AnalysisNode>,
    /// Index of the synthetic volume root ("C:").
    pub root: u32,
    pub total_size: u64,
    pub total_files: u64,
    pub total_folders: u64,
    /// Maximum tree depth reached from the root.
    pub deepest_path: u32,
    /// Per-category (bytes, file count) over all files.
    pub category_totals: [u64; 16],
    pub build_elapsed: Duration,
}

impl DiskAnalysisModel {
    /// Build the tree model from a raw snapshot. O(n) passes; cycle-safe.
    pub fn build(snapshot: DiskAnalysisSnapshot) -> Self {
        let started = Instant::now();
        let drive_letter = snapshot.drive_letter;

        let mut nodes: Vec<AnalysisNode> = Vec::with_capacity(snapshot.records.len() + 1);
        // Synthetic root for the whole volume.
        let root = 0u32;
        nodes.push(AnalysisNode {
            name: format!("{}:", drive_letter.to_ascii_uppercase()),
            parent: 0,
            children: Vec::new(),
            size: 0,
            subtree_size: 0,
            file_count: 0,
            folder_count: 0,
            depth: 0,
            is_dir: true,
            is_reparse: false,
            category: FileCategory::Other,
            cat_bytes: Some(Box::new([0u64; 8])),
        });

        // Pass 1: create one node per record and map FRN -> node index.
        let mut frn_to_idx: HashMap<u64, u32> = HashMap::with_capacity(snapshot.records.len());
        for record in &snapshot.records {
            let idx = nodes.len() as u32;
            frn_to_idx.insert(record.frn, idx);
            let category = if record.is_dir {
                FileCategory::Other
            } else {
                FileCategory::from_file_name(&record.name)
            };
            nodes.push(AnalysisNode {
                name: record.name.clone(),
                parent: root,
                children: Vec::new(),
                size: record.size,
                subtree_size: record.size,
                file_count: u64::from(!record.is_dir),
                folder_count: u64::from(record.is_dir),
                depth: 1,
                is_dir: record.is_dir,
                is_reparse: record.is_reparse,
                category,
                cat_bytes: if record.is_dir {
                    Some(Box::new([0u64; 8]))
                } else {
                    None
                },
            });
        }

        // Pass 2: link children. Orphans (missing parent, self-parent, or a
        // file recorded as parent) attach to the tree root. The volume root
        // record (self-parent directory) becomes the tree root itself.
        let volume_root_idx: Option<u32> = snapshot
            .records
            .iter()
            .position(|r| r.parent_frn == r.frn && r.is_dir)
            .map(|p| (p + 1) as u32);
        let root = volume_root_idx.unwrap_or(0);
        if let Some(vr) = volume_root_idx {
            let node = &mut nodes[vr as usize];
            node.parent = vr;
            node.depth = 0;
            node.name = format!("{}:", drive_letter.to_ascii_uppercase());
            // Unused synthetic root must not skew deepest_path.
            nodes[0].depth = 0;
        }
        for (record, idx) in snapshot.records.iter().zip(1u32..) {
            if Some(idx) == volume_root_idx {
                continue;
            }
            let parent_idx = if record.parent_frn == record.frn {
                None
            } else {
                frn_to_idx.get(&record.parent_frn).copied()
            };
            let parent_idx = match parent_idx {
                Some(p) if p != idx && nodes[p as usize].is_dir => p,
                _ => root,
            };
            nodes[idx as usize].parent = parent_idx;
            nodes[parent_idx as usize].children.push(idx);
        }
        drop(frn_to_idx);

        // Pass 3: iterative post-order aggregation from the root. The child
        // structure is a forest (one parent per node), so cycles are simply
        // unreachable components and can never loop this traversal. Reparse
        // directories are leaves: their subtree is not descended into.
        let mut category_totals = [0u64; 16];
        let mut stack: Vec<(u32, bool)> = vec![(root, false)];
        while let Some((idx, visited)) = stack.pop() {
            let node = &nodes[idx as usize];
            if visited {
                let descend = node.is_dir && !node.is_reparse;
                if descend {
                    let mut totals = [0u64; 8];
                    let mut subtree = node.size;
                    let mut files = 0u64;
                    let mut folders = if node.is_dir { 1 } else { 0 };
                    for &child in &node.children {
                        let c = &nodes[child as usize];
                        subtree += c.subtree_size;
                        files += c.file_count;
                        folders += c.folder_count;
                        if let Some(cb) = &c.cat_bytes {
                            for i in 0..8 {
                                totals[i] += cb[i];
                            }
                        } else {
                            totals[c.category.index()] += c.size;
                        }
                    }
                    let dominant = totals
                        .iter()
                        .enumerate()
                        .max_by_key(|(_, v)| **v)
                        .map(|(i, _)| FileCategory::ALL[i])
                        .unwrap_or(FileCategory::Other);
                    let n = &mut nodes[idx as usize];
                    n.subtree_size = subtree;
                    n.file_count = files;
                    n.folder_count = folders;
                    n.category = dominant;
                    if let Some(cb) = n.cat_bytes.as_mut() {
                        **cb = totals;
                    }
                } else if node.is_dir {
                    // Reparse dir: count itself, no descent.
                    let n = &mut nodes[idx as usize];
                    n.subtree_size = n.size;
                    n.file_count = 0;
                    n.folder_count = 1;
                } else {
                    category_totals[node.category.index()] += node.size;
                    category_totals[node.category.index() + 8] += 1;
                }
                continue;
            }
            stack.push((idx, true));
            if node.is_dir && !node.is_reparse {
                for &child in &node.children {
                    stack.push((child, false));
                }
            }
        }

        // Depths: assign via a top-down walk (parent depth known). Reparse
        // directories are leaves: depth under a junction is not meaningful.
        let mut stack = vec![root];
        while let Some(idx) = stack.pop() {
            let (depth, children, descend) = {
                let n = &nodes[idx as usize];
                (n.depth, n.children.clone(), !n.is_reparse)
            };
            if !descend {
                continue;
            }
            for child in children {
                if nodes[child as usize].depth == 1 && child != root {
                    nodes[child as usize].depth = depth + 1;
                }
                stack.push(child);
            }
        }
        // Recompute deepest from the top-down walk values.
        let deepest_path = nodes.iter().map(|n| n.depth).max().unwrap_or(0);

        let root_node = &nodes[root as usize];
        let total_size = root_node.subtree_size;
        let total_files = root_node.file_count;
        let total_folders = root_node.folder_count.saturating_sub(1);
        Self {
            drive_letter,
            nodes,
            root,
            total_size,
            total_files,
            total_folders,
            deepest_path,
            category_totals,
            build_elapsed: started.elapsed(),
        }
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
        let root_children = &model.nodes[model.root as usize].children;
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

        let docs_idx = model
            .nodes
            .iter()
            .position(|n| n.name == "docs")
            .unwrap() as u32;
        assert_eq!(model.nodes[docs_idx as usize].subtree_size, 400);
        assert_eq!(model.nodes[docs_idx as usize].file_count, 2);
        assert_eq!(model.nodes[docs_idx as usize].folder_count, 1);
        assert_eq!(
            model.nodes[docs_idx as usize].category,
            FileCategory::Documents
        );
        assert_eq!(model.total_folders, 1);

        let docs = FileCategory::Documents.index();
        let video = FileCategory::Video.index();
        assert_eq!(model.category_totals[docs], 400);
        assert_eq!(model.category_totals[docs + 8], 2);
        assert_eq!(model.category_totals[video], 600);
        assert_eq!(model.category_totals[video + 8], 1);
    }

    #[test]
    fn path_of_walks_parents() {
        let model = DiskAnalysisModel::build(snapshot(vec![
            rec(5, 5, "", 0, true),
            rec(10, 5, "dir", 0, true),
            rec(11, 10, "a.txt", 1, false),
        ]));
        let file_idx = model
            .nodes
            .iter()
            .position(|n| n.name == "a.txt")
            .unwrap() as u32;
        assert_eq!(model.path_of(file_idx), r"C:\dir\a.txt");
    }
}
