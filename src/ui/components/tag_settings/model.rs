use crate::domain::{auto_tag_rule::AutoTagRule, file_tag::FileTag, tag_tree};
use rustc_hash::{FxHashMap, FxHashSet};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Tab {
    #[default]
    Tags,
    AutoTag,
}

#[derive(Clone, Default)]
pub(super) struct ListState {
    pub search: String,
    pub selected: Option<i64>,
    pub reset_scroll: bool,
    pub focus_selected: bool,
}

impl ListState {
    pub fn select(&mut self, id: i64) {
        self.selected = Some(id);
    }

    pub fn search_changed(&mut self) {
        self.reset_scroll = true;
        // Do not apply a queued arrow-key focus transfer after the user has
        // switched to typing in the search field.
        self.focus_selected = false;
    }

    pub fn reconcile(&mut self, exists: impl FnOnce(i64) -> bool) {
        if self.selected.is_some_and(|id| !exists(id)) {
            self.selected = None;
            self.focus_selected = false;
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct BrowserState {
    pub tab: Tab,
    pub tags: ListState,
    pub rules: ListState,
    pub collapsed: FxHashSet<i64>,
    pub creation: Option<Tab>,
}

/// Breadcrumbs disambiguate identically named tags in different branches.
pub(super) fn tag_labels(defs: &FxHashMap<i64, FileTag>) -> FxHashMap<i64, String> {
    defs.keys()
        .map(|id| {
            let names: Vec<_> = tag_tree::ancestor_chain(defs, *id)
                .into_iter()
                .rev()
                .filter_map(|ancestor| defs.get(&ancestor).map(|tag| tag.name.as_str()))
                .collect();
            (*id, names.join(" / "))
        })
        .collect()
}

pub(super) fn visible_tags(
    defs: &FxHashMap<i64, FileTag>,
    sorted: &[i64],
    query: &str,
    collapsed: &FxHashSet<i64>,
) -> Vec<i64> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return sorted
            .iter()
            .copied()
            .filter(|id| {
                !tag_tree::ancestor_chain(defs, *id)
                    .into_iter()
                    .skip(1)
                    .any(|ancestor| collapsed.contains(&ancestor))
            })
            .collect();
    }
    // Search temporarily reveals matching descendants and their context,
    // without changing the user's expansion choices.
    let mut visible = FxHashSet::default();
    for tag in defs
        .values()
        .filter(|tag| tag.name.to_lowercase().contains(&query))
    {
        visible.extend(tag_tree::ancestor_chain(defs, tag.id));
    }
    sorted
        .iter()
        .copied()
        .filter(|id| visible.contains(id))
        .collect()
}

pub(super) fn visible_rules(
    rules: &[AutoTagRule],
    labels: &FxHashMap<i64, String>,
    query: &str,
) -> Vec<i64> {
    let query = query.trim().to_lowercase();
    rules
        .iter()
        .filter(|rule| {
            query.is_empty()
                || rule
                    .folder_path
                    .to_string_lossy()
                    .to_lowercase()
                    .contains(&query)
                || labels
                    .get(&rule.tag_id)
                    .is_some_and(|label| label.to_lowercase().contains(&query))
        })
        .map(|rule| rule.id)
        .collect()
}

/// Arrow navigation belongs to the list, never to a text editor or combo box.
pub(super) fn adjacent(ids: &[i64], selected: Option<i64>, forward: bool) -> Option<i64> {
    if ids.is_empty() {
        return None;
    }
    let index = selected.and_then(|id| ids.iter().position(|candidate| *candidate == id));
    Some(
        ids[match index {
            Some(index) if forward => (index + 1).min(ids.len() - 1),
            Some(index) => index.saturating_sub(1),
            None if forward => 0,
            None => ids.len() - 1,
        }],
    )
}

#[cfg(test)]
mod tests;
