use super::file_entry::SortMode;

impl SortMode {
    pub fn preference_value(self) -> String {
        match self {
            Self::Name => "name".into(),
            Self::Date => "date".into(),
            Self::Size => "size".into(),
            Self::Type => "type".into(),
            Self::DriveTotalSpace => "drive_total".into(),
            Self::DriveFreeSpace => "drive_free".into(),
            Self::DriveLetter => "drive_letter".into(),
            Self::Column(id) => format!(
                "column:{}",
                serde_json::to_string(&id).expect("column identity serializes")
            ),
        }
    }
    pub fn from_preference(value: &str) -> Self {
        match value {
            "date" => Self::Date,
            "size" => Self::Size,
            "type" => Self::Type,
            "drive_total" => Self::DriveTotalSpace,
            "drive_free" => Self::DriveFreeSpace,
            "drive_letter" => Self::DriveLetter,
            _ => value
                .strip_prefix("column:")
                .and_then(|id| serde_json::from_str(id).ok())
                .map(Self::Column)
                .unwrap_or(Self::Name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::list_columns::{ColumnId, PropertyId};
    #[test]
    fn shell_sort_identity_round_trips_without_translated_labels() {
        for id in [
            ColumnId::Tags,
            ColumnId::OriginalLocation,
            ColumnId::Property(PropertyId {
                fmtid: u128::MAX,
                pid: 42,
            }),
        ] {
            let mode = SortMode::Column(id);
            assert_eq!(SortMode::from_preference(&mode.preference_value()), mode);
        }
        assert_eq!(SortMode::from_preference("column:invalid"), SortMode::Name);
    }
}
