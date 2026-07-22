use std::collections::HashMap;

/// Reverse index `parent FRN -> child FRNs`, stored as an immutable CSR
/// (compressed-sparse-row) base plus a per-directory copy-on-write overlay.
///
/// # Layout
/// The base is three parallel arrays built by [`ChildIndex::from_edges`]:
///   * `dir_frns`  — sorted, unique parent FRNs present in the base.
///   * `offsets`   — `dir_frns.len() + 1` entries; the children of `dir_frns[i]`
///     live in `children[offsets[i]..offsets[i + 1]]`.
///   * `children`  — flat child FRNs grouped by parent, each group sorted+deduped.
///
/// This replaces the previous `HashMap<u64, Vec<u64>>`, eliminating the
/// per-directory `Vec` header (24 bytes) and the hash-table slot overhead
/// (~32 bytes) for every directory, which dominated the reverse-index cost.
///
/// # Mutations
/// USN updates mutate a directory's child list. On the first write to a
/// directory, its list is copied from the CSR base into `overlay` (copy on
/// write), leaving the base untouched. Once a directory has an overlay entry
/// the overlay is authoritative for it and the base slice is ignored — an
/// empty overlay `Vec` therefore means "explicitly no children" and correctly
/// shadows any stale base slice.
///
/// A full [`ChildIndex::from_edges`] rebuild (performed on every DB/binary load
/// and after bulk scans) folds all overlay deltas back into a fresh compact
/// base, so the overlay only ever holds within-session changes.
#[derive(Debug)]
pub struct ChildIndex {
    dir_frns: Vec<u64>,
    offsets: Vec<u32>,
    children: Vec<u64>,
    overlay: HashMap<u64, Vec<u64>>,
}

impl ChildIndex {
    /// Create an empty index (no base, no overlay).
    pub fn new() -> Self {
        Self {
            dir_frns: Vec::new(),
            offsets: Vec::new(),
            children: Vec::new(),
            overlay: HashMap::new(),
        }
    }

    /// Build a compact CSR base from `(parent, child)` edges. The input is
    /// sorted and de-duplicated, so callers may push edges in any order and
    /// may include duplicate pairs (e.g. a long name + 8.3 short name that map
    /// to the same parent). The resulting overlay is empty.
    pub fn from_edges(mut edges: Vec<(u64, u64)>) -> Self {
        edges.sort_unstable();
        edges.dedup();

        let mut dir_frns: Vec<u64> = Vec::new();
        let mut offsets: Vec<u32> = Vec::new();
        let mut children: Vec<u64> = Vec::with_capacity(edges.len());

        let mut current_parent: Option<u64> = None;
        for (parent, child) in edges {
            if current_parent != Some(parent) {
                dir_frns.push(parent);
                // Debug-only guard: the load path caps record counts well below
                // u32::MAX edges, so this offset never truncates in practice.
                debug_assert!(children.len() <= u32::MAX as usize);
                offsets.push(children.len() as u32);
                current_parent = Some(parent);
            }
            children.push(child);
        }
        offsets.push(children.len() as u32);

        dir_frns.shrink_to_fit();
        offsets.shrink_to_fit();
        children.shrink_to_fit();

        Self {
            dir_frns,
            offsets,
            children,
            overlay: HashMap::new(),
        }
    }

    /// Return the child FRNs of `parent`, or `None` if `parent` has no entry.
    /// The overlay (when present) shadows the base for that directory.
    #[inline]
    pub fn get(&self, parent: u64) -> Option<&[u64]> {
        if let Some(list) = self.overlay.get(&parent) {
            return Some(list.as_slice());
        }
        let index = self.dir_frns.binary_search(&parent).ok()?;
        let start = self.offsets[index] as usize;
        let end = self.offsets[index + 1] as usize;
        Some(&self.children[start..end])
    }

    /// Ensure `parent` has an overlay entry, seeding it from the base slice on
    /// first write (copy on write). Returns the mutable overlay list.
    fn overlay_entry(&mut self, parent: u64) -> &mut Vec<u64> {
        if !self.overlay.contains_key(&parent) {
            let seed = match self.dir_frns.binary_search(&parent) {
                Ok(index) => {
                    let start = self.offsets[index] as usize;
                    let end = self.offsets[index + 1] as usize;
                    self.children[start..end].to_vec()
                }
                Err(_) => Vec::new(),
            };
            self.overlay.insert(parent, seed);
        }
        self.overlay
            .get_mut(&parent)
            .expect("overlay entry just inserted")
    }

    /// Add `child` under `parent` (no-op if already present).
    pub fn add_child(&mut self, parent: u64, child: u64) {
        if self.get(parent).is_some_and(|list| list.contains(&child)) {
            return;
        }
        let list = self.overlay_entry(parent);
        list.push(child);
    }

