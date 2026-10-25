//! Group identity is independent of its localized or formatted display label.
use super::GroupKey;
use std::hash::{Hash, Hasher};

impl PartialEq for GroupKey {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Name(a), Self::Name(b)) => a == b,
            (Self::Date(a), Self::Date(b)) => a == b,
            (Self::Type(a), Self::Type(b)) => a == b,
            (Self::Size(a), Self::Size(b)) => a == b,
            (Self::LocalDrives, Self::LocalDrives)
            | (Self::NetworkDrives, Self::NetworkDrives)
            | (Self::NetworkLocations, Self::NetworkLocations) => true,
            (
                Self::Column {
                    column: a,
                    identity: a_id,
                    ..
                },
                Self::Column {
                    column: b,
                    identity: b_id,
                    ..
                },
            ) => a == b && a_id == b_id,
            _ => false,
        }
    }
}

impl Eq for GroupKey {}

impl Hash for GroupKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::Name(value) => value.hash(state),
            Self::Date(value) => value.hash(state),
            Self::Type(value) => value.hash(state),
            Self::Size(value) => value.hash(state),
            Self::Column {
                column, identity, ..
            } => {
                column.hash(state);
                identity.hash(state);
            }
            Self::LocalDrives | Self::NetworkDrives | Self::NetworkLocations => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::list_columns::ColumnId;
    #[test]
    fn changing_a_group_caption_preserves_hash_identity_and_collapsed_state() {
        let original = GroupKey::Column {
            column: ColumnId::Tags,
            identity: "example".into(),
            label: "Example".into(),
        };
        let renamed = GroupKey::Column {
            column: ColumnId::Tags,
            identity: "example".into(),
            label: "EXAMPLE".into(),
        };
        assert_eq!(original, renamed);
        let collapsed = rustc_hash::FxHashSet::from_iter([original.clone()]);
        assert!(collapsed.contains(&renamed));
        let different_column = GroupKey::Column {
            column: ColumnId::OriginalLocation,
            identity: "example".into(),
            label: "Example".into(),
        };
        assert_ne!(original, different_column);
        assert!(!collapsed.contains(&different_column));
    }
}
