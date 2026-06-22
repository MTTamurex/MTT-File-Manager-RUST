//! Batch rename state and name-generation logic.

use rust_i18n::t;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

// ── Enums ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum NumberPosition {
    Suffix,
    Prefix,
}

impl NumberPosition {
    pub fn display_name(&self) -> String {
        match self {
            NumberPosition::Suffix => t!("batch_rename.pos_suffix").to_string(),
            NumberPosition::Prefix => t!("batch_rename.pos_prefix").to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum NumberSeparator {
    Parentheses,
    Underscore,
    Dash,
    Space,
    None,
}

impl NumberSeparator {
    pub fn display_name(&self) -> String {
        match self {
            NumberSeparator::Parentheses => t!("batch_rename.sep_parentheses").to_string(),
            NumberSeparator::Underscore => t!("batch_rename.sep_underscore").to_string(),
            NumberSeparator::Dash => t!("batch_rename.sep_dash").to_string(),
            NumberSeparator::Space => t!("batch_rename.sep_space").to_string(),
            NumberSeparator::None => t!("batch_rename.sep_none").to_string(),
        }
    }
}

// ── DragState ────────────────────────────────────────────────────────────────

/// Tracks an in-progress drag-to-reorder operation in the modal list.
#[derive(Debug, Clone)]
pub struct DragState {
    pub dragging_idx: usize,
    pub hover_idx: usize,
}

// ── PreviewRow ───────────────────────────────────────────────────────────────

/// One row in the live preview table.
#[derive(Debug, Clone)]
pub struct PreviewRow {
    pub source: PathBuf,
    pub old_name: String,
    pub new_name: String,
    /// true if the generated destination already exists on disk or another row
    /// in this batch generates the same destination path.
    pub conflict: bool,
}

// ── BatchRenameState ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct BatchRenameState {
    /// Ordered list of files to rename (user may reorder via drag).
    pub sources: Vec<PathBuf>,
    /// Base name template (no extension).
    pub name_template: String,
    pub position: NumberPosition,
    pub separator: NumberSeparator,
    pub start: u32,
    pub step: u32,
    /// Zero-padding width (0 = no padding).
    pub padding: usize,
    /// Active drag-to-reorder state, `None` when idle.
    pub drag_state: Option<DragState>,
}

impl BatchRenameState {
    /// Creates a new state seeded with `sources`.
    /// The name template defaults to the stem of the first file.
    pub fn new(sources: Vec<PathBuf>) -> Self {
        let name_template = sources
            .first()
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        Self {
            sources,
            name_template,
            position: NumberPosition::Suffix,
            separator: NumberSeparator::Parentheses,
            start: 1,
            step: 1,
            padding: 0,
            drag_state: None,
        }
    }

    /// Returns `true` if any row in `preview` has a naming conflict.
    pub fn has_conflicts(preview: &[PreviewRow]) -> bool {
        preview.iter().any(|r| r.conflict)
    }

    /// Generates the ordered list of (old_name, new_name, conflict) triples.
    ///
    /// Conflict detection:
    /// - The generated destination path exists on disk, OR
    /// - another row in this same batch generates the same destination path.
    pub fn compute_preview(&self) -> Vec<PreviewRow> {
        struct PendingRow {
            source: PathBuf,
            old_name: String,
            new_name: String,
            dest: Option<PathBuf>,
        }

        let mut pending = Vec::with_capacity(self.sources.len());
        let mut dest_counts: HashMap<String, usize> = HashMap::new();
        let mut n = self.start as u64;

        for source in &self.sources {
            let old_name = source
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();

            let ext = source.extension().and_then(|s| s.to_str()).unwrap_or("");

            let num_str = if self.padding > 0 {
                format!("{:0>width$}", n, width = self.padding)
            } else {
                n.to_string()
            };

            let new_name = self.build_new_name(&self.name_template, &num_str, ext);

            let dest = source.parent().map(|p| p.join(&new_name));
            if let Some(dest) = &dest {
                *dest_counts.entry(destination_key(dest)).or_insert(0) += 1;
            }

            pending.push(PendingRow {
                source: source.clone(),
                old_name,
                new_name,
                dest,
            });

            n = n.saturating_add(self.step as u64);
        }

        pending
            .into_iter()
            .map(|row| {
                let conflict = row.dest.as_ref().is_some_and(|dest| {
                    dest.exists()
                        || dest_counts
                            .get(&destination_key(dest))
                            .is_some_and(|count| *count > 1)
                });

                PreviewRow {
                    source: row.source,
                    old_name: row.old_name,
                    new_name: row.new_name,
                    conflict,
                }
            })
            .collect()
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    fn build_new_name(&self, base: &str, num_str: &str, ext: &str) -> String {
        let ext_suffix = if ext.is_empty() {
            String::new()
        } else {
            format!(".{}", ext)
        };

        match (&self.position, &self.separator) {
            (NumberPosition::Suffix, NumberSeparator::Parentheses) => {
                format!("{} ({}){}", base, num_str, ext_suffix)
            }
            (NumberPosition::Suffix, NumberSeparator::Underscore) => {
                format!("{}_{}{}", base, num_str, ext_suffix)
            }
            (NumberPosition::Suffix, NumberSeparator::Dash) => {
                format!("{}-{}{}", base, num_str, ext_suffix)
            }
            (NumberPosition::Suffix, NumberSeparator::Space) => {
                format!("{} {}{}", base, num_str, ext_suffix)
            }
            (NumberPosition::Suffix, NumberSeparator::None) => {
                format!("{}{}{}", base, num_str, ext_suffix)
            }
            (NumberPosition::Prefix, NumberSeparator::Parentheses) => {
                format!("({}) {}{}", num_str, base, ext_suffix)
            }
            (NumberPosition::Prefix, NumberSeparator::Underscore) => {
                format!("{}_{}{}", num_str, base, ext_suffix)
            }
            (NumberPosition::Prefix, NumberSeparator::Dash) => {
                format!("{}-{}{}", num_str, base, ext_suffix)
            }
            (NumberPosition::Prefix, NumberSeparator::Space) => {
                format!("{} {}{}", num_str, base, ext_suffix)
            }
            (NumberPosition::Prefix, NumberSeparator::None) => {
                format!("{}{}{}", num_str, base, ext_suffix)
            }
        }
    }
}

fn destination_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    #[test]
    fn selected_source_destination_is_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let source_one = dir.path().join("Photo1.txt");
        let source_two = dir.path().join("Photo2.txt");
        File::create(&source_one).unwrap();
        File::create(&source_two).unwrap();

        let mut state = BatchRenameState::new(vec![source_one, source_two]);
        state.name_template = "Photo".to_string();
        state.separator = NumberSeparator::None;
        state.start = 2;
        state.step = 1;

        let preview = state.compute_preview();

        assert!(preview[0].conflict);
        assert!(!preview[1].conflict);
    }

    #[test]
    fn no_op_rename_is_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("Photo1.txt");
        File::create(&source).unwrap();

        let mut state = BatchRenameState::new(vec![source]);
        state.name_template = "Photo".to_string();
        state.separator = NumberSeparator::None;
        state.start = 1;

        let preview = state.compute_preview();

        assert!(preview[0].conflict);
    }

    #[test]
    fn duplicate_destinations_in_same_batch_are_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        let source_one = dir.path().join("A.txt");
        let source_two = dir.path().join("B.txt");
        File::create(&source_one).unwrap();
        File::create(&source_two).unwrap();

        let mut state = BatchRenameState::new(vec![source_one, source_two]);
        state.name_template = "Photo".to_string();
        state.separator = NumberSeparator::None;
        state.start = 1;
        state.step = 0;

        let preview = state.compute_preview();

        assert!(preview.iter().all(|row| row.conflict));
    }
}
