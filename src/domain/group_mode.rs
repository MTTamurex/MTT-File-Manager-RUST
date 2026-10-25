use super::file_entry::GroupMode;

impl GroupMode {
    pub fn preference_value(self) -> String {
        match self {
            Self::None => "none".into(),
            Self::Name => "name".into(),
            Self::Date => "date".into(),
            Self::Type => "type".into(),
            Self::Size => "size".into(),
            Self::Column(id) => format!(
                "column:{}",
                serde_json::to_string(&id).expect("column identity serializes")
            ),
        }
    }

    pub fn from_preference(value: &str) -> Self {
        match value {
            "name" => Self::Name,
            "date" => Self::Date,
            "type" => Self::Type,
            "size" => Self::Size,
            _ => value
                .strip_prefix("column:")
                .and_then(|id| serde_json::from_str(id).ok())
                .map(Self::Column)
                .unwrap_or(Self::None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::list_columns::{ColumnId, PropertyId};
    #[test]
    fn grouping_preferences_preserve_existing_modes_and_column_identities() {
        for mode in [
            GroupMode::None,
            GroupMode::Name,
            GroupMode::Date,
            GroupMode::Type,
            GroupMode::Size,
            GroupMode::Column(ColumnId::Tags),
            GroupMode::Column(ColumnId::Property(PropertyId {
                fmtid: u128::MAX,
                pid: 42,
            })),
        ] {
            assert_eq!(GroupMode::from_preference(&mode.preference_value()), mode);
        }
        assert_eq!(
            GroupMode::from_preference("column:invalid"),
            GroupMode::None
        );
    }
}
