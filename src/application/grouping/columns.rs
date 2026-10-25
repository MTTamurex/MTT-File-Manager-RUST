//! Project complete worker classifications onto the current filtered/sorted item indices.
use super::{GroupKey, GroupProjection, GroupSection};
use crate::domain::{file_entry::FileEntry, list_columns::ColumnId};
use std::{collections::HashMap, path::PathBuf};

#[derive(Default)]
pub struct ColumnGroups {
    pub assignments: HashMap<PathBuf, GroupKey>,
    pub order: HashMap<GroupKey, usize>,
    pub deferred: bool,
}

pub fn missing(column: ColumnId) -> GroupKey {
    GroupKey::Column {
        column,
        identity: "missing".into(),
        label: String::new(),
    }
}

pub fn date_category(timestamp: u64) -> GroupKey {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_secs())
        .unwrap_or(0);
    GroupKey::Date(super::classify_date(
        timestamp,
        super::unix_to_local_date(now),
    ))
}

pub fn alphabetic_category(text: &str) -> GroupKey {
    GroupKey::Name(super::classify_name(text))
}

pub fn size_category(bytes: u64) -> GroupKey {
    GroupKey::Size(super::classify_byte_size(bytes))
}

impl ColumnGroups {
    pub fn project(
        &self,
        items: &[FileEntry],
        column: ColumnId,
        descending: bool,
    ) -> GroupProjection {
        let missing = missing(column);
        let mut positions = HashMap::new();
        let mut sections: Vec<(GroupKey, Vec<usize>)> = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let key = self.assignments.get(&item.path).unwrap_or(&missing);
            let position = *positions.entry(key.clone()).or_insert_with(|| {
                sections.push((key.clone(), Vec::new()));
                sections.len() - 1
            });
            sections[position].1.push(index);
        }
        sections.sort_by(|(a, _), (b, _)| {
            match (a == &missing, b == &missing) {
                (true, false) => return std::cmp::Ordering::Greater,
                (false, true) => return std::cmp::Ordering::Less,
                _ => {}
            }
            let order = self.order.get(a).cmp(&self.order.get(b));
            if descending {
                order.reverse()
            } else {
                order
            }
        });
        GroupProjection {
            sections: sections
                .into_iter()
                .map(|(key, indices)| GroupSection {
                    key,
                    item_indices: indices.into(),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn groups_follow_filtered_sorted_indices_keep_missing_last_and_support_collapsing() {
        let a = FileEntry::from_path("first.txt".into(), false);
        let b = FileEntry::from_path("second.txt".into(), false);
        let c = FileEntry::from_path("third.txt".into(), false);
        let first = GroupKey::Column {
            column: ColumnId::Tags,
            identity: "first".into(),
            label: "First".into(),
        };
        let second = GroupKey::Column {
            column: ColumnId::Tags,
            identity: "second".into(),
            label: "Second".into(),
        };
        let groups = ColumnGroups {
            assignments: HashMap::from([
                (a.path.clone(), first.clone()),
                (b.path.clone(), second.clone()),
            ]),
            order: HashMap::from([(first.clone(), 0), (second.clone(), 1)]),
            deferred: false,
        };
        let items = [c, b, a];
        for descending in [false, true] {
            let projected = groups.project(&items, ColumnId::Tags, descending);
            assert_eq!(
                projected.sections.last().unwrap().item_indices.as_ref(),
                [0]
            );
            assert_eq!(
                projected.sections[0].item_indices.as_ref(),
                [if descending { 1 } else { 2 }]
            );
            let collapsed = rustc_hash::FxHashSet::from_iter([first.clone()]);
            assert!(
                !super::super::visible_item_indices(&projected, Some(&collapsed), items.len())
                    .contains(&2)
            );
        }
    }
}
