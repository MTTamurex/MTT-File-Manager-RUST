use std::fmt;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct OrganizerConflictId(u64);

impl OrganizerConflictId {
    pub const fn from_raw(value: u64) -> Option<Self> {
        if value == 0 {
            None
        } else {
            Some(Self(value))
        }
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for OrganizerConflictId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrganizerConflictStatus {
    Pending,
    Resolved,
    Obsolete,
    Cancelled,
}

impl OrganizerConflictStatus {
    pub const fn is_terminal(self) -> bool {
        !matches!(self, Self::Pending)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Resolved => "resolved",
            Self::Obsolete => "obsolete",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_persisted(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "resolved" => Some(Self::Resolved),
            "obsolete" => Some(Self::Obsolete),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrganizerConflictNameError {
    Empty,
    InvalidCharacter,
    DotName,
    TrailingSpaceOrDot,
    ReservedName,
    TooLong,
}

impl fmt::Display for OrganizerConflictNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Empty => "file name cannot be empty",
            Self::InvalidCharacter => "file name contains an invalid character",
            Self::DotName => "dot names are not valid file names",
            Self::TrailingSpaceOrDot => "file name cannot end with a space or dot",
            Self::ReservedName => "file name is reserved by Windows",
            Self::TooLong => "file name is too long",
        };
        formatter.write_str(message)
    }
}

fn is_reserved_windows_name(name: &str) -> bool {
    let base = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches([' ', '.']);
    let upper = base.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

pub fn validate_organizer_conflict_name(name: &str) -> Result<(), OrganizerConflictNameError> {
    if name.is_empty() {
        return Err(OrganizerConflictNameError::Empty);
    }
    if name == "." || name == ".." {
        return Err(OrganizerConflictNameError::DotName);
    }
    if name.chars().any(|character| {
        character <= '\u{1f}'
            || matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            )
    }) {
        return Err(OrganizerConflictNameError::InvalidCharacter);
    }
    if name.ends_with([' ', '.']) {
        return Err(OrganizerConflictNameError::TrailingSpaceOrDot);
    }
    if name.encode_utf16().count() > 255 {
        return Err(OrganizerConflictNameError::TooLong);
    }
    if is_reserved_windows_name(name) {
        return Err(OrganizerConflictNameError::ReservedName);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{validate_organizer_conflict_name, OrganizerConflictNameError};

    #[test]
    fn conflict_names_reject_windows_unsafe_components() {
        for (name, error) in [
            ("", OrganizerConflictNameError::Empty),
            ("..", OrganizerConflictNameError::DotName),
            ("report?.txt", OrganizerConflictNameError::InvalidCharacter),
            ("report\n.txt", OrganizerConflictNameError::InvalidCharacter),
            (
                "report.txt.",
                OrganizerConflictNameError::TrailingSpaceOrDot,
            ),
            ("CON.txt", OrganizerConflictNameError::ReservedName),
        ] {
            assert_eq!(validate_organizer_conflict_name(name), Err(error));
        }
    }

    #[test]
    fn conflict_names_limit_utf16_component_length() {
        let name = "a".repeat(256);
        assert_eq!(
            validate_organizer_conflict_name(&name),
            Err(OrganizerConflictNameError::TooLong)
        );
        assert!(validate_organizer_conflict_name("report (1).txt").is_ok());
    }
}
