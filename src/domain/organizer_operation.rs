use std::fmt;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct OrganizerOperationId(u64);

impl OrganizerOperationId {
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

impl fmt::Display for OrganizerOperationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrganizerOperationStatus {
    Started,
    Completed,
    Skipped,
    Cancelled,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrganizerOperationType {
    Move,
    Retry,
    Undo,
}

impl OrganizerOperationType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Move => "move",
            Self::Retry => "retry",
            Self::Undo => "undo",
        }
    }

    pub fn from_persisted(value: &str) -> Option<Self> {
        match value {
            "move" => Some(Self::Move),
            "retry" => Some(Self::Retry),
            "undo" => Some(Self::Undo),
            _ => None,
        }
    }
}

impl OrganizerOperationStatus {
    pub const fn is_terminal(self) -> bool {
        !matches!(self, Self::Started)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Completed => "completed",
            Self::Skipped => "skipped",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }

    pub fn from_persisted(value: &str) -> Option<Self> {
        match value {
            "started" => Some(Self::Started),
            "completed" => Some(Self::Completed),
            "skipped" => Some(Self::Skipped),
            "cancelled" => Some(Self::Cancelled),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}