    /// Remove `child` from `parent`. Copies the directory into the overlay only
    /// when it actually contains `child`, so read-only directories stay in the
    /// compact base.
    pub fn remove_child(&mut self, parent: u64, child: u64) {
        if self.get(parent).is_some_and(|list| list.contains(&child)) {
            let parent_exists_in_base = self.dir_frns.binary_search(&parent).is_ok();
            let list = self.overlay_entry(parent);
            list.retain(|&c| c != child);
            if list.is_empty() && !parent_exists_in_base {
                self.overlay.remove(&parent);
            }
        }
    }

    /// Fold all copy-on-write deltas into a fresh compact CSR base.
    fn compact_overlay(&mut self) {
        if self.overlay.is_empty() {
            return;
        }

        for children in self.overlay.values_mut() {
            children.sort_unstable();
            children.dedup();
        }

        let mut parents = self.dir_frns.clone();
        parents.extend(self.overlay.keys().copied());
        parents.sort_unstable();
        parents.dedup();

        let total_children = parents
            .iter()
            .map(|parent| self.get(*parent).map_or(0, <[u64]>::len))
            .sum();
        let mut dir_frns = Vec::with_capacity(parents.len());
        let mut offsets = Vec::with_capacity(parents.len() + 1);
        let mut children = Vec::with_capacity(total_children);

        for parent in parents {
            let Some(list) = self.get(parent) else {
                continue;
            };
            if list.is_empty() {
                continue;
            }
            dir_frns.push(parent);
            offsets.push(u32::try_from(children.len()).expect("child index exceeds u32 offsets"));
            children.extend_from_slice(list);
        }
        offsets.push(u32::try_from(children.len()).expect("child index exceeds u32 offsets"));

        self.dir_frns = dir_frns;
        self.offsets = offsets;
        self.children = children;
        self.overlay.clear();
    }

    /// Consolidate deltas and release excess capacity after stabilization.
    pub fn shrink_to_fit(&mut self) {
        self.compact_overlay();
        self.dir_frns.shrink_to_fit();
        self.offsets.shrink_to_fit();
        self.children.shrink_to_fit();
        self.overlay.shrink_to_fit();
        for list in self.overlay.values_mut() {
            list.shrink_to_fit();
        }
    }

    /// Drop all base and overlay data (for a full re-scan).
    pub fn clear(&mut self) {
        self.dir_frns.clear();
        self.offsets.clear();
        self.children.clear();
        self.overlay.clear();
    }
}

impl Default for ChildIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::ChildIndex;

    #[test]
    fn from_edges_groups_sorts_and_dedups() {
        let index = ChildIndex::from_edges(vec![(5, 30), (5, 10), (5, 10), (7, 40)]);
        assert_eq!(index.get(5), Some([10u64, 30].as_slice()));
        assert_eq!(index.get(7), Some([40u64].as_slice()));
        assert_eq!(index.get(99), None);
    }

    #[test]
    fn add_child_copies_base_on_write_without_touching_other_dirs() {
        let mut index = ChildIndex::from_edges(vec![(5, 10), (7, 40)]);
        index.add_child(5, 20);
        // Overlayed directory reflects the new child on top of the base.
        let mut children = index.get(5).unwrap().to_vec();
        children.sort_unstable();
        assert_eq!(children, vec![10, 20]);
        // Untouched directory still served from the compact base.
        assert_eq!(index.get(7), Some([40u64].as_slice()));
    }

    #[test]
    fn add_child_is_idempotent() {
        let mut index = ChildIndex::from_edges(vec![(5, 10)]);
        index.add_child(5, 10);
        index.add_child(5, 10);
        assert_eq!(index.get(5), Some([10u64].as_slice()));
    }

    #[test]
    fn remove_child_shadows_base_and_can_empty_a_directory() {
        let mut index = ChildIndex::from_edges(vec![(5, 10), (5, 20)]);
        index.remove_child(5, 10);
        assert_eq!(index.get(5), Some([20u64].as_slice()));
        index.remove_child(5, 20);
        // Empty overlay shadows the stale base slice.
        assert_eq!(index.get(5), Some([].as_slice()));
    }

    #[test]
    fn add_child_to_new_directory_creates_overlay_entry() {
        let mut index = ChildIndex::new();
        index.add_child(5, 10);
        index.add_child(5, 20);
        let mut children = index.get(5).unwrap().to_vec();
        children.sort_unstable();
        assert_eq!(children, vec![10, 20]);
    }

    #[test]
    fn remove_child_absent_is_noop_and_keeps_base_shared() {
        let mut index = ChildIndex::from_edges(vec![(5, 10)]);
        index.remove_child(5, 999);
        assert_eq!(index.get(5), Some([10u64].as_slice()));
    }

    #[test]
    fn shrink_to_fit_folds_overlay_and_drops_empty_transient_parents() {
        let mut index = ChildIndex::from_edges(vec![(5, 10), (7, 40)]);
        index.add_child(5, 20);
        index.add_child(9, 90);
        index.remove_child(9, 90);
        index.remove_child(7, 40);

        index.shrink_to_fit();

        assert_eq!(index.get(5), Some([10u64, 20].as_slice()));
        assert_eq!(index.get(7), None);
        assert_eq!(index.get(9), None);
        assert!(index.overlay.is_empty());
    }
}
