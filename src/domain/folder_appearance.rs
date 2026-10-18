/// User-selected folder artwork, independent of the interface theme.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FolderAppearance {
    #[default]
    Original,
    PastelYellow,
    BlueGray,
    DarkSlateBlue,
    Lavender,
}

impl FolderAppearance {
    pub const PREFERENCE_KEY: &'static str = "folder_appearance";
    pub const ALL: [Self; 5] = [
        Self::Original,
        Self::PastelYellow,
        Self::BlueGray,
        Self::DarkSlateBlue,
        Self::Lavender,
    ];

    pub fn from_preference(value: &str) -> Self {
        match value {
            "soft" | "pastel_yellow" => Self::PastelYellow,
            "blue_gray" => Self::BlueGray,
            "sage_green" | "dark_slate_blue" => Self::DarkSlateBlue,
            "lavender" => Self::Lavender,
            _ => Self::Original,
        }
    }

    pub fn preference(self) -> &'static str {
        match self {
            Self::Original => "original",
            Self::PastelYellow => "pastel_yellow",
            Self::BlueGray => "blue_gray",
            Self::DarkSlateBlue => "dark_slate_blue",
            Self::Lavender => "lavender",
        }
    }

    pub fn color(self) -> Option<[u8; 3]> {
        match self {
            Self::Original => None,
            Self::PastelYellow => Some([0xE8, 0xD8, 0x8A]),
            Self::BlueGray => Some([0x8F, 0xAF, 0xC4]),
            Self::DarkSlateBlue => Some([0x46, 0x5D, 0x70]),
            Self::Lavender => Some([0xB5, 0xA6, 0xCB]),
        }
    }

    pub fn cache_version(self, original_version: i64) -> i64 {
        // Keep existing Original cache entries valid while separating the artwork variants.
        match self {
            Self::Original => original_version,
            // The former darkened Soft variant used namespace 1_000_000.
            Self::PastelYellow => original_version + 2_000_000,
            Self::BlueGray => original_version + 3_000_000,
            Self::DarkSlateBlue => original_version + 7_000_000,
            Self::Lavender => original_version + 5_000_000,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_round_trip_and_default_to_original() {
        for appearance in FolderAppearance::ALL {
            assert_eq!(
                FolderAppearance::from_preference(appearance.preference()),
                appearance
            );
        }
        assert_eq!(
            FolderAppearance::from_preference("unknown"),
            FolderAppearance::Original
        );
        assert_ne!(
            FolderAppearance::Original.cache_version(2),
            FolderAppearance::PastelYellow.cache_version(2)
        );
        assert_eq!(
            FolderAppearance::from_preference("soft"),
            FolderAppearance::PastelYellow
        );
        let versions: std::collections::HashSet<_> = FolderAppearance::ALL
            .into_iter()
            .map(|a| a.cache_version(2))
            .collect();
        assert_eq!(versions.len(), FolderAppearance::ALL.len());
        assert!(!versions.contains(&1_000_002));
        assert!(!versions.contains(&4_000_002));
        assert!(!versions.contains(&6_000_002));
        assert_eq!(
            FolderAppearance::DarkSlateBlue.color(),
            Some([0x46, 0x5D, 0x70])
        );
        assert_eq!(
            FolderAppearance::from_preference("sage_green"),
            FolderAppearance::DarkSlateBlue
        );
    }
}
